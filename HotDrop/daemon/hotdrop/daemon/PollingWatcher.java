package hotdrop.daemon;

import java.io.Closeable;
import java.io.IOException;
import java.nio.file.FileVisitResult;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.SimpleFileVisitor;
import java.nio.file.attribute.BasicFileAttributes;
import java.util.HashMap;
import java.util.Map;
import java.util.function.Consumer;

/**
 * Stat-based watcher for platforms where the JDK WatchService is slow (macOS polls every few seconds) or
 * when native watches cannot be registered. Scans only .java files, comparing modification time and size.
 * A few thousand files cost tens of milliseconds per scan, so it runs every {@code intervalMs}.
 */
final class PollingWatcher implements Closeable {
    private record Stamp(long mtime, long size) {}

    private final Iterable<Path> roots;
    private final Consumer<Path> sink;
    private final long intervalMs;
    private final Thread thread;
    private volatile boolean running = true;
    private Map<Path, Stamp> previous;

    PollingWatcher(Iterable<Path> roots, Consumer<Path> sink, long intervalMs) {
        this.roots = roots;
        this.sink = sink;
        this.intervalMs = intervalMs;
        this.previous = scan();
        this.thread = new Thread(this::loop, "hotdrop-poller");
        this.thread.setDaemon(true);
        this.thread.start();
    }

    private void loop() {
        try {
            while (running) {
                Thread.sleep(intervalMs);
                Map<Path, Stamp> now = scan();
                for (Map.Entry<Path, Stamp> e : now.entrySet()) {
                    if (!e.getValue().equals(previous.get(e.getKey()))) sink.accept(e.getKey());
                }
                for (Path gone : previous.keySet()) {
                    if (!now.containsKey(gone)) sink.accept(gone);
                }
                previous = now;
            }
        } catch (InterruptedException e) {
            // shutting down
        }
    }

    private Map<Path, Stamp> scan() {
        Map<Path, Stamp> out = new HashMap<>();
        for (Path root : roots) {
            if (!Files.isDirectory(root)) continue;
            try {
                Files.walkFileTree(root, new SimpleFileVisitor<>() {
                    @Override
                    public FileVisitResult preVisitDirectory(Path dir, BasicFileAttributes a) {
                        String n = dir.getFileName() == null ? "" : dir.getFileName().toString();
                        return n.equals(".git") || n.equals("node_modules") ? FileVisitResult.SKIP_SUBTREE : FileVisitResult.CONTINUE;
                    }

                    @Override
                    public FileVisitResult visitFile(Path f, BasicFileAttributes a) {
                        if (f.getFileName().toString().endsWith(".java")) {
                            out.put(f, new Stamp(a.lastModifiedTime().toMillis(), a.size()));
                        }
                        return FileVisitResult.CONTINUE;
                    }

                    @Override
                    public FileVisitResult visitFileFailed(Path f, IOException e) {
                        return FileVisitResult.CONTINUE;
                    }
                });
            } catch (IOException e) {
                Log.debug("scan of %s failed: %s", root, e.getMessage());
            }
        }
        return out;
    }

    @Override
    public void close() {
        running = false;
        thread.interrupt();
    }
}
