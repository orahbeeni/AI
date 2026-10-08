package hotdrop.daemon;

import java.io.Closeable;
import java.io.IOException;
import java.nio.file.DirectoryStream;
import java.nio.file.FileVisitResult;
import java.nio.file.Files;
import java.nio.file.NoSuchFileException;
import java.nio.file.Path;
import java.nio.file.SimpleFileVisitor;
import java.nio.file.attribute.BasicFileAttributes;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.Iterator;
import java.util.LinkedHashSet;
import java.util.Set;
import java.util.List;
import java.util.Map;
import java.util.function.Consumer;

/**
 * Stat-based watcher for platforms where the JDK WatchService is slow (macOS polls every few seconds) or
 * when native watches cannot be registered. Compares modification time and size of .java files.
 * <p>
 * Cost matters: on a real project (2,000 files in 1,000 directories) a full walk every 100 ms used 20% of a core, and
 * statting every known file and directory each tick still used 17%. So each tick it stats:
 * <ul>
 *   <li>every file that changed in the last {@link #HOT_MS} (the files being worked on) and their directories;</li>
 *   <li>a rotating tenth of all other files and directories, so a save to an untouched file is seen within ten ticks
 *       (a directory's mtime changes when a file is created, deleted or renamed there, which also covers IntelliJ's
 *       "safe write").</li>
 * </ul>
 * Measured on that project: 8.6% of a core when every directory was checked on every tick; the time is the kernel's
 * stat calls, which are much slower 100 ms apart than back to back. The trade-off is up to one second to notice the
 * first save of a file nobody touched for {@link #HOT_MS}; after that it is one tick.
 */
final class PollingWatcher implements Closeable {
    private record Stamp(long mtime, long size) {}

    private final List<Path> roots = new ArrayList<>();
    private final Consumer<Path> sink;
    private final long intervalMs;
    private final Thread thread;
    private volatile boolean running = true;
    private static final long HOT_MS = 30 * 60_000L;
    private static final int SWEEP_TICKS = 10;

    private final Map<Path, Long> dirs = new HashMap<>();
    private final Map<Path, Stamp> files = new HashMap<>();
    /** Files changed recently, with the time of their last change; checked on every tick. */
    private final Map<Path, Long> hot = new HashMap<>();
    private final List<Path> sweep = new ArrayList<>();
    private boolean sweepStale = true;
    private int sweepPos;
    private final List<Path> dirSweep = new ArrayList<>();
    private boolean dirSweepStale = true;
    private int dirSweepPos;

    PollingWatcher(Iterable<Path> roots, Consumer<Path> sink, long intervalMs) {
        for (Path r : roots) this.roots.add(r);
        this.sink = sink;
        this.intervalMs = intervalMs;
        for (Path r : this.roots) walk(r, false);
        this.thread = new Thread(this::loop, "hotdrop-poller");
        this.thread.setDaemon(true);
        this.thread.start();
    }

    private void loop() {
        while (running) {
            try {
                Thread.sleep(intervalMs);
                tick();
            } catch (InterruptedException e) {
                return;
            } catch (RuntimeException e) {
                // Never die quietly: a dead poller means saves are silently ignored.
                Log.warn("file polling error (continuing): %s", e);
            }
        }
    }

    private void tick() {
        // roots that did not exist yet (or were deleted and re-created)
        for (Path r : roots) {
            if (!dirs.containsKey(r) && Files.isDirectory(r)) walk(r, true);
        }
        long now = System.currentTimeMillis();
        hot.values().removeIf(t -> now - t > HOT_MS);
        // directories: a changed mtime means entries were added, removed or renamed. The directories of hot files
        // every tick, the rest in the rotating sweep.
        Set<Path> dirsNow = new LinkedHashSet<>();
        for (Path f : hot.keySet()) dirsNow.add(f.getParent());
        if (dirSweepStale) {
            dirSweep.clear();
            dirSweep.addAll(dirs.keySet());
            dirSweepStale = false;
            dirSweepPos = 0;
        }
        int dirSlice = (dirSweep.size() + SWEEP_TICKS - 1) / SWEEP_TICKS;
        for (int i = 0; i < dirSlice && !dirSweep.isEmpty(); i++) {
            if (dirSweepPos >= dirSweep.size()) dirSweepPos = 0;
            dirsNow.add(dirSweep.get(dirSweepPos++));
        }
        List<Path> changedDirs = new ArrayList<>();
        List<Path> goneDirs = new ArrayList<>();
        for (Path d : dirsNow) {
            Long was = dirs.get(d);
            if (was == null) continue;
            try {
                if (Files.getLastModifiedTime(d).toMillis() != was) changedDirs.add(d);
            } catch (IOException e) {
                goneDirs.add(d);
            }
        }
        for (Path gone : goneDirs) forgetDir(gone);
        for (Path d : changedDirs) relist(d);
        // files: in-place writes keep the directory mtime, so stat the hot files and a slice of the rest
        for (Path f : new ArrayList<>(hot.keySet())) check(f, now);
        if (sweepStale) {
            sweep.clear();
            sweep.addAll(files.keySet());
            sweepStale = false;
            sweepPos = 0;
        }
        int slice = (sweep.size() + SWEEP_TICKS - 1) / SWEEP_TICKS;
        for (int i = 0; i < slice && !sweep.isEmpty(); i++) {
            if (sweepPos >= sweep.size()) sweepPos = 0;
            Path f = sweep.get(sweepPos++);
            if (!hot.containsKey(f)) check(f, now);
        }
    }

    private void check(Path f, long now) {
        Stamp was = files.get(f);
        if (was == null) {
            hot.remove(f);
            return;
        }
        Stamp st = stamp(f);
        if (st == null) {
            files.remove(f);
            hot.remove(f);
            sweepStale = true;
            sink.accept(f);
        } else if (!st.equals(was)) {
            files.put(f, st);
            hot.put(f, now);
            sink.accept(f);
        }
    }

    private void relist(Path dir) {
        try {
            dirs.put(dir, Files.getLastModifiedTime(dir).toMillis());
        } catch (IOException e) {
            forgetDir(dir);
            return;
        }
        try (DirectoryStream<Path> s = Files.newDirectoryStream(dir)) {
            for (Path p : s) {
                if (Files.isDirectory(p)) {
                    if (!dirs.containsKey(p) && !skipped(p)) walk(p, true);
                } else if (isJava(p) && !files.containsKey(p)) {
                    Stamp st = stamp(p);
                    if (st != null) {
                        files.put(p, st);
                        hot.put(p, System.currentTimeMillis());
                        sweepStale = true;
                        sink.accept(p);
                    }
                }
            }
        } catch (IOException e) {
            Log.debug("cannot list %s: %s", dir, e.getMessage());
        }
        // A file deleted here, or replaced by a rename (safe write), changes this directory: check its known files now
        // instead of waiting for the sweep.
        long now = System.currentTimeMillis();
        for (Path f : new ArrayList<>(files.keySet())) {
            if (dir.equals(f.getParent())) check(f, now);
        }
    }

    private void forgetDir(Path dir) {
        dirs.keySet().removeIf(d -> d.startsWith(dir));
        dirSweepStale = true;
        for (Iterator<Path> it = files.keySet().iterator(); it.hasNext(); ) {
            Path f = it.next();
            if (f.startsWith(dir)) {
                it.remove();
                hot.remove(f);
                sweepStale = true;
                sink.accept(f);
            }
        }
    }

    /** Records every directory and .java file under {@code start}; with {@code report}, new files count as changes. */
    private void walk(Path start, boolean report) {
        try {
            Files.walkFileTree(start, new SimpleFileVisitor<>() {
                @Override
                public FileVisitResult preVisitDirectory(Path dir, BasicFileAttributes a) {
                    if (skipped(dir)) return FileVisitResult.SKIP_SUBTREE;
                    if (dirs.put(dir, a.lastModifiedTime().toMillis()) == null) dirSweepStale = true;
                    return FileVisitResult.CONTINUE;
                }

                @Override
                public FileVisitResult visitFile(Path f, BasicFileAttributes a) {
                    if (isJava(f) && !files.containsKey(f)) {
                        files.put(f, new Stamp(a.lastModifiedTime().toMillis(), a.size()));
                        sweepStale = true;
                        if (report) {
                            hot.put(f, System.currentTimeMillis());
                            sink.accept(f);
                        }
                    }
                    return FileVisitResult.CONTINUE;
                }

                @Override
                public FileVisitResult visitFileFailed(Path f, IOException e) {
                    return FileVisitResult.CONTINUE;
                }
            });
        } catch (NoSuchFileException e) {
            // gone already
        } catch (IOException e) {
            Log.debug("scan of %s failed: %s", start, e.getMessage());
        }
    }

    private static Stamp stamp(Path f) {
        try {
            BasicFileAttributes a = Files.readAttributes(f, BasicFileAttributes.class);
            return new Stamp(a.lastModifiedTime().toMillis(), a.size());
        } catch (IOException e) {
            return null;
        }
    }

    private static boolean isJava(Path p) {
        return p.getFileName().toString().endsWith(".java");
    }

    private static boolean skipped(Path dir) {
        String n = dir.getFileName() == null ? "" : dir.getFileName().toString();
        return n.equals(".git") || n.equals("node_modules");
    }

    @Override
    public void close() {
        running = false;
        thread.interrupt();
    }
}
