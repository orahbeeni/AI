import com.sun.net.httpserver.HttpExchange;
import com.sun.net.httpserver.HttpServer;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStreamReader;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.URLDecoder;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Daemon-only checks (no server JVM needed): `doctor` on a fake Hybris tree, the Backoffice config and items.xml notices,
 * and ImpEx on save against a stub that imitates the HAC pages. The stub follows what HAC is believed to do; it proves the
 * client logic, not that a real HAC answers the same way.
 * Run: java it/ResourcesIntegrationTest.java   (after java Build.java)
 */
public class ResourcesIntegrationTest {
    static final List<String> daemonOut = Collections.synchronizedList(new ArrayList<>());
    static final List<Map<String, String>> imports = Collections.synchronizedList(new ArrayList<>());
    static int logins;
    static int failures;

    public static void main(String[] args) throws Exception {
        Path repo = Path.of(System.getProperty("user.dir"));
        Path daemonJar = repo.resolve("build/hotdrop-daemon.jar");
        Path work = Files.createTempDirectory("hotdrop-res-it");
        boolean windows = System.getProperty("os.name").toLowerCase().contains("win");
        String java = Path.of(System.getProperty("java.home"), "bin", windows ? "java.exe" : "java").toString();

        // a fake hybris tree: one custom extension with the directories HotDrop watches
        Path hybris = work.resolve("hybris");
        Path ext = hybris.resolve("bin/custom/acme");
        Files.createDirectories(hybris.resolve("bin/platform"));
        Files.createDirectories(hybris.resolve("config"));
        Files.createDirectories(ext.resolve("src/acme"));
        Files.createDirectories(ext.resolve("resources/localization"));
        Files.createDirectories(ext.resolve("resources/impex/sub"));
        Files.createDirectories(ext.resolve("backoffice/resources"));
        Files.writeString(ext.resolve("extensioninfo.xml"), "<extensioninfo><extension name=\"acme\"/></extensioninfo>");
        Files.writeString(ext.resolve("src/acme/A.java"), "package acme; public class A {}\n");
        Files.writeString(hybris.resolve("bin/platform/project.properties"), "tomcat.development.mode=true\n");
        Files.writeString(hybris.resolve("config/local.properties"), "tomcat.development.mode=false\n");
        Path items = ext.resolve("resources/acme-items.xml");
        Path boConfig = ext.resolve("backoffice/resources/acme-backoffice-config.xml");
        Files.writeString(items, "<items/>\n");
        Files.writeString(boConfig, "<config/>\n");

        System.out.println("doctor on a fake tree");
        Process doc = new ProcessBuilder(java, "-jar", daemonJar.toString(), "doctor", "--hybris", hybris.toString(),
                "--home", work.resolve("home").toString()).redirectErrorStream(true).start();
        String out = new String(doc.getInputStream().readAllBytes());
        doc.waitFor();
        check("local.properties overrides the default and doctor warns about JSP", out.contains("tomcat.development.mode=false")
                && out.contains("JSP and tag edits need a restart"));
        check("doctor lists watched resource directories", out.contains("Spring/model dir(s)") && out.contains("message dir(s)"));
        Files.writeString(hybris.resolve("config/local.properties"), "# nothing\n");
        doc = new ProcessBuilder(java, "-jar", daemonJar.toString(), "doctor", "--hybris", hybris.toString(),
                "--home", work.resolve("home").toString()).redirectErrorStream(true).start();
        out = new String(doc.getInputStream().readAllBytes());
        doc.waitFor();
        check("development mode on is reported as fine", out.contains("tomcat.development.mode=true") && out.contains("recompiles edited JSPs"));

        // stub HAC
        HttpServer hac = HttpServer.create(new InetSocketAddress(InetAddress.getLoopbackAddress(), 0), 0);
        hac.createContext("/hac/login.jsp", ex -> reply(ex, 200, "<html><meta name=\"_csrf\" content=\"tok-login\"/></html>", null));
        hac.createContext("/hac/j_spring_security_check", ex -> {
            Map<String, String> f = form(ex);
            boolean ok = "admin".equals(f.get("j_username")) && "nimda".equals(f.get("j_password")) && "tok-login".equals(f.get("_csrf"));
            if (ok) logins++;
            ex.getResponseHeaders().add("Set-Cookie", "JSESSIONID=abc; Path=/hac");
            ex.getResponseHeaders().add("Location", ok ? "/hac/" : "/hac/login.jsp?login_error=1");
            reply(ex, 302, "", null);
        });
        hac.createContext("/hac/console/impex/import", ex -> {
            if (ex.getRequestMethod().equals("GET")) {
                reply(ex, 200, "<html><meta name=\"_csrf\" content=\"tok-import\"/></html>", null);
                return;
            }
            Map<String, String> f = form(ex);
            imports.add(f);
            boolean bad = f.getOrDefault("scriptContent", "").contains("BROKEN");
            reply(ex, 200, "<div id=\"impexResult\" data-level=\"" + (bad ? "error" : "notice") + "\" data-result=\""
                    + (bad ? "Line 3: unknown type &quot;Nope&quot;" : "Import finished successfully") + "\"></div>", null);
        });
        hac.start();
        String hacUrl = "http://127.0.0.1:" + hac.getAddress().getPort() + "/hac";

        System.out.println("running daemon");
        Process daemon = new ProcessBuilder(java, "-jar", daemonJar.toString(), "start", "--hybris", hybris.toString(), "--impex",
                "--impex-dir", ext.resolve("resources/impex").toString(), "--hac", hacUrl, "--home", work.resolve("home").toString(), "--debug")
                .directory(work.toFile()).redirectErrorStream(true).start();
        Thread t = new Thread(() -> {
            try (BufferedReader r = new BufferedReader(new InputStreamReader(daemon.getInputStream()))) {
                String line;
                while ((line = r.readLine()) != null) daemonOut.add(line);
            } catch (IOException ignored) { }
        });
        t.setDaemon(true);
        t.start();
        try {
            check("daemon indexed", waitFor("indexed", 20_000) >= 0);
            check("daemon watches ImpEx directories", waitFor("ImpEx files", 2_000) >= 0);
            Thread.sleep(600);

            System.out.println("Backoffice config and items.xml");
            Files.writeString(boConfig, "<config><changed/></config>\n");
            check("backoffice config change is noted", waitFor("[note] acme-backoffice-config.xml", 5_000) >= 0);
            Files.writeString(items, "<items><itemtypes/></items>\n");
            check("items.xml change is restart required", waitFor("[restart required] acme-items.xml", 5_000) >= 0);

            System.out.println("ImpEx on save");
            Path plain = ext.resolve("resources/impex/plain.impex");
            Files.writeString(plain, "INSERT_UPDATE Title;code[unique=true]\n;mr\n");
            check("a file without the marker is not run", waitFor("add a line '# hotdrop-on-save'", 5_000) >= 0 && imports.isEmpty());

            Path auto = ext.resolve("resources/impex/sub/auto.impex");
            Files.writeString(auto, "# hotdrop-on-save\nINSERT_UPDATE Title;code[unique=true]\n;dr\n");
            check("a file with the marker is imported", waitFor("[impex] auto.impex imported", 8_000) >= 0);
            Map<String, String> sent = imports.isEmpty() ? Map.of() : imports.get(0);
            check("   the script text arrived", sent.getOrDefault("scriptContent", "").contains(";dr"));
            check("   with the CSRF token and strict validation", "tok-import".equals(sent.get("_csrf")) && "IMPORT_STRICT".equals(sent.get("validationEnum")));
            check("   after one login", logins == 1);

            Files.writeString(auto, "# hotdrop-on-save\nBROKEN\n");
            check("a failing import is reported with the server's message", waitFor("[impex failed] auto.impex: Line 3: unknown type \"Nope\"", 8_000) >= 0);
            Files.writeString(auto, "# hotdrop-on-save\nINSERT_UPDATE Title;code[unique=true]\n;prof\n");
            check("fixing it imports again, still one login", waitFor("[impex] auto.impex imported", 8_000) >= 0 && logins == 1);
        } finally {
            daemon.destroy();
            hac.stop(0);
        }
        System.out.println(failures == 0 ? "\nALL CHECKS PASSED" : "\n" + failures + " CHECK(S) FAILED");
        if (failures > 0) synchronized (daemonOut) { daemonOut.forEach(System.out::println); }
        System.exit(failures == 0 ? 0 : 1);
    }

    static Map<String, String> form(HttpExchange ex) throws IOException {
        String body = new String(ex.getRequestBody().readAllBytes(), StandardCharsets.UTF_8);
        Map<String, String> m = new LinkedHashMap<>();
        for (String kv : body.split("&")) {
            int i = kv.indexOf('=');
            if (i > 0) m.put(URLDecoder.decode(kv.substring(0, i), StandardCharsets.UTF_8), URLDecoder.decode(kv.substring(i + 1), StandardCharsets.UTF_8));
        }
        return m;
    }

    static void reply(HttpExchange ex, int code, String body, String type) throws IOException {
        byte[] b = body.getBytes(StandardCharsets.UTF_8);
        ex.getResponseHeaders().add("Content-Type", type == null ? "text/html" : type);
        ex.sendResponseHeaders(code, b.length == 0 ? -1 : b.length);
        if (b.length > 0) ex.getResponseBody().write(b);
        ex.close();
    }

    static void check(String what, boolean ok) {
        System.out.println((ok ? "  PASS " : "  FAIL ") + what.strip());
        if (!ok) failures++;
    }

    static int waitFor(String needle, long timeoutMs) throws InterruptedException {
        long end = System.currentTimeMillis() + timeoutMs;
        do {
            synchronized (daemonOut) {
                for (int i = 0; i < daemonOut.size(); i++) if (daemonOut.get(i).contains(needle)) return i;
            }
            Thread.sleep(5);
        } while (System.currentTimeMillis() < end);
        return -1;
    }
}
