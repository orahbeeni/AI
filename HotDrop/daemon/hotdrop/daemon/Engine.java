package hotdrop.daemon;

import hotdrop.daemon.RootCompiler.Diag;
import hotdrop.daemon.RootCompiler.FileResult;
import hotdrop.daemon.RootCompiler.Output;
import hotdrop.protocol.Wire;
import hotdrop.protocol.Wire.AgentInfo;
import hotdrop.protocol.Wire.BatchResult;
import hotdrop.protocol.Wire.ClassEntry;
import hotdrop.protocol.Wire.ClassResult;
import hotdrop.protocol.Wire.LoaderInfo;

import java.io.IOException;
import java.net.URI;
import java.nio.file.FileVisitOption;
import java.nio.file.FileVisitResult;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.SimpleFileVisitor;
import java.nio.file.StandardCopyOption;
import java.nio.file.attribute.BasicFileAttributes;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Collection;
import java.util.Collections;
import java.util.Comparator;
import java.util.Deque;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.stream.Stream;

/**
 * The scheduler. Holds the state of every watched source file and runs compile cycles.
 *
 * <pre>
 * CLEAN --edit--> compile ok ---------------> written + swapped --> CLEAN
 *                 compile errors ----------> BROKEN   (retried on every later cycle)
 *                 uses a BROKEN type ------> HELD     (retried on every later cycle)
 *                 JVM refused the swap ----> RESTART  (until the file changes again)
 * </pre>
 * Only error-free, non-held units are ever written to disk or sent to the server.
 */
final class Engine implements AutoCloseable {
    enum State { CLEAN, BROKEN, HELD, RESTART }

    static final class Unit {
        final Path file;
        Root root;
        State state = State.CLEAN;
        Set<String> declared = Set.of();
        Set<String> refs = Set.of();
        String abi;
        List<Diag> errors = List.of();
        String note = "";

        Unit(Path file, Root root) {
            this.file = file;
            this.root = root;
        }
    }

    static final class Report {
        final List<String> swapped = new ArrayList<>();
        final List<String> notLoaded = new ArrayList<>();
        final List<String> rejected = new ArrayList<>();
        final Map<Path, List<Diag>> broken = new LinkedHashMap<>();
        final Map<Path, String> held = new LinkedHashMap<>();
        final List<Path> deleted = new ArrayList<>();
        int compiled;
        int rounds;
        long compileNanos;
        long sendNanos;
        long writeNanos;
        long agentNanos;
        long totalNanos;
        boolean undeliveredNoAgent;
        boolean empty = true;
    }

    private final Map<Path, String> notifiedProblems = new HashMap<>();

    /** Tells the server's console about files that did not go live, once per change, so the Hybris log shows why. */
    synchronized void notifyServer(Report rep) {
        if (rep.empty) return;
        notifiedProblems.keySet().removeIf(p -> !rep.broken.containsKey(p) && !rep.held.containsKey(p));
        for (var e : rep.broken.entrySet()) {
            Diag first = e.getValue().isEmpty() ? null : e.getValue().get(0);
            String msg = "Not reloaded: " + e.getKey().getFileName() + " does not compile (" + e.getValue().size() + " error(s))"
                    + (first == null ? "" : ", first: " + first) + " - kept pending until it compiles";
            if (!msg.equals(notifiedProblems.put(e.getKey(), msg))) agent.notice(msg);
        }
        for (var e : rep.held.entrySet()) {
            String msg = "Held back: " + e.getKey().getFileName() + " - " + e.getValue();
            if (!msg.equals(notifiedProblems.put(e.getKey(), msg))) agent.notice(msg);
        }
    }

    private record Undelivered(String name, String scope, byte[] bytes, long writtenMillis) {}

    private final Config cfg;
    final List<Root> roots;
    private final AgentLink agent;
    private final List<String> javacOptions;
    private final List<Path> sourcepath = new ArrayList<>();

    private final Map<Path, Unit> units = new HashMap<>();
    private final Map<String, String> lastSent = new HashMap<>();
    private final Map<String, Path> classOwner = new HashMap<>();
    private final Map<String, Undelivered> undelivered = new LinkedHashMap<>();
    private final Map<Root, RootCompiler> compilers = new HashMap<>();
    private final Map<Root, List<Path>> classpathCache = new HashMap<>();
    private List<Path> fallbackClasspath;
    private int seenGeneration = -1;
    private long batchCounter;
    private final Deque<Long> recentTotals = new ArrayDeque<>();
    private int indexedRoots;
    private final RootCompiler.Shared sharedFiles;
    /** Null when no Spring directories are configured. */
    final SpringSync spring;
    private long springServerId;
    private final Map<Path, Integer> modelHashes;

    Engine(Config cfg, List<Root> roots, AgentLink agent, List<String> javacOptions) throws IOException {
        this.cfg = cfg;
        this.roots = new ArrayList<>(roots);
        this.roots.sort(Comparator.comparing((Root r) -> r.web));
        this.agent = agent;
        this.javacOptions = javacOptions;
        for (Root r : this.roots) sourcepath.add(r.src);
        this.sharedFiles = new RootCompiler.Shared(sourcepath);
        this.spring = cfg.springDirs.isEmpty() ? null : new SpringSync(cfg.springDirs, agent);
        this.modelHashes = new HashMap<>(Resources.modelHashes(cfg.springDirs));
    }

    /** Spring XML, model XML, Backoffice config, message bundles and ImpEx files that changed since the last cycle. */
    synchronized void runResources(Set<Path> changed) {
        if (changed.isEmpty()) return;
        Set<Path> springFiles = new LinkedHashSet<>();
        boolean messages = false;
        for (Path p : changed) {
            switch (Resources.kind(p)) {
                case SPRING -> springFiles.add(p);
                case MODEL, BACKOFFICE -> modelChanged(p);
                case IMPEX -> impexSaved(p);
                case MESSAGES -> messages = true;
                default -> { }
            }
        }
        if (spring != null && !springFiles.isEmpty()) spring.apply(springFiles);
        if (messages) {
            agent.clearMessages();
            Log.info("[ok] message bundle changed: MessageSource caches cleared");
        }
    }

    private void modelChanged(Path p) {
        Integer now = Resources.hash(p);
        Integer was = modelHashes.put(p, now);
        if (now == null || now.equals(was)) return;
        String name = p.getFileName().toString();
        if (Resources.kind(p) == Resources.Kind.BACKOFFICE) {
            String what = "Backoffice keeps its configuration cached: reload it from Backoffice or restart (widget classes themselves are swapped like any other class)";
            Log.warn("[note] %s: %s", name, what);
            agent.notice(name + " changed - " + what);
            return;
        }
        String what = name.endsWith("-items.xml")
                ? "the type system changed: run 'ant build', restart, and update the running system (HAC > Platform > Update)"
                : "generated DTO / event classes are stale: run 'ant build' and restart";
        Log.warn("[restart required] %s: %s", name, what);
        agent.notice(name + " changed - " + what);
    }

    private final Set<Path> impexHinted = new HashSet<>();
    private ImpexRunner impex;
    private java.util.concurrent.ExecutorService impexThread;

    /** Opt-in (--impex) and per file ('# hotdrop-on-save'); runs on its own thread so a slow import never delays a swap. */
    private void impexSaved(Path p) {
        if (!cfg.impex) return;
        String name = p.getFileName().toString();
        if (!ImpexRunner.hasMarker(p)) {
            if (impexHinted.add(p)) Log.info("[impex] %s saved but not run: add a line '# hotdrop-on-save' at its top to run it on save", name);
            return;
        }
        if (impexThread == null) {
            impexThread = java.util.concurrent.Executors.newSingleThreadExecutor(r -> {
                Thread t = new Thread(r, "hotdrop-impex");
                t.setDaemon(true);
                return t;
            });
        }
        impexThread.execute(() -> {
            long t0 = System.nanoTime();
            try {
                if (impex == null) impex = new ImpexRunner(cfg.hacUrl, cfg.hacUser, cfg.hacPassword);
                String result = impex.run(p);
                Log.info("[impex] %s imported in %s: %s", name, Log.ms(System.nanoTime() - t0), result);
                agent.notice("ImpEx " + name + " imported: " + result);
            } catch (Exception e) {
                Log.warn("[impex failed] %s: %s", name, e.getMessage());
                agent.notice("ImpEx " + name + " failed: " + e.getMessage());
            }
        });
    }

    // ---- agent lifecycle ----

    /** Called periodically: notices a new or restarted server and brings it up to date. */
    synchronized void maintenance() {
        boolean up = agent.poll();
        if (!up) {
            seenGeneration = -1;
            return;
        }
        if (agent.generation() != seenGeneration) {
            seenGeneration = agent.generation();
            resetCompilers();
            classpathCache.clear();
            flushUndelivered();
            // a reconnect to the same server keeps what is still pending; a restarted server loaded the files from disk
            AgentInfo now = agent.info();
            long id = now == null ? 0 : now.pid() * 31 + now.startMillis();
            if (spring != null && id != springServerId) spring.rebaseline();
            springServerId = id;
        }
    }

    private void flushUndelivered() {
        AgentInfo info = agent.info();
        if (info == null || undelivered.isEmpty()) return;
        List<ClassEntry> batch = new ArrayList<>();
        for (Undelivered u : undelivered.values()) {
            // a class written before this JVM started was loaded from disk already
            if (u.writtenMillis() >= info.startMillis()) batch.add(new ClassEntry(u.name(), u.scope(), u.bytes()));
        }
        undelivered.clear();
        if (batch.isEmpty()) return;
        BatchResult br = agent.redefine(++batchCounter, batch);
        if (br == null) {
            for (ClassEntry e : batch) undelivered.put(e.name(), new Undelivered(e.name(), e.scope(), e.bytes(), System.currentTimeMillis()));
            return;
        }
        int ok = 0;
        for (ClassResult r : br.results()) if (r.status() == Wire.SWAPPED) ok++;
        Log.info("delivered %d queued class(es) to the newly connected server", ok);
    }

    // ---- indexing ----

    /** Compiles every source of a root without output, to learn the dependency graph and API baseline. */
    synchronized void indexRoot(Root r) {
        long t0 = System.nanoTime();
        List<Path> files = sourcesOf(r);
        if (files.isEmpty()) {
            indexedRoots++;
            return;
        }
        try {
            Output out = compiler(r).compile(files, false);
            int errs = 0;
            for (FileResult fr : out.files.values()) {
                Unit u = units.computeIfAbsent(fr.file, f -> new Unit(f, r));
                if (!fr.errors.isEmpty()) errs++;
                if (fr.analyzed) {
                    u.declared = Set.copyOf(fr.declared);
                    u.refs = Set.copyOf(fr.refs);
                    u.abi = fr.abi();
                }
            }
            indexedRoots++;
            Log.info("indexed %s: %d files in %s%s", r.name, files.size(), Log.ms(System.nanoTime() - t0),
                    errs > 0 ? " (" + errs + " with errors, ignored)" : "");
        } catch (IOException | RuntimeException e) {
            indexedRoots++;
            Log.warn("indexing %s failed: %s", r.name, e);
        }
    }

    private List<Path> sourcesOf(Root r) {
        List<Path> out = new ArrayList<>();
        try (Stream<Path> s = Files.walk(r.src)) {
            s.filter(p -> p.toString().endsWith(".java") && Files.isRegularFile(p)).forEach(out::add);
        } catch (IOException e) {
            Log.warn("cannot list %s: %s", r.src, e.getMessage());
        }
        return out;
    }

    // ---- the cycle ----

    synchronized Report runCycle(Set<Path> changed, Set<Path> deleted) {
        long t0 = System.nanoTime();
        Report rep = new Report();

        Set<String> rippleTypes = new HashSet<>();
        for (Path d : deleted) {
            Unit u = units.remove(d);
            if (u != null) {
                rippleTypes.addAll(u.declared);
                rep.deleted.add(d);
                rep.empty = false;
            }
        }
        Set<Path> cand = new LinkedHashSet<>();
        for (Path c : changed) {
            if (Files.isRegularFile(c) && rootOf(c) != null) cand.add(c);
        }
        for (Unit u : units.values()) {
            if ((u.state == State.BROKEN || u.state == State.HELD) && Files.isRegularFile(u.file)) cand.add(u.file);
        }
        cand.addAll(dependentsOf(rippleTypes, cand));
        if (cand.isEmpty()) return rep;
        rep.empty = false;

        // 1. compile, widening the set while the API of a changed class affects other files
        Output out;
        int rounds = 0;
        long compileNanos = 0;
        while (true) {
            rounds++;
            out = compile(cand, true);
            compileNanos += out.nanos;
            for (FileResult fr : out.files.values()) {
                Unit u = units.get(fr.file);
                if (u != null && u.abi != null && fr.analyzed && fr.errors.isEmpty() && !u.abi.equals(fr.abi())) {
                    rippleTypes.addAll(u.declared);
                    rippleTypes.addAll(fr.declared);
                }
            }
            Set<Path> grow = new LinkedHashSet<>(dependentsOf(rippleTypes, cand));
            for (Path foreign : out.foreignErrors.keySet()) {
                if (rootOf(foreign) != null && !cand.contains(foreign)) grow.add(foreign);
            }
            if (grow.isEmpty() || rounds >= 6) break;
            cand.addAll(grow);
        }
        rep.rounds = rounds;
        rep.compiled = cand.size();

        // 2. classify
        Map<Path, FileResult> latest = new LinkedHashMap<>(out.files);
        Set<Path> broken = new LinkedHashSet<>();
        for (FileResult fr : latest.values()) if (!fr.errors.isEmpty()) broken.add(fr.file);
        Map<Path, String> held = new LinkedHashMap<>();
        Map<Path, Map<String, byte[]>> readyClasses = new LinkedHashMap<>();

        if (!out.anyErrors) {
            for (FileResult fr : latest.values()) readyClasses.put(fr.file, fr.classes);
        } else {
            if (broken.isEmpty()) {
                // errors javac could not attribute to a file we know about: block everything this cycle
                String why = out.unattributed.isEmpty() ? "compiler reported errors outside the edited files" : out.unattributed.get(0);
                for (Path p : cand) held.put(p, why);
            } else {
                for (int attempt = 0; attempt < 4; attempt++) {
                    Set<String> bad = new HashSet<>();
                    for (Path b : broken) bad.addAll(declaredOf(latest.get(b), b));
                    boolean grew = true;
                    while (grew) {
                        grew = false;
                        for (FileResult fr : latest.values()) {
                            if (broken.contains(fr.file) || held.containsKey(fr.file)) continue;
                            String hit = firstIntersection(fr.refs, bad);
                            if (hit != null) {
                                held.put(fr.file, "waiting on " + hit);
                                bad.addAll(fr.declared);
                                grew = true;
                            }
                        }
                    }
                    List<Path> ready = new ArrayList<>();
                    for (Path p : cand) if (!broken.contains(p) && !held.containsKey(p)) ready.add(p);
                    readyClasses.clear();
                    if (ready.isEmpty()) break;
                    // javac skips code generation once anything fails, so compile just the sound files again
                    Output second = compile(ready, true);
                    compileNanos += second.nanos;
                    Set<Path> newlyBroken = new LinkedHashSet<>();
                    for (FileResult fr : second.files.values()) if (!fr.errors.isEmpty()) newlyBroken.add(fr.file);
                    if (newlyBroken.isEmpty() && !second.anyErrors) {
                        for (FileResult fr : second.files.values()) readyClasses.put(fr.file, fr.classes);
                        break;
                    }
                    if (newlyBroken.isEmpty()) {
                        String why = second.unattributed.isEmpty() ? "compiler blocked by errors elsewhere" : second.unattributed.get(0);
                        for (Path p : ready) held.put(p, why);
                        break;
                    }
                    for (Path nb : newlyBroken) {
                        latest.put(nb, second.files.get(nb));
                        broken.add(nb);
                    }
                    held.clear();
                }
            }
        }
        rep.compileNanos = compileNanos;

        // 3. commit the graph and states
        for (FileResult fr : latest.values()) {
            Unit u = units.computeIfAbsent(fr.file, f -> new Unit(f, rootOf(f)));
            if (u.root == null) u.root = rootOf(fr.file);
            if (fr.errors.isEmpty() && fr.analyzed) {
                u.declared = Set.copyOf(fr.declared);
                u.refs = Set.copyOf(fr.refs);
                u.abi = fr.abi();
            }
            if (broken.contains(fr.file)) {
                u.state = State.BROKEN;
                u.errors = List.copyOf(fr.errors);
                u.note = "";
                rep.broken.put(fr.file, u.errors);
            } else if (held.containsKey(fr.file)) {
                u.state = State.HELD;
                u.errors = List.of();
                u.note = held.get(fr.file);
                rep.held.put(fr.file, u.note);
            } else {
                u.state = State.CLEAN;
                u.errors = List.of();
                u.note = "";
            }
        }

        // 4. write and deliver the sound classes
        long sendStart = System.nanoTime();
        List<ClassEntry> batch = new ArrayList<>();
        Map<String, Path> batchOwner = new HashMap<>();
        for (Map.Entry<Path, Map<String, byte[]>> e : readyClasses.entrySet()) {
            Root r = rootOf(e.getKey());
            if (r == null) continue;
            for (Map.Entry<String, byte[]> c : e.getValue().entrySet()) {
                String sha = Abi.sha1(c.getValue());
                if (sha.equals(lastSent.get(c.getKey()))) continue;
                try {
                    long w0 = System.nanoTime();
                    writeClass(r, c.getKey(), c.getValue());
                    rep.writeNanos += System.nanoTime() - w0;
                } catch (IOException ex) {
                    Log.warn("cannot write class %s: %s", c.getKey(), ex.getMessage());
                    continue;
                }
                lastSent.put(c.getKey(), sha);
                classOwner.put(c.getKey(), e.getKey());
                batch.add(new ClassEntry(c.getKey(), r.scope.toString(), c.getValue()));
                batchOwner.put(c.getKey(), e.getKey());
            }
        }
        if (!batch.isEmpty()) {
            BatchResult br = agent.redefine(++batchCounter, batch);
            if (br == null) {
                rep.undeliveredNoAgent = true;
                long now = System.currentTimeMillis();
                for (ClassEntry ce : batch) {
                    undelivered.put(ce.name(), new Undelivered(ce.name(), ce.scope(), ce.bytes(), now));
                    rep.swapped.add(ce.name());
                }
            } else {
                rep.agentNanos = br.nanos();
                for (ClassResult cr : br.results()) {
                    switch (cr.status()) {
                        case Wire.SWAPPED -> rep.swapped.add(cr.name());
                        case Wire.NOT_LOADED -> rep.notLoaded.add(cr.name());
                        default -> {
                            rep.rejected.add(cr.name() + " - " + cr.message());
                            Unit u = units.get(batchOwner.get(cr.name()));
                            if (u != null) {
                                u.state = State.RESTART;
                                u.note = cr.name() + ": " + cr.message();
                            }
                        }
                    }
                }
            }
        }
        rep.sendNanos = System.nanoTime() - sendStart;
        rep.totalNanos = System.nanoTime() - t0;
        recentTotals.addLast(rep.totalNanos);
        if (recentTotals.size() > 200) recentTotals.removeFirst();
        return rep;
    }

    private static Set<String> declaredOf(FileResult fr, Path p) {
        return fr == null ? Set.of() : fr.declared;
    }

    private static String firstIntersection(Set<String> a, Set<String> b) {
        for (String x : a) if (b.contains(x)) return x;
        return null;
    }

    private Collection<Path> dependentsOf(Set<String> types, Set<Path> exclude) {
        if (types.isEmpty()) return List.of();
        List<Path> out = new ArrayList<>();
        for (Unit u : units.values()) {
            if (exclude.contains(u.file) || !Files.isRegularFile(u.file)) continue;
            if (firstIntersection(u.refs, types) != null) out.add(u.file);
        }
        return out;
    }

    Root rootOf(Path file) {
        Root r = rootOfExact(file);
        if (r != null) return r;
        // the file may have been reached through a symlink (macOS /var -> /private/var, CCv2 checkouts)
        Path real = real(file);
        for (Root c : roots) {
            if (real.startsWith(real(c.src))) return c;
        }
        return null;
    }

    private Root rootOfExact(Path file) {
        Root best = null;
        for (Root r : roots) {
            if (file.startsWith(r.src) && (best == null || r.src.getNameCount() > best.src.getNameCount())) best = r;
        }
        return best;
    }

    private Output compile(Collection<Path> files, boolean generate) {
        Map<Root, List<Path>> byRoot = new LinkedHashMap<>();
        for (Root r : roots) byRoot.put(r, new ArrayList<>());
        for (Path p : files) {
            Root r = rootOf(p);
            if (r != null) byRoot.get(r).add(p);
        }
        Output merged = new Output();
        for (Map.Entry<Root, List<Path>> e : byRoot.entrySet()) {
            if (e.getValue().isEmpty()) continue;
            try {
                Output o = compiler(e.getKey()).compile(e.getValue(), generate);
                merged.files.putAll(o.files);
                o.foreignErrors.forEach((k, v) -> merged.foreignErrors.computeIfAbsent(k, x -> new ArrayList<>()).addAll(v));
                merged.unattributed.addAll(o.unattributed);
                merged.anyErrors |= o.anyErrors;
                merged.nanos += o.nanos;
            } catch (IOException | RuntimeException ex) {
                merged.unattributed.add("cannot set up compiler for " + e.getKey() + ": " + ex);
                merged.anyErrors = true;
                for (Path p : e.getValue()) merged.files.put(p, new FileResult(p));
            }
        }
        return merged;
    }

    // ---- compiler and classpath ----

    private RootCompiler compiler(Root r) throws IOException {
        List<Path> cp = classpathFor(r);
        RootCompiler c = compilers.get(r);
        if (c == null || !c.classpath.equals(cp)) {
            long t0 = System.nanoTime();
            c = new RootCompiler(sharedFiles, cp, javacOptions);
            compilers.put(r, c);
            Log.debug("compiler for %s created (%d classpath entries) in %s", r.name, cp.size(), Log.ms(System.nanoTime() - t0));
        }
        return c;
    }

    private List<Path> classpathFor(Root r) {
        List<Path> cached = classpathCache.get(r);
        if (cached != null) return cached;
        List<Path> cp = fromInventory(r);
        boolean fromServer = cp != null;
        if (cp == null) {
            cp = new ArrayList<>(fallback());
        }
        if (!cp.contains(r.out)) cp.add(0, r.out);
        // A server that is still starting has no loader for this root yet: use the scan now, ask the server again next time.
        if (fromServer || !agent.connected()) classpathCache.put(r, cp);
        else agent.refreshInventory();
        Log.debug("classpath for %s: %d entries (%s)", r.name, cp.size(), agent.connected() ? "from server" : "scanned");
        return cp;
    }

    private List<Path> fromInventory(Root r) {
        List<LoaderInfo> inv = agent.inventory();
        if (inv == null) return null;
        Path out = real(r.out);
        Map<Long, LoaderInfo> byId = new HashMap<>();
        for (LoaderInfo l : inv) byId.put(l.id(), l);
        LoaderInfo owner = null;
        for (LoaderInfo l : inv) {
            for (String u : l.urls()) {
                Path p = urlPath(u);
                if (p != null && real(p).equals(out)) {
                    owner = l;
                    break;
                }
            }
            if (owner != null) break;
        }
        if (owner == null) return null;
        Deque<LoaderInfo> chain = new ArrayDeque<>();
        for (LoaderInfo l = owner; l != null; l = byId.get(l.parentId())) chain.addFirst(l);
        LinkedHashSet<Path> cp = new LinkedHashSet<>();
        for (LoaderInfo l : chain) {
            for (String u : l.urls()) {
                Path p = urlPath(u);
                if (p != null && Files.exists(p)) cp.add(p);
            }
        }
        return new ArrayList<>(cp);
    }

    private static Path urlPath(String url) {
        try {
            URI uri = URI.create(url);
            return "file".equals(uri.getScheme()) ? Path.of(uri) : null;
        } catch (RuntimeException e) {
            return null;
        }
    }

    private static Path real(Path p) {
        try {
            return p.toRealPath();
        } catch (IOException e) {
            return p.toAbsolutePath().normalize();
        }
    }

    /** Used when no server is running: every jar under hybris/bin plus the extensions' class directories. */
    private List<Path> fallback() {
        if (fallbackClasspath != null) return fallbackClasspath;
        LinkedHashSet<Path> cp = new LinkedHashSet<>(cfg.extraClasspath);
        if (cfg.hybris != null) {
            Path bin = cfg.hybris.resolve("bin");
            Set<String> skip = Set.of("node_modules", ".git", "src", "testsrc", "gensrc");
            try {
                Files.walkFileTree(bin, Set.of(FileVisitOption.FOLLOW_LINKS), Integer.MAX_VALUE, new SimpleFileVisitor<>() {
                    @Override
                    public FileVisitResult preVisitDirectory(Path dir, BasicFileAttributes a) {
                        String n = dir.getFileName() == null ? "" : dir.getFileName().toString();
                        if (skip.contains(n)) return FileVisitResult.SKIP_SUBTREE;
                        if (n.equals("classes") && (Files.isRegularFile(dir.getParent().resolve("extensioninfo.xml"))
                                || dir.endsWith(Path.of("WEB-INF", "classes")))) cp.add(dir);
                        return FileVisitResult.CONTINUE;
                    }

                    @Override
                    public FileVisitResult visitFile(Path f, BasicFileAttributes a) {
                        String n = f.getFileName().toString();
                        if (n.endsWith(".jar") && !n.endsWith("-sources.jar")) cp.add(f);
                        return FileVisitResult.CONTINUE;
                    }
                });
            } catch (IOException e) {
                Log.warn("classpath scan failed: %s", e.getMessage());
            }
        }
        fallbackClasspath = new ArrayList<>(cp);
        Log.info("no server connected: compiling against %d scanned classpath entries", fallbackClasspath.size());
        return fallbackClasspath;
    }

    // ---- disk ----

    private void writeClass(Root r, String binaryName, byte[] bytes) throws IOException {
        Path target = r.out.resolve(binaryName.replace('.', '/') + ".class");
        boolean isNew = !Files.exists(target);
        Files.createDirectories(target.getParent());
        Path tmp = target.resolveSibling(target.getFileName() + ".hotdrop.tmp");
        Files.write(tmp, bytes);
        // Windows refuses to replace a file another process has open for a moment (antivirus, indexer): retry briefly.
        IOException last = null;
        for (int attempt = 0; attempt < 10; attempt++) {
            try {
                Files.move(tmp, target, StandardCopyOption.REPLACE_EXISTING);
                last = null;
                break;
            } catch (IOException e) {
                last = e;
                try {
                    Thread.sleep(20L * (attempt + 1));
                } catch (InterruptedException ie) {
                    Thread.currentThread().interrupt();
                    break;
                }
            }
        }
        if (last != null) {
            Files.deleteIfExists(tmp);
            throw last;
        }
        if (isNew) {
            int dot = binaryName.lastIndexOf('.');
            String pkg = dot < 0 ? "" : binaryName.substring(0, dot);
            sharedFiles.invalidatePackage(pkg);
        }
    }

    // ---- status ----

    private void resetCompilers() {
        compilers.clear();
        try {
            sharedFiles.reset();
        } catch (IOException e) {
            Log.warn("could not reset the compiler caches: %s", e.getMessage());
        }
    }

    synchronized void forgetClasspaths() {
        resetCompilers();
        classpathCache.clear();
        fallbackClasspath = null;
        agent.refreshInventory();
    }

    synchronized String status() {
        int clean = 0, broken = 0, held = 0, restart = 0;
        StringBuilder detail = new StringBuilder();
        for (Unit u : units.values()) {
            switch (u.state) {
                case CLEAN -> clean++;
                case BROKEN -> {
                    broken++;
                    detail.append("  BROKEN   ").append(u.file).append('\n');
                    for (Diag d : u.errors.subList(0, Math.min(3, u.errors.size()))) detail.append("             ").append(d).append('\n');
                }
                case HELD -> {
                    held++;
                    detail.append("  HELD     ").append(u.file).append("  (").append(u.note).append(")\n");
                }
                case RESTART -> {
                    restart++;
                    detail.append("  RESTART  ").append(u.file).append("  (").append(u.note).append(")\n");
                }
            }
        }
        StringBuilder sb = new StringBuilder();
        AgentInfo info = agent.info();
        sb.append("server:   ").append(info == null ? "not connected"
                : "pid " + info.pid() + ", " + info.vendor() + " " + info.javaVersion()
                + ", enhanced redefinition " + (info.enhancedRedefine() ? "ON" : "off")).append('\n');
        if (spring != null) sb.append("spring:   ").append(spring.files()).append(" XML file(s) watched\n");
        sb.append("roots:    ").append(roots.size()).append(" (indexed ").append(indexedRoots).append(")\n");
        sb.append("files:    ").append(units.size()).append(" known, ").append(clean).append(" clean, ")
                .append(broken).append(" broken, ").append(held).append(" held, ").append(restart).append(" need restart\n");
        sb.append("queued:   ").append(undelivered.size()).append(" class(es) waiting for a server\n");
        if (!recentTotals.isEmpty()) {
            List<Long> sorted = new ArrayList<>(recentTotals);
            Collections.sort(sorted);
            sb.append("cycles:   ").append(sorted.size()).append(", p50 ").append(Log.ms(sorted.get(sorted.size() / 2)))
                    .append(", p95 ").append(Log.ms(sorted.get(Math.min(sorted.size() - 1, (int) (sorted.size() * 0.95))))).append('\n');
        }
        sb.append(detail);
        return sb.toString();
    }

    @Override
    public synchronized void close() {
        if (impexThread != null) impexThread.shutdownNow();
        agent.close();
    }
}
