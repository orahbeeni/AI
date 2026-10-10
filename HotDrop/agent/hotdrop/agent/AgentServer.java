package hotdrop.agent;

import hotdrop.protocol.Wire;
import hotdrop.protocol.Wire.AgentInfo;
import hotdrop.protocol.Wire.BatchResult;
import hotdrop.protocol.Wire.ClassEntry;
import hotdrop.protocol.Wire.ClassResult;
import hotdrop.protocol.Wire.Frame;
import hotdrop.protocol.Wire.LoaderInfo;

import java.io.BufferedInputStream;
import java.io.BufferedOutputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.EOFException;
import java.io.IOException;
import java.lang.instrument.ClassDefinition;
import java.lang.instrument.Instrumentation;
import java.lang.management.ManagementFactory;
import java.net.InetAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.net.SocketException;
import java.net.URI;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.nio.file.attribute.PosixFilePermissions;
import java.security.CodeSource;
import java.security.MessageDigest;
import java.security.ProtectionDomain;
import java.security.SecureRandom;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.IdentityHashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Properties;
import java.util.Set;

/**
 * Loopback-only server inside the Hybris JVM. It does exactly three things: reports the classloader tree,
 * redefines classes the daemon has compiled, and answers pings. It never compiles anything.
 */
final class AgentServer {
    private final Instrumentation inst;
    private final Map<String, String> opts;
    private final byte[] token;
    private final ServerSocket server;
    private final Path infoFile;
    private final SpringBridge spring = new SpringBridge(this::say);

    AgentServer(Instrumentation inst, Map<String, String> opts) throws IOException {
        this.inst = inst;
        this.opts = opts;
        byte[] raw = new byte[32];
        new SecureRandom().nextBytes(raw);
        this.token = hex(raw).getBytes(StandardCharsets.UTF_8);
        this.server = new ServerSocket(0, 8, InetAddress.getLoopbackAddress());
        Path dir = opts.containsKey("dir")
                ? Path.of(opts.get("dir"))
                : Path.of(System.getProperty("user.home"), ".hotdrop", "agents");
        Files.createDirectories(dir);
        this.infoFile = dir.resolve(ProcessHandleCompat.pid() + ".properties");
    }

    void start(boolean autostart) throws IOException {
        writeInfoFile();
        Runtime.getRuntime().addShutdownHook(new Thread(() -> {
            try {
                Files.deleteIfExists(infoFile);
            } catch (IOException ignored) {
                // best effort
            }
        }, "hotdrop-agent-cleanup"));
        if (!"false".equals(opts.get("spring"))) spring.startFinder(inst);
        Thread t = new Thread(this::acceptLoop, "hotdrop-agent");
        t.setDaemon(true);
        t.start();
        if (!"true".equals(opts.get("quiet"))) {
            System.out.println("[HotDrop] agent listening on 127.0.0.1:" + server.getLocalPort()
                    + " (pid " + ProcessHandleCompat.pid() + ", enhanced redefinition: " + enhanced() + ")");
        }
        if (autostart) DaemonLauncher.launchInBackground(opts, infoFile.getParent());
    }

    private void writeInfoFile() throws IOException {
        Properties p = new Properties();
        p.setProperty("port", String.valueOf(server.getLocalPort()));
        p.setProperty("token", new String(token, StandardCharsets.UTF_8));
        p.setProperty("pid", String.valueOf(ProcessHandleCompat.pid()));
        p.setProperty("startMillis", String.valueOf(ManagementFactory.getRuntimeMXBean().getStartTime()));
        p.setProperty("platformHome", platformHome());
        p.setProperty("javaHome", System.getProperty("java.home", ""));
        Path tmp = Files.createTempFile(infoFile.getParent(), "agent", ".tmp");
        try {
            try {
                Files.setPosixFilePermissions(tmp, PosixFilePermissions.fromString("rw-------"));
            } catch (UnsupportedOperationException ignored) {
                // not a POSIX file system
            }
            try (var w = Files.newBufferedWriter(tmp, StandardCharsets.UTF_8)) {
                p.store(w, "HotDrop agent");
            }
            try {
                Files.move(tmp, infoFile, StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE);
            } catch (java.nio.file.AtomicMoveNotSupportedException e) {
                Files.move(tmp, infoFile, StandardCopyOption.REPLACE_EXISTING);
            }
        } finally {
            Files.deleteIfExists(tmp);
        }
    }

    private void acceptLoop() {
        while (!server.isClosed()) {
            try {
                Socket s = server.accept();
                Thread t = new Thread(() -> serve(s), "hotdrop-agent-conn");
                t.setDaemon(true);
                t.start();
            } catch (IOException e) {
                if (!server.isClosed()) System.err.println("[HotDrop] accept failed: " + e);
            }
        }
    }

    private void serve(Socket s) {
        try (s) {
            s.setTcpNoDelay(true);
            s.setSoTimeout(5000);
            DataInputStream in = new DataInputStream(new BufferedInputStream(s.getInputStream()));
            DataOutputStream out = new DataOutputStream(new BufferedOutputStream(s.getOutputStream()));
            Frame hello = Wire.read(in);
            if (hello.type() != Wire.HELLO || !authorised(hello.payload())) {
                Wire.write(out, Wire.ERROR, new Wire.Out().str("unauthorised").done());
                return;
            }
            s.setSoTimeout(0);
            Wire.write(out, Wire.HELLO_OK, Wire.encodeAgentInfo(info()));
            while (true) {
                Frame f = Wire.read(in);
                switch (f.type()) {
                    case Wire.PING -> Wire.write(out, Wire.PONG, new byte[0]);
                    case Wire.INVENTORY_REQ -> Wire.write(out, Wire.INVENTORY, Wire.encodeInventory(inventory()));
                    case Wire.NOTICE -> {
                        say(new Wire.In(f.payload()).str());
                        Wire.write(out, Wire.PONG, new byte[0]);
                    }
                    case Wire.CLEAR_MESSAGES -> {
                        spring.clearMessages();
                        Wire.write(out, Wire.PONG, new byte[0]);
                    }
                    case Wire.SPRING -> Wire.write(out, Wire.SPRING_RESULT,
                            Wire.encodeSpringResult(spring.apply(Wire.decodeSpring(f.payload()))));
                    case Wire.REDEFINE -> Wire.write(out, Wire.RESULT, Wire.encodeResult(redefine(f.payload())));
                    default -> Wire.write(out, Wire.ERROR, new Wire.Out().str("unknown message " + f.type()).done());
                }
            }
        } catch (EOFException | SocketException ignored) {
            // daemon went away
        } catch (Throwable t) {
            System.err.println("[HotDrop] connection error: " + t);
        }
    }

    private boolean authorised(byte[] payload) {
        try {
            Wire.In in = new Wire.In(payload);
            int version = in.i32();
            byte[] given = in.str().getBytes(StandardCharsets.UTF_8);
            return version == Wire.PROTOCOL_VERSION && MessageDigest.isEqual(given, token);
        } catch (IOException e) {
            return false;
        }
    }

    private AgentInfo info() {
        return new AgentInfo(Wire.PROTOCOL_VERSION, ProcessHandleCompat.pid(),
                System.getProperty("java.vm.vendor", ""), System.getProperty("java.vm.version", ""),
                System.getProperty("java.home", ""), System.getProperty("java.version", ""),
                inst.isRedefineClassesSupported(), enhanced(),
                ManagementFactory.getRuntimeMXBean().getStartTime(), platformHome());
    }

    /** PLATFORM_HOME is not always set as a system property; derive it from HYBRIS_BIN_DIR or catalina.home. */
    private static String platformHome() {
        String v = System.getProperty("PLATFORM_HOME");
        if (v != null && !v.isEmpty()) return v;
        String bin = System.getProperty("HYBRIS_BIN_DIR");
        if (bin != null && !bin.isEmpty()) return java.nio.file.Path.of(bin, "platform").toString();
        String home = System.getProperty("catalina.home");
        if (home != null && !home.isEmpty()) {
            java.nio.file.Path parent = java.nio.file.Path.of(home).getParent();
            if (parent != null) return parent.toString();
        }
        return "";
    }

    private static boolean enhanced() {
        for (String a : ManagementFactory.getRuntimeMXBean().getInputArguments()) {
            if (a.equals("-XX:+AllowEnhancedClassRedefinition")) return true;
        }
        return false;
    }

    // ---- inventory ----

    private List<LoaderInfo> inventory() {
        IdentityHashMap<ClassLoader, Long> ids = new IdentityHashMap<>();
        List<ClassLoader> order = new ArrayList<>();
        for (Class<?> c : inst.getAllLoadedClasses()) {
            for (ClassLoader cl = c.getClassLoader(); cl != null && !ids.containsKey(cl); cl = cl.getParent()) {
                ids.put(cl, (long) (order.size() + 1));
                order.add(cl);
            }
        }
        List<LoaderInfo> out = new ArrayList<>(order.size());
        for (ClassLoader cl : order) {
            ClassLoader parent = cl.getParent();
            Long parentId = parent == null ? Long.valueOf(0) : ids.getOrDefault(parent, 0L);
            out.add(new LoaderInfo(ids.get(cl), parentId, cl.getClass().getName(),
                    cl.getName() == null ? "" : cl.getName(), urlsOf(cl)));
        }
        return out;
    }

    private static List<String> urlsOf(ClassLoader cl) {
        List<String> urls = new ArrayList<>();
        if (cl instanceof URLClassLoader u) {
            for (URL url : u.getURLs()) urls.add(url.toString());
        } else if (cl == ClassLoader.getSystemClassLoader()) {
            String cp = System.getProperty("java.class.path", "");
            for (String e : cp.split(java.io.File.pathSeparator)) {
                if (!e.isEmpty()) urls.add(Path.of(e).toAbsolutePath().toUri().toString());
            }
        }
        return urls;
    }

    // ---- redefinition ----

    private synchronized BatchResult redefine(byte[] payload) throws IOException {
        long start = System.nanoTime();
        long batchId = Wire.redefineBatchId(payload);
        List<ClassEntry> entries = Wire.decodeRedefine(payload);

        Set<String> wanted = new HashSet<>();
        for (ClassEntry e : entries) wanted.add(e.name());
        long scanStart = System.nanoTime();
        Map<String, List<Class<?>>> loaded = new HashMap<>();
        for (Class<?> c : inst.getAllLoadedClasses()) {
            if (wanted.contains(c.getName()) && inst.isModifiableClass(c)) {
                loaded.computeIfAbsent(c.getName(), k -> new ArrayList<>()).add(c);
            }
        }

        long scanNanos = System.nanoTime() - scanStart;
        Map<String, Path> locationCache = new HashMap<>();
        Map<String, Path> scopeCache = new HashMap<>();
        Map<ClassEntry, List<ClassDefinition>> perEntry = new LinkedHashMap<>();
        for (ClassEntry e : entries) {
            Path scope = scopeCache.computeIfAbsent(e.scope(), AgentServer::realPath);
            List<ClassDefinition> defs = new ArrayList<>();
            for (Class<?> c : loaded.getOrDefault(e.name(), List.of())) {
                Path loc = codeSourcePath(c, locationCache);
                if (loc != null && scope != null && loc.startsWith(scope)) {
                    defs.add(new ClassDefinition(c, e.bytes()));
                }
            }
            perEntry.put(e, defs);
        }

        List<ClassDefinition> all = new ArrayList<>();
        perEntry.values().forEach(all::addAll);

        Map<ClassEntry, ClassResult> results = new LinkedHashMap<>();
        boolean fallback = false;
        long redefineStart = System.nanoTime();
        if (!all.isEmpty()) {
            try {
                inst.redefineClasses(all.toArray(new ClassDefinition[0]));
                perEntry.forEach((e, defs) -> {
                    if (!defs.isEmpty()) results.put(e, new ClassResult(e.name(), Wire.SWAPPED, "", defs.size()));
                });
            } catch (Throwable batchFailure) {
                // The JVM applies nothing from a rejected batch. Retry one class at a time so a single
                // unswappable class (for example one that gained a lambda) does not block the others.
                fallback = true;
                for (Map.Entry<ClassEntry, List<ClassDefinition>> en : perEntry.entrySet()) {
                    List<ClassDefinition> defs = en.getValue();
                    if (defs.isEmpty()) continue;
                    ClassEntry e = en.getKey();
                    try {
                        inst.redefineClasses(defs.toArray(new ClassDefinition[0]));
                        results.put(e, new ClassResult(e.name(), Wire.SWAPPED, "", defs.size()));
                    } catch (Throwable t) {
                        results.put(e, new ClassResult(e.name(), Wire.REJECTED, describe(t), defs.size()));
                    }
                }
            }
        }
        if (Boolean.getBoolean("hotdrop.debug")) {
            System.out.println("[HotDrop] timing: scanning " + inst.getAllLoadedClasses().length + " loaded classes "
                    + (scanNanos / 1_000_000) + " ms, redefine " + ((System.nanoTime() - redefineStart) / 1_000_000) + " ms");
        }
        if (spring.active()) {
            List<Class<?>> done = new ArrayList<>();
            results.forEach((e, r) -> {
                if (r.status() == Wire.SWAPPED) perEntry.get(e).forEach(d -> done.add(d.getDefinitionClass()));
            });
            if (!done.isEmpty()) {
                try {
                    spring.afterSwap(done);
                } catch (Throwable t) {
                    System.err.println("[HotDrop] Spring refresh failed: " + t);
                }
            }
        }
        List<ClassResult> list = new ArrayList<>();
        for (ClassEntry e : perEntry.keySet()) {
            list.add(results.getOrDefault(e, new ClassResult(e.name(), Wire.NOT_LOADED, "", 0)));
        }
        for (ClassResult r : list) {
            switch (r.status()) {
                case Wire.SWAPPED -> say("Reloaded class '" + r.name() + "'" + (r.copies() > 1 ? " (" + r.copies() + " class loaders)" : ""));
                case Wire.REJECTED -> say("Could not reload class '" + r.name() + "' - restart required: " + explain(r.message()));
                default -> say("Compiled class '" + r.name() + "' (not loaded yet, used when first needed)");
            }
        }
        return new BatchResult(batchId, fallback, System.nanoTime() - start, list);
    }

    /** Reload messages go to the server console, so they show up in the Hybris log next to everything else. */
    private void say(String msg) {
        if (!"true".equals(opts.get("quiet")) && !"false".equalsIgnoreCase(System.getProperty("hotdrop.log"))) {
            System.out.println("[HotDrop] " + msg);
        }
    }

    private static String explain(String jvmMessage) {
        String m = jvmMessage == null ? "" : jvmMessage;
        if (m.contains("add a method")) return "a method, constructor or lambda was added (" + m + ")";
        if (m.contains("delete a method")) return "a method was removed (" + m + ")";
        if (m.contains("schema change")) return "a field was added or removed (" + m + ")";
        if (m.contains("hierarchy change")) return "the superclass or interfaces changed (" + m + ")";
        return m;
    }

    private static String describe(Throwable t) {
        String m = t.getMessage();
        return t.getClass().getSimpleName() + (m == null ? "" : ": " + m);
    }

    private static Path codeSourcePath(Class<?> c, Map<String, Path> cache) {
        ProtectionDomain pd = c.getProtectionDomain();
        CodeSource cs = pd == null ? null : pd.getCodeSource();
        URL url = cs == null ? null : cs.getLocation();
        if (url == null || !"file".equals(url.getProtocol())) return null;
        return cache.computeIfAbsent(url.toString(), k -> {
            try {
                return realPath(Path.of(URI.create(k)).toString());
            } catch (Exception e) {
                return null;
            }
        });
    }

    private static Path realPath(String s) {
        try {
            return Path.of(s).toRealPath();
        } catch (IOException | RuntimeException e) {
            return Path.of(s).toAbsolutePath().normalize();
        }
    }

    private static String hex(byte[] b) {
        StringBuilder sb = new StringBuilder();
        for (byte x : b) sb.append(String.format("%02x", x));
        return sb.toString();
    }
}
