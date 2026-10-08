package hotdrop.daemon;

import com.sun.tools.attach.VirtualMachine;
import com.sun.tools.attach.VirtualMachineDescriptor;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Properties;

/**
 * "hotdrop up": finds the running Hybris server by itself, runs on the server's own JDK, loads the agent when the
 * server has none (also after a server restart) and starts watching. One command, no JAVA_HOME, no separate attach.
 */
final class Up {
    private static final String REEXEC = "hotdrop.reexec";

    /** A Hybris JVM found among the running Java processes. */
    record Server(long pid, Path hybris, Path javaHome) {}

    private Up() {}

    static void run(Config cfg, List<String> rawArgs) throws Exception {
        if (watcherRunning(cfg)) {
            if (System.getProperty("hotdrop.child") != null) return;
            Log.info("a HotDrop watcher is already running (started with the server); showing its log, Ctrl+C to leave it running");
            followLog(cfg.home.resolve("daemon.log"));
            return;
        }
        Server server = null;
        if (cfg.hybris == null) {
            server = awaitServer(cfg);
            cfg.hybris = server.hybris();
        } else {
            server = findServer(cfg.hybris);
        }
        remember(cfg);

        if (server != null && System.getProperty(REEXEC) == null && !sameJdk(server.javaHome())) {
            System.exit(reexec(server, cfg, rawArgs));
        }
        Log.info("Hybris: %s", cfg.hybris);
        Thread keeper = new Thread(() -> keepAttached(cfg), "hotdrop-attach-keeper");
        keeper.setDaemon(true);
        keeper.start();
        Main.startDaemon(cfg);
    }

    // ---- one watcher at a time ----

    /** True when a watcher answers on its control port, or the one the server's agent started is still alive. */
    private static boolean watcherRunning(Config cfg) {
        try {
            Path pidFile = cfg.home.resolve("autostart.pid");
            if (Files.isRegularFile(pidFile)) {
                long pid = Long.parseLong(Files.readString(pidFile).trim());
                if (pid != ProcessHandle.current().pid() && ProcessHandle.of(pid).map(ProcessHandle::isAlive).orElse(false)) return true;
            }
            Path f = cfg.home.resolve("daemon.properties");
            if (!Files.isRegularFile(f)) return false;
            Properties p = new Properties();
            try (var r = Files.newBufferedReader(f)) {
                p.load(r);
            }
            try (java.net.Socket s = new java.net.Socket()) {
                s.connect(new java.net.InetSocketAddress(java.net.InetAddress.getLoopbackAddress(),
                        Integer.parseInt(p.getProperty("port"))), 300);
                return true;
            }
        } catch (IOException | RuntimeException e) {
            return false;
        }
    }

    /** Prints the last lines of the log, then follows it like tail -f. */
    private static void followLog(Path log) throws IOException, InterruptedException {
        long pos = 0;
        if (Files.isRegularFile(log)) {
            List<String> lines = Files.readAllLines(log);
            for (String l : lines.subList(Math.max(0, lines.size() - 20), lines.size())) System.out.println(l);
            pos = Files.size(log);
        }
        while (true) {
            Thread.sleep(300);
            if (!Files.isRegularFile(log)) continue;
            long size = Files.size(log);
            if (size < pos) pos = 0;
            if (size == pos) continue;
            try (java.io.RandomAccessFile f = new java.io.RandomAccessFile(log.toFile(), "r")) {
                f.seek(pos);
                byte[] buf = new byte[(int) (size - pos)];
                f.readFully(buf);
                System.out.print(new String(buf, java.nio.charset.StandardCharsets.UTF_8));
                System.out.flush();
            }
            pos = size;
        }
    }

    // ---- finding the server ----

    /** Returns the running Hybris server, waiting for one to start when there is none. */
    private static Server awaitServer(Config cfg) throws InterruptedException {
        Server s = findServer(null);
        if (s != null) return s;
        Path guess = remembered(cfg);
        if (guess == null) guess = enclosingHybris();
        if (guess != null) {
            Log.info("no Hybris server running; waiting for the one in %s to start ...", guess);
        } else {
            Log.info("no Hybris server running; waiting for one to start (or pass --hybris <dir>) ...");
        }
        while (true) {
            Thread.sleep(2000);
            s = findServer(guess);
            if (s != null) return s;
        }
    }

    /** Looks through running JVMs for Hybris. With {@code wanted} set, only a server of that installation matches. */
    static Server findServer(Path wanted) {
        for (VirtualMachineDescriptor d : VirtualMachine.list()) {
            if (d.id().equals(String.valueOf(ProcessHandle.current().pid()))) continue;
            VirtualMachine vm;
            try {
                vm = VirtualMachine.attach(d);
            } catch (Exception e) {
                continue;
            }
            try {
                Properties p = vm.getSystemProperties();
                Path bin = null;
                String binDir = p.getProperty("HYBRIS_BIN_DIR");
                if (binDir != null) bin = Path.of(binDir);
                else if (p.getProperty("PLATFORM_HOME") != null) bin = Path.of(p.getProperty("PLATFORM_HOME")).getParent();
                else if (p.getProperty("catalina.home") != null && Path.of(p.getProperty("catalina.home")).getParent() != null) {
                    bin = Path.of(p.getProperty("catalina.home")).getParent().getParent();
                }
                if (bin == null || bin.getParent() == null || !Files.isDirectory(bin.resolve("platform"))) continue;
                Path hybris = bin.getParent();
                if (wanted != null && !Files.isSameFile(hybris, wanted)) continue;
                return new Server(Long.parseLong(d.id()), hybris, Path.of(p.getProperty("java.home")));
            } catch (Exception e) {
                // not readable or not Hybris: next
            } finally {
                try {
                    vm.detach();
                } catch (IOException ignored) {
                    // nothing to do
                }
            }
        }
        return null;
    }

    private static Path enclosingHybris() {
        for (Path p = Path.of("").toAbsolutePath(); p != null; p = p.getParent()) {
            if (Files.isDirectory(p.resolve("bin/custom")) && Files.isRegularFile(p.resolve("config/localextensions.xml"))) return p;
            Path nested = p.resolve("hybris");
            if (Files.isDirectory(nested.resolve("bin/custom")) && Files.isRegularFile(nested.resolve("config/localextensions.xml"))) return nested;
        }
        return null;
    }

    private static Path remembered(Config cfg) {
        try {
            Path f = cfg.home.resolve("last-hybris");
            if (Files.isRegularFile(f)) {
                Path p = Path.of(Files.readString(f).trim());
                if (Files.isDirectory(p.resolve("bin/custom"))) return p;
            }
        } catch (IOException | RuntimeException ignored) {
            // no usable memory
        }
        return null;
    }

    private static void remember(Config cfg) {
        try {
            Files.createDirectories(cfg.home);
            Files.writeString(cfg.home.resolve("last-hybris"), cfg.hybris.toString());
        } catch (IOException ignored) {
            // remembering is a convenience
        }
    }

    // ---- running on the server's JDK ----

    private static boolean sameJdk(Path serverJavaHome) {
        try {
            return Files.isSameFile(serverJavaHome, Path.of(System.getProperty("java.home")));
        } catch (IOException e) {
            return true;
        }
    }

    private static int reexec(Server server, Config cfg, List<String> rawArgs) throws Exception {
        Path java = server.javaHome().resolve("bin").resolve(System.getProperty("os.name").startsWith("Windows") ? "java.exe" : "java");
        if (!Files.isExecutable(java)) {
            Log.info("server runs on %s but %s is not usable; continuing on this JDK", server.javaHome(), java);
            return -1;
        }
        Path jar = Path.of(Main.class.getProtectionDomain().getCodeSource().getLocation().toURI());
        Log.info("switching to the server's JDK: %s", server.javaHome());
        List<String> cmd = new ArrayList<>(List.of(java.toString(), "-D" + REEXEC + "=1", "-jar", jar.toString(), "up"));
        if (!rawArgs.contains("--hybris")) cmd.addAll(List.of("--hybris", cfg.hybris.toString()));
        cmd.addAll(rawArgs);
        return new ProcessBuilder(cmd).inheritIO().start().waitFor();
    }

    // ---- keeping the agent loaded ----

    /** True when an agent has registered for a process that is still running (so no scan of all JVMs is needed). */
    private static boolean hasLiveAgent(Config cfg) {
        if (!Files.isDirectory(cfg.agentsDir())) return false;
        try (var files = Files.list(cfg.agentsDir())) {
            return files.anyMatch(f -> {
                String n = f.getFileName().toString();
                if (!n.endsWith(".properties")) return false;
                try {
                    return ProcessHandle.of(Long.parseLong(n.substring(0, n.length() - ".properties".length()))).isPresent();
                } catch (NumberFormatException e) {
                    return false;
                }
            });
        } catch (IOException e) {
            return false;
        }
    }

    private static void keepAttached(Config cfg) {
        long lastTried = -1;
        while (true) {
            try {
                Server s = hasLiveAgent(cfg) ? null : findServer(cfg.hybris);
                if (s != null && s.pid() != lastTried) {
                    lastTried = s.pid();
                    Main.loadAgent(cfg, String.valueOf(s.pid()));
                    Log.info("agent loaded into the server (pid %d)", s.pid());
                }
                Thread.sleep(3000);
            } catch (InterruptedException e) {
                return;
            } catch (Exception e) {
                Log.info("could not load the agent: %s", e.getMessage());
                try {
                    Thread.sleep(5000);
                } catch (InterruptedException ie) {
                    return;
                }
            }
        }
    }
}
