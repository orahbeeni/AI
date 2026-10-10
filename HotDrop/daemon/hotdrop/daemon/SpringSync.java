package hotdrop.daemon;

import hotdrop.protocol.Wire;
import hotdrop.protocol.Wire.BeanResult;

import java.io.IOException;
import java.nio.file.DirectoryStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;

/**
 * Keeps the last-seen version of every watched Spring XML file and, when one is saved, sends the server the beans
 * that were added or had property values changed. Anything else is reported as restart required.
 * A file whose changes could not all be applied keeps its old baseline, so the next save retries them.
 */
final class SpringSync {
    private final List<Path> dirs;
    private final AgentLink agent;
    private final Map<Path, SpringXml.Snapshot> seen = new HashMap<>();

    SpringSync(List<Path> dirs, AgentLink agent) {
        this.dirs = List.copyOf(dirs);
        this.agent = agent;
        rebaseline();
    }

    List<Path> dirs() {
        return dirs;
    }

    synchronized int files() {
        return seen.size();
    }

    List<Path> springFiles() {
        List<Path> out = new ArrayList<>();
        for (Path d : dirs) {
            try (DirectoryStream<Path> ds = Files.newDirectoryStream(d)) {
                for (Path f : ds) if (SpringXml.isSpringFile(f) && Files.isRegularFile(f)) out.add(f.toAbsolutePath().normalize());
            } catch (IOException e) {
                // directory vanished
            }
        }
        return out;
    }

    /** Takes the files on disk as what the server has loaded (at start, or when a new server connects). */
    synchronized void rebaseline() {
        seen.clear();
        for (Path f : springFiles()) {
            SpringXml.Snapshot s = SpringXml.tryRead(f);
            if (s != null) seen.put(f, s);
        }
    }

    synchronized void apply(Set<Path> changed) {
        for (Path file : changed) {
            if (!Files.isRegularFile(file)) continue;
            SpringXml.Snapshot now = SpringXml.tryRead(file);
            if (now == null) continue;
            SpringXml.Snapshot old = seen.get(file);
            if (old == null) {
                seen.put(file, now);
                Log.info("[spring] new file %s: loaded by the server at its next start", file.getFileName());
                continue;
            }
            SpringXml.Diff diff = SpringXml.diff(file, old, now);
            if (diff.empty()) {
                seen.put(file, now);
                continue;
            }
            long t0 = System.nanoTime();
            boolean complete = diff.restart().isEmpty();
            Set<String> settled = new HashSet<>();
            String name = file.getFileName().toString();
            if (!diff.change().added().isEmpty() || !diff.change().changed().isEmpty()) {
                List<BeanResult> results = agent.spring(diff.change());
                if (results == null) {
                    Log.warn("[spring] %s changed but no server is connected; will retry on the next save", name);
                    complete = false;
                } else {
                    for (BeanResult r : results) {
                        if (r.status() == Wire.BEAN_ADDED || r.status() == Wire.BEAN_UPDATED) settled.add(r.id());
                        switch (r.status()) {
                            case Wire.BEAN_ADDED -> Log.info("[spring] %s: bean '%s' added (%s)", name, r.id(), r.message());
                            case Wire.BEAN_UPDATED -> Log.info("[spring] %s: bean '%s' updated (%s)", name, r.id(), r.message());
                            case Wire.BEAN_NO_CONTEXT -> {
                                Log.warn("[spring] %s: no running Spring context has loaded this file (%s)", name, r.message());
                                complete = false;
                            }
                            default -> {
                                Log.warn("[not applied] %s: bean '%s': %s (kept pending; fix and save again, or restart)", name, r.id(), r.message());
                                complete = false;
                            }
                        }
                    }
                    Log.info("[spring] %s applied in %s", name, Log.ms(System.nanoTime() - t0));
                }
            }
            for (String why : diff.restart()) {
                Log.warn("[restart required] %s: %s", name, why);
                agent.notice("Spring change in " + name + " needs a restart: " + why);
            }
            // Beans that were applied do not come back on the next save; the ones that were not are retried.
            seen.put(file, complete ? now : SpringXml.advance(old, now, settled));
        }
    }
}
