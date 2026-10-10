package hotdrop.daemon;

import javax.net.ssl.SSLContext;
import javax.net.ssl.SSLParameters;
import javax.net.ssl.TrustManager;
import javax.net.ssl.X509TrustManager;
import java.io.IOException;
import java.net.CookieManager;
import java.net.URI;
import java.net.URLEncoder;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.cert.X509Certificate;
import java.time.Duration;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Runs an ImpEx file in the running server through the HAC console (the same form the HAC ImpEx import page posts),
 * which takes care of the tenant, the admin session and the import service. Opt-in; see Config.impex.
 * <p>
 * The HAC protocol used here (login form, CSRF token, impex import form fields, result element) is written from
 * knowledge of HAC, not checked against a real server; failures are reported with what the server answered.
 * TLS certificate checks are skipped only when the host is a loopback address, because HAC's dev certificate is self-signed.
 */
final class ImpexRunner {
    private static final Pattern CSRF_META = Pattern.compile("<meta\\s+name=\"_csrf\"\\s+content=\"([^\"]+)\"");
    private static final Pattern CSRF_INPUT = Pattern.compile("name=\"_csrf\"\\s+value=\"([^\"]+)\"");
    private static final Pattern RESULT = Pattern.compile("id=\"impexResult\"[^>]*");
    private static final Pattern LEVEL = Pattern.compile("data-level=\"([^\"]*)\"");
    private static final Pattern RESULT_TEXT = Pattern.compile("data-result=\"([^\"]*)\"");

    private final URI base;
    private final String user;
    private final String password;
    private final HttpClient http;
    private boolean loggedIn;

    ImpexRunner(String hacUrl, String user, String password) throws Exception {
        String u = hacUrl.endsWith("/") ? hacUrl.substring(0, hacUrl.length() - 1) : hacUrl;
        this.base = URI.create(u);
        this.user = user;
        this.password = password;
        HttpClient.Builder b = HttpClient.newBuilder().cookieHandler(new CookieManager())
                .followRedirects(HttpClient.Redirect.NORMAL).connectTimeout(Duration.ofSeconds(5));
        String host = base.getHost() == null ? "" : base.getHost();
        if ("https".equals(base.getScheme())) {
            if (!(host.equals("localhost") || host.equals("127.0.0.1") || host.equals("::1") || host.equals("[::1]"))) {
                throw new IllegalArgumentException("--hac must be a localhost address when it is https (self-signed certificates are accepted)");
            }
            SSLContext ctx = SSLContext.getInstance("TLS");
            ctx.init(null, new TrustManager[]{new X509TrustManager() {
                public void checkClientTrusted(X509Certificate[] c, String a) { }
                public void checkServerTrusted(X509Certificate[] c, String a) { }
                public X509Certificate[] getAcceptedIssuers() { return new X509Certificate[0]; }
            }}, null);
            SSLParameters params = new SSLParameters();
            params.setEndpointIdentificationAlgorithm("");
            b.sslContext(ctx).sslParameters(params);
        }
        this.http = b.build();
    }

    /** Imports the file; returns the server's summary. Throws with the server's message when the import fails. */
    synchronized String run(Path file) throws Exception {
        String script = Files.readString(file, StandardCharsets.UTF_8);
        for (int attempt = 0; ; attempt++) {
            if (!loggedIn) login();
            String page = get("/console/impex/import");
            String csrf = csrf(page);
            if (csrf == null) {
                loggedIn = false;
                if (attempt == 0) continue;
                throw new IOException("the HAC ImpEx page has no CSRF token (not logged in, or not a HAC address?)");
            }
            Map<String, String> form = new LinkedHashMap<>();
            form.put("scriptContent", script);
            form.put("validationEnum", "IMPORT_STRICT");
            form.put("encoding", "UTF-8");
            form.put("maxThreads", "1");
            form.put("_csrf", csrf);
            HttpResponse<String> r = post("/console/impex/import", form, csrf);
            String body = r.body();
            Matcher m = RESULT.matcher(body);
            if (!m.find()) {
                if (r.statusCode() == 403 || body.contains("j_spring_security_check")) {
                    loggedIn = false;
                    if (attempt == 0) continue;
                }
                throw new IOException("unexpected answer from HAC (HTTP " + r.statusCode() + ")");
            }
            Matcher lv = LEVEL.matcher(m.group());
            Matcher tx = RESULT_TEXT.matcher(m.group());
            String level = lv.find() ? lv.group(1) : "";
            String text = tx.find() ? unescape(tx.group(1)) : "";
            if (level.equalsIgnoreCase("error")) throw new IOException(text.isEmpty() ? "import failed" : text);
            return text.isEmpty() ? "imported" : text;
        }
    }

    private void login() throws Exception {
        String page = get("/login.jsp");
        String csrf = csrf(page);
        Map<String, String> form = new LinkedHashMap<>();
        form.put("j_username", user);
        form.put("j_password", password);
        if (csrf != null) form.put("_csrf", csrf);
        HttpResponse<String> r = post("/j_spring_security_check", form, csrf);
        if (r.uri().toString().contains("login_error") || r.statusCode() == 401 || r.statusCode() == 403) {
            throw new IOException("HAC login failed for user '" + user + "' (set --hac-user / --hac-password)");
        }
        loggedIn = true;
    }

    private String get(String path) throws Exception {
        HttpRequest req = HttpRequest.newBuilder(URI.create(base + path)).timeout(Duration.ofSeconds(30)).GET().build();
        return http.send(req, HttpResponse.BodyHandlers.ofString()).body();
    }

    private HttpResponse<String> post(String path, Map<String, String> form, String csrf) throws Exception {
        StringBuilder sb = new StringBuilder();
        for (var e : form.entrySet()) {
            if (sb.length() > 0) sb.append('&');
            sb.append(URLEncoder.encode(e.getKey(), StandardCharsets.UTF_8)).append('=')
                    .append(URLEncoder.encode(e.getValue(), StandardCharsets.UTF_8));
        }
        HttpRequest.Builder b = HttpRequest.newBuilder(URI.create(base + path)).timeout(Duration.ofMinutes(5))
                .header("Content-Type", "application/x-www-form-urlencoded");
        if (csrf != null) b.header("X-CSRF-TOKEN", csrf);
        return http.send(b.POST(HttpRequest.BodyPublishers.ofString(sb.toString())).build(), HttpResponse.BodyHandlers.ofString());
    }

    static String csrf(String html) {
        Matcher m = CSRF_META.matcher(html);
        if (m.find()) return m.group(1);
        m = CSRF_INPUT.matcher(html);
        return m.find() ? m.group(1) : null;
    }

    private static String unescape(String s) {
        return s.replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&#39;", "'")
                .replace("&#10;", "\n").replace("&amp;", "&");
    }

    /** True when the file asks to be run on save: a comment line "# hotdrop-on-save" near the top. */
    static boolean hasMarker(Path file) {
        try (var lines = Files.lines(file, StandardCharsets.UTF_8)) {
            return lines.limit(20).anyMatch(l -> {
                String t = l.strip().toLowerCase();
                return t.startsWith("#") && t.contains("hotdrop-on-save");
            });
        } catch (IOException | RuntimeException e) {
            return false;
        }
    }
}
