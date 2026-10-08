package hotdrop.daemon;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Deque;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Set;
import java.util.concurrent.BlockingQueue;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.TimeUnit;

/** File events in, compile cycles out. One worker thread, so cycles never overlap. */
final class Daemon implements AutoCloseable {
    private record FsEvent(Path path, boolean flush) {}

    private final Config cfg;
    final Engine engine;
    private final BlockingQueue<FsEvent> queue = new LinkedBlockingQueue<>();
    private final Set<Path> pendingChanged = new LinkedHashSet<>();
    private final Set<Path> pendingDeleted = new LinkedHashSet<>();
    private volatile boolean paused;
    private volatile boolean running = true;
    private volatile boolean reindex;
    private Thread worker;
    private final List<java.io.Closeable> watchers = new ArrayList<>();

    Daemon(Config cfg, Engine engine) {
        this.cfg = cfg;
        this.engine = engine;
    }

    void start() throws IOException {
        List<Path> srcs = new ArrayList<>();
        for (Root r : engine.roots) srcs.add(r.src);
        startWatchers(srcs);
        worker = new Thread(this::loop, "hotdrop-worker");
        worker.start();
    }

    private static boolean isMac() {
        return System.getProperty("os.name", "").toLowerCase().contains("mac");
    }

    private void startWatchers(List<Path> srcs) throws IOException {
        boolean poll = cfg.watch.equals("poll") || (cfg.watch.equals("auto") && isMac());
        if (!poll) {
            SourceWatcher w = new SourceWatcher(srcs, p -> submit(p, false));
            watchers.add(w);
            if (w.failed() > 0) {
                Log.warn("%d director(ies) could not be watched natively; adding a polling watcher as well", w.failed());
                poll = true;
            }
        }
        if (poll) {
            watchers.add(new PollingWatcher(srcs, p -> submit(p, false), cfg.pollMs));
            Log.info("watching by polling every %dms (%s)", cfg.pollMs,
                    cfg.watch.equals("auto") ? "the JDK's native watcher is slow on macOS" : "--watch " + cfg.watch);
        }
    }

    void join() throws InterruptedException {
        worker.join();
    }

    void submit(Path file, boolean flush) {
        queue.add(new FsEvent(file == null ? null : file.toAbsolutePath().normalize(), flush));
    }

    void pause(boolean p) {
        paused = p;
        Log.info(p ? "paused: changes are collected, not compiled" : "resumed");
        if (!p) submit(null, true);
    }

    void rescan() {
        reindex = true;
        submit(null, true);
    }

    void stop() {
        running = false;
        queue.add(new FsEvent(null, true));
    }

    private void loop() {
        Deque<Root> toIndex = new ArrayDeque<>();
        if (cfg.index) toIndex.addAll(engine.roots);
        long lastMaintenance = 0;
        engine.maintenance();
        try {
            while (running) {
                if (reindex) {
                    reindex = false;
                    engine.forgetClasspaths();
                    toIndex.clear();
                    if (cfg.index) toIndex.addAll(engine.roots);
                }
                FsEvent ev = queue.poll(toIndex.isEmpty() ? 1000 : 0, TimeUnit.MILLISECONDS);
                if (ev == null) {
                    if (!toIndex.isEmpty()) {
                        engine.indexRoot(toIndex.poll());
                    } else if (System.currentTimeMillis() - lastMaintenance > 2000) {
                        engine.maintenance();
                        lastMaintenance = System.currentTimeMillis();
                    }
                    continue;
                }
                boolean flush = accumulate(ev);
                while (!flush && running) {
                    FsEvent next = queue.poll(cfg.debounceMs, TimeUnit.MILLISECONDS);
                    if (next == null) break;
                    flush |= accumulate(next);
                }
                if (!running || paused) continue;
                cycle();
                lastMaintenance = System.currentTimeMillis();
            }
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        }
        close();
    }

    private boolean accumulate(FsEvent ev) {
        if (ev.path() != null) {
            if (Files.exists(ev.path())) {
                pendingChanged.add(ev.path());
                pendingDeleted.remove(ev.path());
            } else {
                pendingDeleted.add(ev.path());
                pendingChanged.remove(ev.path());
            }
        }
        return ev.flush();
    }

    private void cycle() {
        Set<Path> changed = new LinkedHashSet<>(pendingChanged);
        Set<Path> deleted = new LinkedHashSet<>(pendingDeleted);
        pendingChanged.clear();
        pendingDeleted.clear();
        try {
            engine.maintenance();
            Engine.Report rep = engine.runCycle(changed, deleted);
            logReport(rep);
            engine.notifyServer(rep);
        } catch (Throwable t) {
            Log.warn("cycle failed: %s", t);
            t.printStackTrace();
        }
    }

    static void logReport(Engine.Report rep) {
        if (rep.empty) return;
        String timing = "compile " + Log.ms(rep.compileNanos) + ", send " + Log.ms(rep.sendNanos)
                + (rep.agentNanos > 0 ? " (write " + Log.ms(rep.writeNanos) + ", server " + Log.ms(rep.agentNanos) + ")" : "")
                + ", total " + Log.ms(rep.totalNanos) + (rep.rounds > 1 ? ", " + rep.rounds + " rounds" : "");
        if (!rep.swapped.isEmpty() || !rep.notLoaded.isEmpty()) {
            String what = rep.swapped.isEmpty() ? "" : "swapped " + simple(rep.swapped);
            if (!rep.notLoaded.isEmpty()) {
                what += (what.isEmpty() ? "" : "; ") + "written (not loaded yet) " + simple(rep.notLoaded);
            }
            Log.info("[ok] %s  (%s)%s", what, timing, rep.undeliveredNoAgent ? "  - no server connected, queued" : "");
        } else if (rep.broken.isEmpty() && rep.held.isEmpty() && rep.rejected.isEmpty()) {
            Log.info("[ok] nothing to swap, class bytes unchanged  (%s)", timing);
        }
        for (String r : rep.rejected) {
            Log.warn("[restart required] %s", r);
        }
        for (var e : rep.broken.entrySet()) {
            Log.warn("[broken] %s - %d error(s), kept pending until it compiles", e.getKey().getFileName(), e.getValue().size());
            int shown = 0;
            for (RootCompiler.Diag d : e.getValue()) {
                if (shown++ >= 5) {
                    System.out.println("      ... " + (e.getValue().size() - 5) + " more");
                    break;
                }
                System.out.println("      " + d);
            }
        }
        for (var e : rep.held.entrySet()) {
            Log.warn("[held] %s - %s", e.getKey().getFileName(), e.getValue());
        }
        if (!rep.broken.isEmpty() || !rep.held.isEmpty()) {
            Log.info("      (%s)", timing);
        }
    }

    private static String simple(List<String> names) {
        List<String> out = new ArrayList<>();
        for (String n : names) out.add(n.substring(n.lastIndexOf('.') + 1));
        return String.join(", ", out);
    }

    @Override
    public void close() {
        running = false;
        for (java.io.Closeable w : watchers) {
            try {
                w.close();
            } catch (IOException ignored) {
                // shutting down
            }
        }
        engine.close();
    }
}
