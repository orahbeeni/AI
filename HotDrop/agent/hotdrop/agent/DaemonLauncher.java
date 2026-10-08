package hotdrop.agent;

import java.io.IOException;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.Socket;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Deque;
import java.util.List;
import java.util.Map;
import java.util.Properties;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;

/**
 * Starts the HotDrop daemon (watcher + compiler) as a child process of the server, so that the single -javaagent
 * line in local.properties is all that is needed. The child runs on the server's own JDK, logs to
 * ~/.hotdrop/daemon.log and is stopped together with the server. Never throws: this is a convenience.
 * Opt out with -Dhotdrop.autostart=false or the agent option autostart=false.
 */
final class DaemonLauncher {
    private DaemonLauncher() {}

    static void launchInBackground(Map<String, String> opts, Path agentsDir) {
        if ("false".equalsIgnoreCase(System.getProperty("hotdrop.autostart", opts.get("autostart")))) return;
        Thread t = new Thread(() -> {
            try {
                launch(agentsDir);
            } catch (Throwable e) {
                System.err.println("[HotDrop] could not start the watcher: " + e);
            }
        }, "hotdrop-autostart");
        t.setDaemon(true);
        t.start();
    }

    private static void launch(Path agentsDir) throws IOException, java.net.URISyntaxException {
        Path home = agentsDir.getParent();
        if (home == null || daemonRunning(home)) return;
        Path agentJar = Path.of(DaemonLauncher.class.getProtectionDomain().getCodeSource().getLocation().toURI());
        Path daemonJar = agentJar.resolveSibling("hotdrop-daemon.jar");
        if (!Files.isRegularFile(daemonJar)) {
            System.err.println("[HotDrop] watcher not started: " + daemonJar + " not found (keep it next to the agent jar)");
            return;
        }
        Path hybris = hybrisDir();
        if (hybris == null) {
            System.err.println("[HotDrop] watcher not started: could not tell where the Hybris directory is");
            return;
        }
        boolean windows = System.getProperty("os.name", "").startsWith("Windows");
        Path java = Path.of(System.getProperty("java.home"), "bin", windows ? "java.exe" : "java");
        Path log = home.resolve("daemon.log");
        // The watcher exits on out-of-memory instead of limping on without its worker threads; the loop below restarts it.
        List<String> cmd = new ArrayList<>(List.of(java.toString(), "-Xmx2g", "-XX:+ExitOnOutOfMemoryError",
                "-Dhotdrop.child=1", "-jar", daemonJar.toString(), "up", "--hybris", hybris.toString()));
        if (!home.equals(Path.of(System.getProperty("user.home"), ".hotdrop"))) cmd.addAll(List.of("--home", home.toString()));

        rotate(log);
        AtomicBoolean stopping = new AtomicBoolean();
        AtomicReference<Process> current = new AtomicReference<>();
        Runtime.getRuntime().addShutdownHook(new Thread(() -> {
            stopping.set(true);
            Process p = current.get();
            if (p != null) p.destroy();
        }, "hotdrop-stop-watcher"));

        Deque<Long> starts = new ArrayDeque<>();
        while (!stopping.get()) {
            ProcessBuilder pb = new ProcessBuilder(cmd);
            pb.redirectErrorStream(true);
            pb.redirectOutput(ProcessBuilder.Redirect.appendTo(log.toFile()));
            Process p = pb.start();
            p.getOutputStream().close();
            current.set(p);
            Files.writeString(home.resolve("autostart.pid"), String.valueOf(p.pid()));
            System.out.println("[HotDrop] watcher started (pid " + p.pid() + "), log: " + log);
            int exit;
            try {
                exit = p.waitFor();
            } catch (InterruptedException e) {
                return;
            }
            if (stopping.get() || exit == 0 || exit == 143) return; // asked to stop, or another watcher already runs
            long now = System.currentTimeMillis();
            starts.addLast(now);
            while (!starts.isEmpty() && now - starts.peekFirst() > 10 * 60_000L) starts.removeFirst();
            if (starts.size() > 5) {
                System.err.println("[HotDrop] watcher keeps failing (exit " + exit + "); giving up. See " + log
                        + " - restart the server or run 'hotdrop up' once the cause is fixed");
                return;
            }
            System.err.println("[HotDrop] watcher stopped unexpectedly (exit " + exit + "); restarting in 5 s. See " + log);
            try {
                Thread.sleep(5000);
            } catch (InterruptedException e) {
                return;
            }
        }
    }

    /** Keeps the log from growing forever: one previous file is kept as daemon.log.1. */
    private static void rotate(Path log) {
        try {
            if (Files.isRegularFile(log) && Files.size(log) > 10L * 1024 * 1024) {
                Files.move(log, log.resolveSibling(log.getFileName() + ".1"), java.nio.file.StandardCopyOption.REPLACE_EXISTING);
            }
        } catch (IOException e) {
            // keep appending to the old file
        }
    }

    private static Path hybrisDir() {
        String bin = System.getProperty("HYBRIS_BIN_DIR");
        if (bin != null && !bin.isEmpty()) return parent(Path.of(bin));
        String platform = System.getProperty("PLATFORM_HOME");
        if (platform != null && !platform.isEmpty()) return parent(parent(Path.of(platform)));
        String catalina = System.getProperty("catalina.home");
        if (catalina != null && !catalina.isEmpty()) return parent(parent(parent(Path.of(catalina))));
        return null;
    }

    private static Path parent(Path p) {
        return p == null ? null : p.getParent();
    }

    /** True when a watcher already answers on its control port, or one we started is still alive. */
    private static boolean daemonRunning(Path home) {
        try {
            Path pidFile = home.resolve("autostart.pid");
            if (Files.isRegularFile(pidFile)) {
                long pid = Long.parseLong(Files.readString(pidFile).trim());
                // process ids get reused: also check the command line where the OS reports it
                boolean watcher = ProcessHandle.of(pid).filter(ProcessHandle::isAlive)
                        .map(h -> h.info().commandLine().map(c -> c.contains("hotdrop-daemon")).orElse(true))
                        .orElse(false);
                if (watcher) return true;
            }
            Path f = home.resolve("daemon.properties");
            if (!Files.isRegularFile(f)) return false;
            Properties p = new Properties();
            try (var r = Files.newBufferedReader(f)) {
                p.load(r);
            }
            try (Socket s = new Socket()) {
                s.connect(new InetSocketAddress(InetAddress.getLoopbackAddress(), Integer.parseInt(p.getProperty("port"))), 300);
                return true;
            }
        } catch (IOException | RuntimeException e) {
            return false;
        }
    }
}
