package hotdrop.daemon;

import java.io.IOException;
import java.nio.file.DirectoryStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

/** What kind of non-Java file a watched path is, and how HotDrop reacts to it. */
final class Resources {
    private Resources() {}

    enum Kind {
        /** *-spring.xml: bean changes are applied to the running contexts. */
        SPRING,
        /** *-items.xml and *-beans.xml: need a build (and a system update), so the user is told. */
        MODEL,
        /** .properties in a localization or messages directory: message source caches are cleared. */
        MESSAGES,
        NONE
    }

    static Kind kind(Path p) {
        String n = p.getFileName() == null ? "" : p.getFileName().toString();
        if (SpringXml.isSpringFile(p)) return Kind.SPRING;
        if (n.endsWith("-items.xml") || n.endsWith("-beans.xml")) return Kind.MODEL;
        if (n.endsWith(".properties") && p.getParent() != null && p.getParent().getFileName() != null) {
            String dir = p.getParent().getFileName().toString();
            if (dir.equals("localization") || dir.equals("messages")) return Kind.MESSAGES;
        }
        return Kind.NONE;
    }

    static boolean watched(Path p) {
        return kind(p) != Kind.NONE;
    }

    /** The content hash of every model file directly in {@code dirs}, to tell a real edit from a touch. */
    static java.util.Map<Path, Integer> modelHashes(List<Path> dirs) {
        java.util.Map<Path, Integer> out = new java.util.HashMap<>();
        for (Path d : dirs) {
            try (DirectoryStream<Path> ds = Files.newDirectoryStream(d)) {
                for (Path f : ds) {
                    if (kind(f) == Kind.MODEL && Files.isRegularFile(f)) {
                        Integer h = hash(f);
                        if (h != null) out.put(f.toAbsolutePath().normalize(), h);
                    }
                }
            } catch (IOException e) {
                // directory vanished
            }
        }
        return out;
    }

    static Integer hash(Path f) {
        try {
            return Arrays.hashCode(Files.readAllBytes(f));
        } catch (IOException e) {
            return null;
        }
    }

    static List<Path> union(List<Path> a, List<Path> b) {
        List<Path> out = new ArrayList<>(a);
        for (Path p : b) if (!out.contains(p)) out.add(p);
        return out;
    }
}
