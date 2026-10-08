package hotdrop.daemon;

import com.sun.tools.attach.VirtualMachine;
import com.sun.tools.attach.VirtualMachineDescriptor;
import hotdrop.protocol.Wire.AgentInfo;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStreamReader;
import java.io.PrintWriter;
import java.net.InetAddress;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Properties;
import java.util.Set;

public final class Main {
    private static final String USAGE = """
            HotDrop - save a Java file, see it live in the running server.

            usage: hotdrop <command> [options]

              start    watch sources, compile on save, hot swap into the server
              swap     compile and swap the given files once, then exit
              attach   load the agent into an already running server (no restart)
              status   show server connection, file states, timings
              flush [file]   compile now, skipping the debounce window
              pause | resume | rescan | stop
              doctor   check the setup

            source options (start, swap, doctor):
              --hybris <dir>        the hybris directory (contains bin/ and config/)
              --root <src>=<out>    a plain source tree and its class output directory (repeatable)
              --cp <path:path>      extra classpath used when no server is connected
            other options:
              --home <dir>          state directory (default ~/.hotdrop)
              --debounce <ms>       quiet window after the last save (default 40)
              --no-index            skip the start-up dependency index
              --watch <mode>        auto (default: native, polling on macOS) | native | poll
              --poll-ms <ms>        polling interval (default 100)
              --pid <pid>           (attach) target JVM; default: the JVM whose PLATFORM_HOME matches --hybris
              --debug
            """;

    private Main() {}

    public static void main(String[] args) throws Exception {
        if (args.length == 0 || args[0].equals("-h") || args[0].equals("--help")) {
            System.out.print(USAGE);
            return;
        }
        String cmd = args[0];
        Config cfg = new Config();
        List<String> positional = new ArrayList<>();
        Long pid = null;
        for (int i = 1; i < args.length; i++) {
            switch (args[i]) {
                case "--hybris" -> cfg.hybris = Path.of(args[++i]).toAbsolutePath().normalize();
                case "--root" -> {
                    String[] kv = args[++i].split("=", 2);
                    if (kv.length != 2) throw new IllegalArgumentException("--root expects <src>=<out>");
                    Path src = Path.of(kv[0]);
                    cfg.manualRoots.add(new Root(src.getFileName().toString(), src, Path.of(kv[1]), Path.of(kv[1]), false));
                }
                case "--cp" -> {
                    for (String e : args[++i].split(java.io.File.pathSeparator)) {
                        if (!e.isEmpty()) cfg.extraClasspath.add(Path.of(e).toAbsolutePath().normalize());
                    }
                }
                case "--home" -> cfg.home = Path.of(args[++i]).toAbsolutePath().normalize();
                case "--debounce" -> cfg.debounceMs = Integer.parseInt(args[++i]);
                case "--no-index" -> cfg.index = false;
                case "--watch" -> cfg.watch = args[++i];
                case "--poll-ms" -> cfg.pollMs = Integer.parseInt(args[++i]);
                case "--pid" -> pid = Long.parseLong(args[++i]);
                case "--debug" -> Log.debug = true;
                default -> positional.add(args[i]);
            }
        }
        switch (cmd) {
            case "start" -> start(cfg);
            case "swap" -> System.exit(swap(cfg, positional));
            case "attach" -> attach(cfg, pid);
            case "doctor" -> doctor(cfg);
            case "status", "pause", "resume", "rescan", "stop" -> control(cfg, cmd.toUpperCase());
            case "flush" -> control(cfg, positional.isEmpty() ? "FLUSH" : "FLUSH " + Path.of(positional.get(0)).toAbsolutePath());
            default -> {
                System.err.println("unknown command: " + cmd);
                System.out.print(USAGE);
                System.exit(2);
            }
        }
    }

    // ---- setup ----

    private static List<Root> roots(Config cfg) throws IOException {
        List<Root> roots = new ArrayList<>(cfg.manualRoots);
        if (cfg.hybris != null) roots.addAll(Discovery.discover(cfg.hybris));
        return roots;
    }

    private static List<String> javacOptions(Config cfg) {
        List<String> o = new ArrayList<>(List.of("-g", "-nowarn", "-proc:none", "-implicit:none", "-Xlint:none",
                "-Xmaxerrs", "10000"));
        String enc = "UTF-8";
        if (cfg.hybris != null) {
            Discovery.BuildSettings s = Discovery.buildSettings(cfg.hybris);
            enc = s.encoding();
            if (s.level() != null && !s.level().isEmpty()) {
                o.addAll(List.of("-source", s.level(), "-target", s.level()));
                o.addAll(s.exports());
            }
        }
        o.addAll(List.of("-encoding", enc));
        return o;
    }

    private static Engine engine(Config cfg) throws IOException {
        List<Root> roots = roots(cfg);
        if (roots.isEmpty()) {
            System.err.println("nothing to watch: pass --hybris <dir> (with custom extensions) or --root <src>=<out>");
            System.exit(2);
        }
        AgentLink link = new AgentLink(cfg.agentsDir(), cfg.platformHome());
        return new Engine(cfg, roots, link, javacOptions(cfg));
    }

    // ---- commands ----

    private static void start(Config cfg) throws Exception {
        Engine engine = engine(cfg);
        Daemon daemon = new Daemon(cfg, engine);
        daemon.start();
        Control control = new Control(daemon, cfg.home);
        Runtime.getRuntime().addShutdownHook(new Thread(control::cleanup));
        Log.info("HotDrop running on Java %s; watching %d source root(s):", System.getProperty("java.version"), engine.roots.size());
        for (Root r : engine.roots) Log.info("  %s  %s  ->  %s", r.name, r.src, r.out);
        Log.info("save a .java file to compile and swap it. Ctrl+C to quit.");
        daemon.join();
        control.cleanup();
    }

    private static int swap(Config cfg, List<String> files) throws IOException {
        if (files.isEmpty()) {
            System.err.println("swap: give at least one .java file");
            return 2;
        }
        Engine engine = engine(cfg);
        engine.maintenance();
        Set<Path> changed = new LinkedHashSet<>();
        for (String f : files) changed.add(Path.of(f).toAbsolutePath().normalize());
        Engine.Report rep = engine.runCycle(changed, Set.of());
        Daemon.logReport(rep);
        engine.close();
        return rep.broken.isEmpty() && rep.rejected.isEmpty() ? 0 : 1;
    }

    private static void control(Config cfg, String command) throws IOException {
        Path f = cfg.home.resolve("daemon.properties");
        if (!Files.isRegularFile(f)) {
            System.err.println("no HotDrop daemon running (" + f + " not found)");
            System.exit(1);
        }
        Properties p = new Properties();
        try (var r = Files.newBufferedReader(f)) {
            p.load(r);
        }
        try (Socket s = new Socket(InetAddress.getLoopbackAddress(), Integer.parseInt(p.getProperty("port")))) {
            s.setSoTimeout(30_000);
            PrintWriter out = new PrintWriter(s.getOutputStream(), true, StandardCharsets.UTF_8);
            out.println(p.getProperty("token") + " " + command);
            BufferedReader in = new BufferedReader(new InputStreamReader(s.getInputStream(), StandardCharsets.UTF_8));
            String line;
            while ((line = in.readLine()) != null) System.out.println(line);
        } catch (IOException e) {
            System.err.println("daemon not reachable: " + e.getMessage());
            System.exit(1);
        }
    }

    private static void attach(Config cfg, Long pid) throws Exception {
        Path jar = Path.of(Main.class.getProtectionDomain().getCodeSource().getLocation().toURI())
                .resolveSibling("hotdrop-agent.jar");
        if (System.getProperty("hotdrop.agent.jar") != null) jar = Path.of(System.getProperty("hotdrop.agent.jar"));
        if (!Files.isRegularFile(jar)) {
            System.err.println("agent jar not found: " + jar);
            System.exit(2);
        }
        String options = cfg.home.equals(Config.class.getDeclaredConstructor().newInstance().home) ? "" : "dir=" + cfg.agentsDir();
        Path wanted = cfg.platformHome();
        for (VirtualMachineDescriptor d : VirtualMachine.list()) {
            if (pid != null && !d.id().equals(String.valueOf(pid))) continue;
            if (d.id().equals(String.valueOf(ProcessHandle.current().pid()))) continue;
            VirtualMachine vm;
            try {
                vm = VirtualMachine.attach(d);
            } catch (Exception e) {
                continue;
            }
            try {
                if (pid == null) {
                    String ph = vm.getSystemProperties().getProperty("PLATFORM_HOME");
                    if (ph == null || wanted == null || !Files.isSameFile(Path.of(ph), wanted)) continue;
                }
                vm.loadAgent(jar.toString(), options);
                System.out.println("agent loaded into pid " + d.id() + " (" + d.displayName() + ")");
                return;
            } finally {
                vm.detach();
            }
        }
        System.err.println("no matching JVM found" + (wanted == null ? "; pass --pid or --hybris" : " for PLATFORM_HOME=" + wanted));
        System.exit(1);
    }

    private static void doctor(Config cfg) throws Exception {
        System.out.println("daemon JDK:       " + System.getProperty("java.version") + " (" + System.getProperty("java.vendor") + ") at " + System.getProperty("java.home"));
        System.out.println("system compiler:  " + (javax.tools.ToolProvider.getSystemJavaCompiler() != null ? "available" : "MISSING - run on a JDK"));
        Path inotify = Path.of("/proc/sys/fs/inotify/max_user_watches");
        if (Files.isRegularFile(inotify)) System.out.println("inotify watches: " + Files.readAllLines(inotify).get(0).trim() + " (raise fs.inotify.max_user_watches if you see 'cannot watch')");
        if (cfg.hybris != null) {
            Discovery.BuildSettings s = Discovery.buildSettings(cfg.hybris);
            System.out.println("hybris:           " + cfg.hybris);
            System.out.println("build.target:     " + s.level() + ", encoding " + s.encoding());
            List<Root> roots = Discovery.discover(cfg.hybris);
            System.out.println("watched roots:    " + roots.size());
            for (Root r : roots) System.out.println("    " + r.name + "  " + r.src);
            Path wrapper = cfg.platformHome().resolve("tomcat/conf/wrapper.conf");
            if (Files.isRegularFile(wrapper)) {
                boolean has = Files.readString(wrapper).contains("hotdrop-agent");
                System.out.println("wrapper.conf:     " + (has ? "contains the HotDrop agent" : "does NOT contain the agent - add -javaagent to tomcat.javaoptions and run 'ant server'"));
            } else {
                System.out.println("wrapper.conf:     not generated yet (run 'ant server' after adding the agent to tomcat.javaoptions)");
            }
        }
        AgentLink link = new AgentLink(cfg.agentsDir(), cfg.platformHome());
        if (link.poll()) {
            AgentInfo i = link.info();
            System.out.println("server:           pid " + i.pid() + ", " + i.vendor() + " " + i.javaVersion() + ", redefine " + (i.redefineSupported() ? "supported" : "NOT supported")
                    + ", enhanced " + (i.enhancedRedefine() ? "ON" : "off"));
            if (!i.javaVersion().split("[.+-]")[0].equals(System.getProperty("java.version").split("[.+-]")[0])) {
                System.out.println("WARNING:          daemon runs Java " + System.getProperty("java.version") + " but the server runs Java " + i.javaVersion()
                        + ". Start HotDrop with the same JDK that builds Hybris, or synthetic names (lambdas) can differ and swaps get rejected.");
            }
        } else {
            System.out.println("server:           no agent connected (start Hybris with the agent, or run 'hotdrop attach')");
        }
        link.close();
    }
}
