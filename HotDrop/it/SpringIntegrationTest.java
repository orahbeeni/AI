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
import java.util.stream.Stream;

/**
 * End-to-end for the Spring layer: a real Spring application context in a JVM with the agent, the real daemon watching
 * a *-spring.xml file and a source tree. Needs Spring 5.3 or 6 jars (spring-core, -beans, -context, -aop, -expression,
 * -web, -webmvc, a logging jar and a servlet API): set HOTDROP_SPRING_LIB to a directory or a path-separated list, or
 * keep a Hybris platform at one of the usual places. Without jars the test is skipped.
 * Run: java it/SpringIntegrationTest.java   (after java Build.java)
 */
public class SpringIntegrationTest {
    static final List<String> appOut = Collections.synchronizedList(new ArrayList<>());
    static final List<String> daemonOut = Collections.synchronizedList(new ArrayList<>());
    static int failures;
    static Path src;
    static Path xml;
    static Path kidXml;
    static Path messages;
    static Path itemsXml;
    static final List<String> latencies = new ArrayList<>();

    static final String APP = """
            import java.lang.reflect.Method;
            import java.net.URL;
            import java.net.URLClassLoader;
            import java.nio.file.Path;
            import org.springframework.beans.factory.xml.XmlBeanDefinitionReader;
            import org.springframework.context.support.GenericApplicationContext;
            import org.springframework.core.io.FileSystemResource;
            import org.springframework.web.servlet.mvc.method.annotation.RequestMappingHandlerMapping;
            public class SApp {
                public static void main(String[] a) throws Exception {
                    URLClassLoader cl = new URLClassLoader(new URL[]{Path.of(a[0]).toUri().toURL()}, SApp.class.getClassLoader());
                    GenericApplicationContext ctx = new GenericApplicationContext();
                    ctx.setClassLoader(cl);
                    ctx.registerBean("messageSource", org.springframework.context.support.ReloadableResourceBundleMessageSource.class, () -> {
                        var ms = new org.springframework.context.support.ReloadableResourceBundleMessageSource();
                        ms.setBasename("file:" + a[3] + "/msg");
                        ms.setCacheSeconds(-1);
                        ms.setDefaultEncoding("UTF-8");
                        return ms;
                    });
                    new XmlBeanDefinitionReader(ctx).loadBeanDefinitions(new FileSystemResource(a[1]));
                    ctx.refresh();
                    hotdrop.agent.SpringHook.register(ctx);
                    // a child context, never registered by hand: HotDrop must learn of it through the parent
                    GenericApplicationContext kids = new GenericApplicationContext(ctx);
                    kids.setClassLoader(cl);
                    new XmlBeanDefinitionReader(kids).loadBeanDefinitions(new FileSystemResource(a[2]));
                    kids.refresh();
                    System.out.println("READY");
                    String lastKid = null, lastMsg = null;
                    String last = null, lastMap = null;
                    while (true) {
                        Object g = ctx.getBean("greeter");
                        String s = String.valueOf(g.getClass().getMethod("hello").invoke(g));
                        for (String n : new String[]{"plugin", "plugin2"}) {
                            if (ctx.containsBean(n)) {
                                Object p = ctx.getBean(n);
                                s += " | " + p.getClass().getMethod("hello").invoke(p);
                            }
                        }
                        if (!s.equals(last)) { System.out.println("CFG " + s); last = s; }
                        String msg = ctx.getMessage("hi", null, "?", java.util.Locale.ENGLISH);
                        if (!msg.equals(lastMsg)) { System.out.println("MSG " + msg); lastMsg = msg; }
                        Object kid = kids.getBean("kid");
                        String k = String.valueOf(kid.getClass().getMethod("hello").invoke(kid));
                        if (!k.equals(lastKid)) { System.out.println("KID " + k); lastKid = k; }
                        String map = ctx.getBean(RequestMappingHandlerMapping.class).getHandlerMethods().keySet().toString();
                        if (!map.equals(lastMap)) { System.out.println("MAP " + map); lastMap = map; }
                        Thread.sleep(5);
                    }
                }
            }
            """;

    static String greeter(String body) {
        return "package demo;\npublic class Greeter {\n  private String greeting; private String suffix; private java.util.List<String> names;\n"
                + "  public void setGreeting(String g) { greeting = g; }\n  public void setSuffix(String s) { suffix = s; }\n"
                + "  public void setNames(java.util.List<String> n) { names = n; }\n"
                + "  public String hello() { return greeting + suffix + \" \" + names" + body + "; }\n}\n";
    }

    static String plugin(String cls, String tag) {
        return "package demo;\npublic class " + cls + " {\n  private String name;\n  public void setName(String n) { name = n; }\n"
                + "  public String hello() { return \"" + tag + " \" + name; }\n}\n";
    }

    static String ctl(String path) {
        return "package demo;\n@org.springframework.stereotype.Controller\npublic class Ctl {\n"
                + "  @org.springframework.web.bind.annotation.RequestMapping(\"" + path + "\")\n  public String page() { return \"x\"; }\n}\n";
    }

    /** extras: more beans appended to the file. */
    static String xml(String greeting, String names, String extras) {
        return """
                <?xml version="1.0" encoding="UTF-8"?>
                <beans xmlns="http://www.springframework.org/schema/beans" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
                       xmlns:util="http://www.springframework.org/schema/util"
                       xsi:schemaLocation="http://www.springframework.org/schema/beans http://www.springframework.org/schema/beans/spring-beans.xsd
                                           http://www.springframework.org/schema/util http://www.springframework.org/schema/util/spring-util.xsd">
                  <bean class="org.springframework.context.support.PropertySourcesPlaceholderConfigurer">
                    <property name="properties"><props><prop key="suffix">!</prop><prop key="plug.name">px</prop><prop key="plug2.name">y</prop></props></property>
                  </bean>
                  <bean id="greeter" class="demo.Greeter">
                    <property name="greeting" value="GREETING"/>
                    <property name="suffix" value="${suffix}"/>
                    <property name="names" ref="names"/>
                  </bean>
                  <util:list id="names">NAMES</util:list>
                  <bean class="org.springframework.web.servlet.mvc.method.annotation.RequestMappingHandlerMapping"/>
                  <bean class="demo.Ctl"/>
                EXTRAS
                </beans>
                """.replace("GREETING", greeting).replace("NAMES", names).replace("EXTRAS", extras);
    }

    static String values(String... v) {
        StringBuilder sb = new StringBuilder();
        for (String s : v) sb.append("<value>").append(s).append("</value>");
        return sb.toString();
    }

    static String kid(String name) {
        return "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<beans xmlns=\"http://www.springframework.org/schema/beans\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" "
                + "xsi:schemaLocation=\"http://www.springframework.org/schema/beans http://www.springframework.org/schema/beans/spring-beans.xsd\">\n"
                + "  <bean id=\"kid\" class=\"demo.Plugin\"><property name=\"name\" value=\"" + name + "\"/></bean>\n</beans>\n";
    }

    static final String PLUGIN_BEAN = "<bean id=\"plugin\" class=\"demo.Plugin\"><property name=\"name\" value=\"${plug.name}\"/></bean>";
    static final String PLUGIN2_BEAN = "<bean id=\"plugin2\" class=\"demo.Plugin2\"><property name=\"name\" value=\"${plug2.name}\"/></bean>";

    public static void main(String[] args) throws Exception {
        List<Path> jars = springJars();
        if (jars.isEmpty()) {
            System.out.println("SKIPPED: no Spring jars found (set HOTDROP_SPRING_LIB)");
            return;
        }
        String cp = join(jars);
        Path repo = Path.of(System.getProperty("user.dir"));
        Path agentJar = repo.resolve("build/hotdrop-agent.jar");
        Path daemonJar = repo.resolve("build/hotdrop-daemon.jar");
        Path work = Files.createTempDirectory("hotdrop-spring-it");
        src = work.resolve("src");
        Path classes = work.resolve("classes");
        Path res = work.resolve("resources");
        Path appDir = work.resolve("app");
        Path home = work.resolve("home");
        Files.createDirectories(src.resolve("demo"));
        Files.createDirectories(classes);
        Files.createDirectories(res);
        Files.createDirectories(appDir);
        xml = res.resolve("demo-spring.xml");
        kidXml = res.resolve("kid-web-spring.xml");
        itemsXml = res.resolve("demo-items.xml");
        Files.createDirectories(res.resolve("localization"));
        messages = res.resolve("localization/msg.properties");
        Files.writeString(messages, "hi=one\n");
        Files.writeString(itemsXml, "<items><itemtypes><itemtype code=\"Demo\"/></itemtypes></items>\n");
        Files.writeString(src.resolve("demo/Greeter.java"), greeter(""));
        Files.writeString(src.resolve("demo/Plugin.java"), plugin("Plugin", "PLUGIN"));
        Files.writeString(src.resolve("demo/Ctl.java"), ctl("/a"));
        Files.writeString(xml, xml("hi", values("a"), ""));
        Files.writeString(kidXml, kid("one"));
        Files.writeString(appDir.resolve("SApp.java"), APP);

        JavaCompiler jc = ToolProvider.getSystemJavaCompiler();
        check("initial compile of demo sources", jc.run(null, null, null, "-g", "-cp", cp, "-d", classes.toString(),
                src.resolve("demo/Greeter.java").toString(), src.resolve("demo/Plugin.java").toString(), src.resolve("demo/Ctl.java").toString()) == 0);
        check("compile app", jc.run(null, null, null, "-cp", cp + File.pathSeparator + agentJar, "-d", appDir.toString(),
                appDir.resolve("SApp.java").toString()) == 0);

        boolean windows = System.getProperty("os.name").toLowerCase().contains("win");
        String java = Path.of(System.getProperty("java.home"), "bin", windows ? "java.exe" : "java").toString();
        Process app = start(work, appOut, "app", java, "-javaagent:" + agentJar + "=dir=" + home.resolve("agents"),
                "-cp", appDir + File.pathSeparator + cp, "SApp", classes.toString(), xml.toString(), kidXml.toString(), res.resolve("localization").toString());
        Process daemon = null;
        try {
            check("app started with a Spring context", waitFor(appOut, "READY", 15_000) >= 0);
            check("app prints the first value", waitFor(appOut, "CFG hi! [a]", 5_000) >= 0);
            check("MVC mapping /a registered", waitFor(appOut, "/a", 2_000) >= 0);

            daemon = start(work, daemonOut, "daemon", java, "-jar", daemonJar.toString(), "start", "--root", src + "=" + classes,
                    "--spring-dir", res.toString(), "--messages-dir", res.resolve("localization").toString(), "--home", home.toString(), "--debug");
            check("daemon connected to the agent", waitFor(daemonOut, "agent connected", 10_000) >= 0);
            check("daemon indexed the sources", waitFor(daemonOut, "indexed", 20_000) >= 0);
            Thread.sleep(500);

            step("1. a property value in the XML changes");
            long ms = editXml(xml("hello", values("a"), ""), "CFG hello! [a]");
            check("   live in the running context", ms >= 0);
            latency("xml property change", ms);
            check("   server console says so", waitFor(appOut, "[HotDrop] Spring bean 'greeter': greeting updated", 3_000) >= 0);

            step("2. a util:list changes");
            ms = editXml(xml("hello", values("a", "b"), ""), "CFG hello! [a, b]");
            check("   list refilled in place; the bean holding it sees it", ms >= 0);
            latency("xml util:list change", ms);

            step("3. a new bean is added (with a ${placeholder})");
            ms = editXml(xml("hello", values("a", "b"), PLUGIN_BEAN), "CFG hello! [a, b] | PLUGIN px");
            check("   bean created and resolved", ms >= 0);
            latency("xml new bean", ms);

            step("4. a new class and a bean using it are saved together");
            Files.writeString(src.resolve("demo/Plugin2.java"), plugin("Plugin2", "PLUGIN2"));
            ms = editXml(xml("hello", values("a", "b"), PLUGIN_BEAN + PLUGIN2_BEAN), "CFG hello! [a, b] | PLUGIN px | PLUGIN2 y");
            check("   class written first, then the bean", ms >= 0);
            latency("new class + new bean", ms);

            step("5. a class change on an existing bean is restart required");
            String before = lastCfg();
            Files.writeString(xml, xml("hello", values("a", "b"),
                    PLUGIN_BEAN.replace("demo.Plugin\"", "demo.Plugin2\"") + PLUGIN2_BEAN));
            check("   daemon says restart required", waitFor(daemonOut, "[restart required] demo-spring.xml: bean 'plugin'", 5_000) >= 0);
            Thread.sleep(400);
            check("   running context untouched", before.equals(lastCfg()));
            Files.writeString(xml, xml("hello", values("a", "b"), PLUGIN_BEAN + PLUGIN2_BEAN));
            Thread.sleep(600);

            step("6. a bean that cannot be created is reported and removed, then fixed");
            before = lastCfg();
            Files.writeString(xml, xml("hello", values("a", "b"), PLUGIN_BEAN + PLUGIN2_BEAN
                    + "<bean id=\"bad\" class=\"demo.Missing\"/>"));
            check("   daemon reports the failure", waitFor(daemonOut, "bean 'bad': ", 5_000) >= 0);
            Thread.sleep(300);
            check("   running context unharmed", before.equals(lastCfg()));
            Files.writeString(xml, xml("hello", values("a", "b"), PLUGIN_BEAN + PLUGIN2_BEAN
                    + "<bean id=\"bad\" class=\"demo.Plugin\"><property name=\"name\" value=\"fixed\"/></bean>"));
            check("   fixing the file adds the bean", waitFor(daemonOut, "bean 'bad' added", 5_000) >= 0);

            step("7. a Java change after Spring work still swaps");
            ms = edit("demo/Greeter.java", greeter(" + \"#\""), "CFG hello! [a, b]# | PLUGIN px | PLUGIN2 y");
            check("   class swapped with Spring caches cleared", ms >= 0);

            step("8. a changed @RequestMapping rebuilds the MVC mapping");
            long t0 = System.nanoTime();
            Files.writeString(src.resolve("demo/Ctl.java"), ctl("/b"));
            check("   mapping moved to /b", waitForSince(appOut, "/b", 5_000));
            latency("mvc mapping change", (System.nanoTime() - t0) / 1_000_000);
            check("   server console says so", waitFor(appOut, "Spring MVC mappings rebuilt", 3_000) >= 0);

            step("9. a child context found through its parent");
            check("   child starts as one", waitFor(appOut, "KID PLUGIN one", 2_000) >= 0);
            t0 = System.nanoTime();
            Files.writeString(kidXml, kid("two"));
            check("   its XML change is applied", waitFor(appOut, "KID PLUGIN two", 5_000) >= 0);
            latency("child context property", (System.nanoTime() - t0) / 1_000_000);

            step("10. a file with a DOCTYPE is still read by the daemon");
            Files.writeString(kidXml, "<?xml version=\"1.0\"?>\n<!DOCTYPE beans PUBLIC \"-//SPRING//DTD BEAN 2.0//EN\" \"https://www.springframework.org/dtd/spring-beans-2.0.dtd\">\n"
                    + "<beans><bean id=\"kid\" class=\"demo.Plugin\"><property name=\"name\" value=\"three\"/></bean></beans>\n");
            // Spring 5.3 can load the DTD from its jar and applies it; Spring 6 dropped DTD support and reports the file as not loadable.
            check("   daemon diffed it (applied, or reported by Spring)",
                    waitFor(appOut, "KID PLUGIN three", 5_000) >= 0 || waitFor(daemonOut, "[not applied] kid-web-spring.xml", 1_000) >= 0);
            check("   daemon never called it malformed", waitFor(daemonOut, "not well-formed", 0) < 0);

            step("11. a message bundle changes");
            check("   bundle starts as one", waitFor(appOut, "MSG one", 2_000) >= 0);
            t0 = System.nanoTime();
            Files.writeString(messages, "hi=two\n");
            check("   next lookup returns the new text", waitFor(appOut, "MSG two", 5_000) >= 0);
            latency("message bundle change", (System.nanoTime() - t0) / 1_000_000);

            step("12. items.xml: a real edit is reported, a touch is not");
            Files.writeString(itemsXml, "<items><itemtypes><itemtype code=\"Demo\"/></itemtypes></items>\n");
            Thread.sleep(800);
            check("   identical content is silent", waitFor(daemonOut, "demo-items.xml", 0) < 0);
            Files.writeString(itemsXml, "<items><itemtypes><itemtype code=\"Demo\"/><itemtype code=\"Other\"/></itemtypes></items>\n");
            check("   daemon says restart required", waitFor(daemonOut, "[restart required] demo-items.xml", 5_000) >= 0);
            check("   server console says so", waitFor(appOut, "demo-items.xml changed - the type system changed", 3_000) >= 0);

            step("13. status mentions the Spring files");
            Process st = new ProcessBuilder(java, "-jar", daemonJar.toString(), "status", "--home", home.toString())
                    .redirectErrorStream(true).start();
            String status = new String(st.getInputStream().readAllBytes());
            st.waitFor();
            check("   status lists watched XML files", status.contains("spring:"));
        } finally {
            if (daemon != null) daemon.destroy();
            app.destroy();
        }

        System.out.println("\nlatency (file write -> change visible in the running JVM):");
        latencies.forEach(l -> System.out.println("  " + l));
        System.out.println(failures == 0 ? "\nALL CHECKS PASSED" : "\n" + failures + " CHECK(S) FAILED");
        if (System.getenv("SHOW_LOG") != null || failures > 0) {
            System.out.println("\n--- daemon output ---");
            synchronized (daemonOut) { daemonOut.forEach(System.out::println); }
            System.out.println("\n--- app output ---");
            synchronized (appOut) { appOut.forEach(System.out::println); }
        }
        System.exit(failures == 0 ? 0 : 1);
    }

    // ---- helpers ----

    static List<Path> springJars() throws IOException {
        List<Path> dirs = new ArrayList<>();
        String env = System.getenv("HOTDROP_SPRING_LIB");
        List<Path> jars = new ArrayList<>();
        if (env != null && !env.isBlank()) {
            for (String e : env.split(File.pathSeparator)) {
                Path p = Path.of(e);
                if (Files.isDirectory(p)) dirs.add(p);
                else if (Files.isRegularFile(p)) jars.add(p);
            }
        } else {
            Path home = Path.of(System.getProperty("user.home"));
            for (Path base : List.of(home.resolve("work/HybrisBackups"), home.resolve("work/cloud"))) {
                if (!Files.isDirectory(base)) continue;
                try (Stream<Path> s = Files.walk(base, 9)) {
                    s.filter(p -> p.endsWith(Path.of("platform", "ext", "core", "lib"))).findFirst().ifPresent(dirs::add);
                }
                if (!dirs.isEmpty()) {
                    Path platform = dirs.get(0).getParent().getParent().getParent();
                    Path servlet = platform.resolve("tomcat/lib/servlet-api.jar");
                    if (Files.isRegularFile(servlet)) jars.add(servlet);
                    break;
                }
            }
        }
        for (Path d : dirs) {
            try (Stream<Path> s = Files.list(d)) {
                s.filter(p -> {
                    String n = p.getFileName().toString();
                    return n.matches("spring-(core|beans|context|aop|expression|web|webmvc|jcl)-[0-9].*\\.jar")
                            || n.matches("commons-logging-[0-9].*\\.jar");
                }).sorted().forEach(jars::add);
            }
        }
        boolean hasWebmvc = jars.stream().anyMatch(p -> p.getFileName().toString().startsWith("spring-webmvc"));
        return hasWebmvc ? jars : List.of();
    }

    static String join(List<Path> paths) {
        StringBuilder sb = new StringBuilder();
        for (Path p : paths) sb.append(sb.length() == 0 ? "" : File.pathSeparator).append(p);
        return sb.toString();
    }

    static String lastCfg() {
        synchronized (appOut) {
            for (int i = appOut.size() - 1; i >= 0; i--) if (appOut.get(i).startsWith("CFG ")) return appOut.get(i);
        }
        return "";
    }

    static long editXml(String content, String expectCfg) throws Exception {
        long t0 = System.nanoTime();
        Files.writeString(xml, content);
        return waitFor(appOut, expectCfg, 6_000) < 0 ? -1 : (System.nanoTime() - t0) / 1_000_000;
    }

    static long edit(String rel, String content, String expect) throws Exception {
        long t0 = System.nanoTime();
        Files.writeString(src.resolve(rel), content);
        return waitFor(appOut, expect, 6_000) < 0 ? -1 : (System.nanoTime() - t0) / 1_000_000;
    }

    /** Waits for a MAP line (printed after the mapping changed) that contains the needle. */
    static boolean waitForSince(List<String> out, String needle, long timeoutMs) throws InterruptedException {
        long end = System.currentTimeMillis() + timeoutMs;
        do {
            synchronized (out) {
                for (int i = out.size() - 1; i >= 0; i--) {
                    if (out.get(i).startsWith("MAP ")) {
                        if (out.get(i).contains(needle)) return true;
                        break;
                    }
                }
            }
            Thread.sleep(5);
        } while (System.currentTimeMillis() < end);
        return false;
    }

    static void step(String name) { System.out.println(name); }

    static void latency(String what, long ms) { latencies.add(String.format("%-28s %d ms", what, ms)); }

    static void check(String what, boolean ok) {
        System.out.println((ok ? "  PASS " : "  FAIL ") + what.strip());
        if (!ok) failures++;
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
        Process p = new ProcessBuilder(cmd).directory(dir.toFile()).redirectErrorStream(true).start();
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
