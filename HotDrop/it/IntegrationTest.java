import javax.tools.JavaCompiler;
import javax.tools.ToolProvider;
import java.io.BufferedReader;
import java.io.File;
import java.io.IOException;
import java.io.InputStreamReader;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

/**
 * End-to-end: a fake "server" JVM (agent attached, classes loaded by a URLClassLoader from a classes directory,
 * the way Hybris does) plus the real daemon watching a source tree. Each step edits a file and checks what the
 * running JVM does. Run: java it/IntegrationTest.java   (after java Build.java)
 * Set HOTDROP_WATCH=poll to exercise the polling watcher used on macOS.
 */
public class IntegrationTest {
    static final List<String> appOut = Collections.synchronizedList(new ArrayList<>());
    static final List<String> daemonOut = Collections.synchronizedList(new ArrayList<>());
    static int failures;
    static Path src;
    static final List<String> latencies = new ArrayList<>();

    static final String APP = """
            import java.lang.reflect.Method;
            import java.net.URL;
            import java.net.URLClassLoader;
            import java.nio.file.Path;
            public class App {
                public static void main(String[] a) throws Exception {
                    URLClassLoader cl = new URLClassLoader(new URL[]{Path.of(a[0]).toUri().toURL()}, App.class.getClassLoader());
                    Object svc = cl.loadClass("demo.Service").getDeclaredConstructor().newInstance();
                    Method m = svc.getClass().getMethod("greet");
                    System.out.println("READY");
                    String last = null;
                    while (true) {
                        String now = String.valueOf(m.invoke(svc));
                        if (!now.equals(last)) { System.out.println("OUT " + now); last = now; }
                        Thread.sleep(5);
                    }
                }
            }
            """;

    static String service(String ver, boolean extra, boolean lambda) {
        return "package demo;\npublic class Service {\n  public String greet() {\n"
                + (lambda ? "    java.util.function.Supplier<String> s = () -> \"lam\";\n" : "")
                + "    return \"hello " + ver + " \" + Helper.PREFIX + \" \" + Helper.tag()"
                + (extra ? " + \" \" + Extra.value()" : "") + (lambda ? " + s.get()" : "") + ";\n  }\n}\n";
    }

    static String helper(String prefix) {
        return "package demo;\npublic class Helper {\n  public static final String PREFIX = \"" + prefix
                + "\";\n  public static String tag() { return \"tag\"; }\n}\n";
    }

    static String extra(String body) {
        return "package demo;\npublic class Extra {\n  public static String value() { return " + body + "; }\n}\n";
    }

    public static void main(String[] args) throws Exception {
        Path repo = Path.of(System.getProperty("user.dir"));
        Path agentJar = repo.resolve("build/hotdrop-agent.jar");
        Path daemonJar = repo.resolve("build/hotdrop-daemon.jar");
        Path work = Files.createTempDirectory("hotdrop-it");
        src = work.resolve("src");
        Path classes = work.resolve("classes");
        Path appDir = work.resolve("app");
        Path home = work.resolve("home");
        Files.createDirectories(src.resolve("demo"));
        Files.createDirectories(classes);
        Files.createDirectories(appDir);
        Files.writeString(src.resolve("demo/Service.java"), service("v1", false, false));
        Files.writeString(src.resolve("demo/Helper.java"), helper("p1"));
        Files.writeString(appDir.resolve("App.java"), APP);

        JavaCompiler jc = ToolProvider.getSystemJavaCompiler();
        check("initial compile of demo sources", jc.run(null, null, null, "-g", "-d", classes.toString(),
                src.resolve("demo/Service.java").toString(), src.resolve("demo/Helper.java").toString()) == 0);
        check("compile app", jc.run(null, null, null, "-d", appDir.toString(), appDir.resolve("App.java").toString()) == 0);

        boolean windows = System.getProperty("os.name").toLowerCase().contains("win");
        String java = Path.of(System.getProperty("java.home"), "bin", windows ? "java.exe" : "java").toString();
        String watch = System.getenv().getOrDefault("HOTDROP_WATCH", "auto");
        Process app = start(work, appOut, "app", java, "-javaagent:" + agentJar + "=dir=" + home.resolve("agents") + ",quiet=true",
                "-cp", appDir.toString(), "App", classes.toString());
        Process daemon = null;
        try {
            check("app started and loaded demo.Service", waitFor(appOut, "READY", 10_000) >= 0);
            check("app prints first value", waitFor(appOut, "OUT hello v1 p1 tag", 5_000) >= 0);

            daemon = start(work, daemonOut, "daemon", java, "-jar", daemonJar.toString(), "start",
                    "--root", src + "=" + classes, "--home", home.toString(), "--watch", watch, "--debug");
            check("daemon connected to the agent", waitFor(daemonOut, "agent connected", 10_000) >= 0);
            check("daemon indexed the sources", waitFor(daemonOut, "indexed", 20_000) >= 0);
            Thread.sleep(300);

            // 1. plain method-body change
            step("1. method body change is swapped live");
            long ms = edit("demo/Service.java", service("v2", false, false), appOut, "OUT hello v2 p1 tag", 5_000);
            check("   live in the running JVM", ms >= 0);
            latency("body change", ms);

            // 2. refers to a class that does not exist yet -> pending
            step("2. file using a not-yet-written class stays pending");
            int before = appOut.size();
            Files.writeString(src.resolve("demo/Service.java"), service("v3", true, false));
            check("   daemon reports Service as broken", waitFor(daemonOut, "[broken] Service.java", 5_000) >= 0);
            Thread.sleep(500);
            check("   running JVM untouched", appOut.size() == before);

            // 3. the class appears but is itself broken -> Service must be held, not swapped
            step("3. dependent of a broken class is held, not swapped");
            Files.writeString(src.resolve("demo/Extra.java"), extra("undefinedVariable"));
            check("   Extra reported broken", waitFor(daemonOut, "[broken] Extra.java", 5_000) >= 0);
            check("   Service reported held", waitFor(daemonOut, "[held] Service.java", 5_000) >= 0);
            Thread.sleep(500);
            check("   running JVM still untouched", appOut.size() == before);

            // 4. fix the other file -> both go live together
            step("4. fixing the other file releases both, in one batch");
            ms = edit("demo/Extra.java", extra("\"e1\""), appOut, "OUT hello v3 p1 tag e1", 5_000);
            check("   both classes live (new class loaded from disk)", ms >= 0);
            latency("pending fix + new class", ms);
            check("   swapped in one cycle", daemonOut.stream().anyMatch(l -> l.contains("swapped") && l.contains("Service") && l.contains("Extra")));

            // 5. changed constant: Service has the old value inlined -> must be recompiled
            step("5. changed constant ripples to the class that inlined it");
            ms = edit("demo/Helper.java", helper("p2"), appOut, "OUT hello v3 p2 tag e1", 5_000);
            check("   dependent recompiled without being edited", ms >= 0);
            latency("constant ripple", ms);

            // 6. lambda added: standard JVM cannot swap it -> clear message, no damage
            step("6. a change the JVM cannot swap is reported, not hidden");
            before = appOut.size();
            Files.writeString(src.resolve("demo/Service.java"), service("v4", true, true));
            check("   daemon says restart required", waitFor(daemonOut, "[restart required]", 5_000) >= 0);
            Thread.sleep(300);
            check("   running JVM unharmed", appOut.size() == before);
            ms = edit("demo/Service.java", service("v5", true, false), appOut, "OUT hello v5 p2 tag e1", 5_000);
            check("   removing the lambda recovers", ms >= 0);

            // 7. CLI
            step("7. CLI talks to the daemon");
            Process st = new ProcessBuilder(java, "-jar", daemonJar.toString(), "status", "--home", home.toString())
                    .redirectErrorStream(true).start();
            String status = new String(st.getInputStream().readAllBytes());
            st.waitFor();
            check("   status shows server and files", status.contains("server:") && status.contains("files:"));
            System.out.println(status.lines().map(l -> "      " + l).reduce("", (x, y) -> x + y + "\n"));
        } finally {
            if (daemon != null) daemon.destroy();
            app.destroy();
        }

        System.out.println("\nlatency (file write -> change visible in the running JVM):");
        latencies.forEach(l -> System.out.println("  " + l));
        System.out.println(failures == 0 ? "\nALL CHECKS PASSED" : "\n" + failures + " CHECK(S) FAILED");
        if (System.getenv("SHOW_LOG") != null) { System.out.println("\n--- daemon log ---"); synchronized (daemonOut) { daemonOut.forEach(System.out::println); } }
        if (failures > 0) {
            System.out.println("\n--- daemon output ---");
            synchronized (daemonOut) { daemonOut.forEach(System.out::println); }
        }
        System.exit(failures == 0 ? 0 : 1);
    }

    static void step(String name) { System.out.println(name); }

    static void latency(String what, long ms) { latencies.add(String.format("%-28s %d ms", what, ms)); }

    static void check(String what, boolean ok) {
        System.out.println((ok ? "  PASS " : "  FAIL ") + what.strip());
        if (!ok) failures++;
    }

    /** Writes a file and returns the milliseconds until {@code expect} shows up in {@code out}, or -1. */
    static long edit(String rel, String content, List<String> out, String expect, long timeoutMs) throws Exception {
        long t0 = System.nanoTime();
        Files.writeString(src.resolve(rel), content);
        int idx = waitFor(out, expect, timeoutMs);
        return idx < 0 ? -1 : (System.nanoTime() - t0) / 1_000_000;
    }

    static int waitFor(List<String> out, String needle, long timeoutMs) throws InterruptedException {
        long end = System.currentTimeMillis() + timeoutMs;
        do {
            synchronized (out) {
                for (int i = 0; i < out.size(); i++) if (out.get(i).contains(needle)) return i;
            }
            Thread.sleep(2);
        } while (System.currentTimeMillis() < end);
        return -1;
    }

    static Process start(Path dir, List<String> sink, String label, String... cmd) throws IOException {
        ProcessBuilder pb = new ProcessBuilder(cmd).directory(dir.toFile()).redirectErrorStream(true);
        Process p = pb.start();
        Thread t = new Thread(() -> {
            try (BufferedReader r = new BufferedReader(new InputStreamReader(p.getInputStream()))) {
                String line;
                while ((line = r.readLine()) != null) sink.add(line);
            } catch (IOException ignored) { }
        }, label + "-out");
        t.setDaemon(true);
        t.start();
        return p;
    }
}
