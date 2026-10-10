package hotdrop.daemon;

import java.io.Closeable;
import java.io.IOException;
import java.nio.file.DirectoryStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.function.Consumer;

/**
 * Polls the (few) directories that hold Spring XML, model (items/beans) and message files, without recursion: they sit
 * directly in an extension's resources, localization, messages and WEB-INF directories. A real project has 100+ such directories, some with hundreds of
 * entries, so a tick must not list them all: it stats each directory (its mtime changes when a file is created,
 * deleted or renamed, which covers editors that save through a temporary file) and each known Spring file (which
 * covers in-place writes), and lists a directory only when its mtime moved.
 * Portable (no inotify/FSEvents dependency).
 */
final class ResourceWatcher implements Closeable {
    private record Stamp(long mtime, long size) {}

    private final List<Path> dirs;
    private final Consumer<Path> sink;
    private final long intervalMs;
    private final Map<Path, Long> dirStamps = new HashMap<>();
    private final Map<Path, Stamp> files = new HashMap<>();
    private final Thread thread;
    private volatile boolean running = true;

    ResourceWatcher(List<Path> dirs, Consumer<Path> sink, long intervalMs) {
        this.dirs = List.copyOf(dirs);
        this.sink = sink;
        this.intervalMs = intervalMs;
        for (Path d : this.dirs) list(d, false);
        this.thread = new Thread(this::loop, "hotdrop-spring-watcher");
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
                Log.warn("spring file polling error (continuing): %s", e);
            }
        }
    }

    private void tick() {
        for (Path d : dirs) {
            Long was = dirStamps.get(d);
            long now = mtime(d);
            if (was == null || was != now) list(d, true);
        }
        for (Path f : List.copyOf(files.keySet())) {
            Stamp now = stamp(f);
            Stamp was = files.get(f);
            if (now == null) {
                files.remove(f);
            } else if (!now.equals(was)) {
                files.put(f, now);
                sink.accept(f);
            }
        }
    }

    private void list(Path d, boolean report) {
        dirStamps.put(d, mtime(d));
        try (DirectoryStream<Path> ds = Files.newDirectoryStream(d)) {
            for (Path f : ds) {
                if (!Resources.watched(f)) continue;
                Stamp now = stamp(f);
                if (now == null) continue;
                Stamp was = files.put(f, now);
                if (report && !now.equals(was)) sink.accept(f);
            }
        } catch (IOException e) {
            // directory missing for now: -1 is remembered, so it is listed again when it reappears
        }
    }

    private static long mtime(Path d) {
        try {
            return Files.getLastModifiedTime(d).toMillis();
        } catch (IOException e) {
            return -1;
        }
    }

    private static Stamp stamp(Path f) {
        try {
            return new Stamp(Files.getLastModifiedTime(f).toMillis(), Files.size(f));
        } catch (IOException e) {
            return null;
        }
    }

    @Override
    public void close() {
        running = false;
        thread.interrupt();
    }
}
