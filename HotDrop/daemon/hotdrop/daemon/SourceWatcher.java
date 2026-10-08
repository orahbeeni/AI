package hotdrop.daemon;

import java.io.Closeable;
import java.io.IOException;
import java.nio.file.ClosedWatchServiceException;
import java.nio.file.FileSystems;
import java.nio.file.FileVisitResult;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.SimpleFileVisitor;
import java.nio.file.StandardWatchEventKinds;
import java.nio.file.WatchEvent;
import java.nio.file.WatchKey;
import java.nio.file.WatchService;
import java.nio.file.attribute.BasicFileAttributes;
import java.util.HashMap;
import java.util.Map;
import java.util.function.Consumer;

/** Recursive .java watcher on the JDK WatchService (inotify on Linux). */
final class SourceWatcher implements Closeable {
    private final WatchService ws;
    private final Map<WatchKey, Path> keys = new HashMap<>();
    private final Consumer<Path> sink;
    private final Thread thread;
    private volatile int failed;

    SourceWatcher(Iterable<Path> roots, Consumer<Path> sink) throws IOException {
        this.ws = FileSystems.getDefault().newWatchService();
        this.sink = sink;
        for (Path r : roots) registerAll(r);
        this.thread = new Thread(this::loop, "hotdrop-watcher");
        this.thread.setDaemon(true);
        this.thread.start();
    }

    /** Directories the OS refused to watch (for example the Linux inotify limit). */
    int failed() {
        return failed;
    }

    private synchronized void registerAll(Path root) throws IOException {
        if (!Files.isDirectory(root)) return;
        Files.walkFileTree(root, new SimpleFileVisitor<>() {
            @Override
            public FileVisitResult preVisitDirectory(Path dir, BasicFileAttributes attrs) throws IOException {
                try {
                    WatchKey k = dir.register(ws, StandardWatchEventKinds.ENTRY_CREATE,
                            StandardWatchEventKinds.ENTRY_MODIFY, StandardWatchEventKinds.ENTRY_DELETE);
                    keys.put(k, dir);
                } catch (IOException e) {
                    failed++;
                    Log.warn("cannot watch %s: %s", dir, e.getMessage());
                }
                return FileVisitResult.CONTINUE;
            }
        });
    }

    private void loop() {
        try {
            while (true) {
                WatchKey key = ws.take();
                Path dir;
                synchronized (this) {
                    dir = keys.get(key);
                }
                if (dir != null) {
                    for (WatchEvent<?> ev : key.pollEvents()) {
                        if (ev.kind() == StandardWatchEventKinds.OVERFLOW) {
                            Log.warn("file event overflow in %s; run 'hotdrop rescan'", dir);
                            continue;
                        }
                        Path p = dir.resolve((Path) ev.context());
                        if (ev.kind() == StandardWatchEventKinds.ENTRY_CREATE && Files.isDirectory(p)) {
                            try {
                                registerAll(p);
                                try (var s = Files.walk(p)) {
                                    s.filter(f -> f.toString().endsWith(".java")).forEach(sink);
                                }
                            } catch (IOException e) {
                                Log.warn("cannot watch new directory %s: %s", p, e.getMessage());
                            }
                        } else if (p.toString().endsWith(".java")) {
                            sink.accept(p);
                        }
                    }
                }
                if (!key.reset()) {
                    synchronized (this) {
                        keys.remove(key);
                    }
                }
            }
        } catch (InterruptedException | ClosedWatchServiceException e) {
            // shutting down
        }
    }

    @Override
    public void close() throws IOException {
        thread.interrupt();
        ws.close();
    }
}
