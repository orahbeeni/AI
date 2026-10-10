package hotdrop.daemon;

import org.w3c.dom.Document;
import org.w3c.dom.Element;
import org.w3c.dom.NodeList;

import javax.xml.parsers.DocumentBuilderFactory;
import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Properties;
import java.util.Set;
import java.util.stream.Stream;

/** Finds custom extensions, their source roots and the compiler settings the Hybris build uses. */
final class Discovery {
    private Discovery() {}

    record BuildSettings(String level, String encoding, List<String> exports, boolean parameters) {}

    static BuildSettings buildSettings(Path hybris) {
        Properties p = properties(hybris);
        String level = p.getProperty("build.target");
        if (level != null) level = level.trim();
        String enc = p.getProperty("build.encoding", "UTF8").trim();
        List<String> exports = new ArrayList<>();
        String raw = p.getProperty("standalone.jdkmodulesexports", "");
        for (String tok : raw.split("\\s+")) {
            String t = tok.replace("\"", "");
            if (t.startsWith("--add-exports=")) exports.add(t);
        }
        // Hybris compiles with -parameters by default (build.parameter-metadata); Spring 6 needs the names for @PathVariable etc.
        boolean parameters = !"false".equalsIgnoreCase(p.getProperty("build.parameter-metadata", "true").trim());
        return new BuildSettings(level, enc, exports, parameters);
    }

    /** Where an enabled custom extension keeps Spring XML: core/global files, web contexts and addon web contexts. */
    static List<Path> springDirs(Path hybris) throws IOException {
        Path custom = hybris.resolve("bin").resolve("custom");
        List<Path> dirs = new ArrayList<>();
        if (!Files.isDirectory(custom)) return dirs;
        Enabled enabled = readLocalExtensions(hybris);
        List<Path> extDirs = new ArrayList<>();
        try (Stream<Path> s = Files.walk(custom, 4)) {
            s.filter(p -> p.getFileName().toString().equals("extensioninfo.xml")).forEach(p -> extDirs.add(p.getParent()));
        }
        for (Path ext : extDirs) {
            if (!enabled.allows(extensionName(ext), ext)) continue;
            for (String rel : new String[]{"resources", "web/webroot/WEB-INF", "acceleratoraddon/web/webroot/WEB-INF", "backoffice/resources"}) {
                Path d = ext.resolve(rel);
                if (Files.isDirectory(d)) dirs.add(d);
            }
        }
        return dirs;
    }

    /** Where an enabled custom extension keeps message bundles. */
    static List<Path> messageDirs(Path hybris) throws IOException {
        Path custom = hybris.resolve("bin").resolve("custom");
        List<Path> dirs = new ArrayList<>();
        if (!Files.isDirectory(custom)) return dirs;
        Enabled enabled = readLocalExtensions(hybris);
        List<Path> extDirs = new ArrayList<>();
        try (Stream<Path> s = Files.walk(custom, 4)) {
            s.filter(p -> p.getFileName().toString().equals("extensioninfo.xml")).forEach(p -> extDirs.add(p.getParent()));
        }
        for (Path ext : extDirs) {
            if (!enabled.allows(extensionName(ext), ext)) continue;
            for (String rel : new String[]{"resources/localization", "web/webroot/WEB-INF/messages", "acceleratoraddon/web/webroot/WEB-INF/messages"}) {
                Path d = ext.resolve(rel);
                if (Files.isDirectory(d)) dirs.add(d);
            }
        }
        return dirs;
    }

    /** resources/impex and resources/<ext>/import, with their subdirectories (down to 4 levels). */
    static List<Path> impexDirs(Path hybris) throws IOException {
        Path custom = hybris.resolve("bin").resolve("custom");
        List<Path> dirs = new ArrayList<>();
        if (!Files.isDirectory(custom)) return dirs;
        Enabled enabled = readLocalExtensions(hybris);
        List<Path> extDirs = new ArrayList<>();
        try (Stream<Path> s = Files.walk(custom, 4)) {
            s.filter(p -> p.getFileName().toString().equals("extensioninfo.xml")).forEach(p -> extDirs.add(p.getParent()));
        }
        for (Path ext : extDirs) {
            String name = extensionName(ext);
            if (!enabled.allows(name, ext)) continue;
            for (Path top : new Path[]{ext.resolve("resources/impex"), ext.resolve("resources").resolve(name).resolve("import")}) {
                if (!Files.isDirectory(top)) continue;
                try (Stream<Path> s = Files.walk(top, 4)) {
                    s.filter(Files::isDirectory).forEach(dirs::add);
                }
            }
        }
        return dirs;
    }

    /** The merged Hybris properties (platform defaults, advanced, then the user's local.properties). */
    static Properties properties(Path hybris) {
        Properties p = new Properties();
        for (Path f : new Path[]{
                hybris.resolve("bin/platform/project.properties"),
                hybris.resolve("bin/platform/resources/advanced.properties"),
                hybris.resolve("config/local.properties")}) {
            if (Files.isRegularFile(f)) {
                try (InputStream in = Files.newInputStream(f)) {
                    p.load(in);
                } catch (IOException e) {
                    Log.warn("cannot read %s: %s", f, e.getMessage());
                }
            }
        }
        return p;
    }

    static List<Root> discover(Path hybris) throws IOException {
        Path bin = hybris.resolve("bin");
        Path custom = bin.resolve("custom");
        List<Root> roots = new ArrayList<>();
        if (!Files.isDirectory(custom)) {
            Log.warn("no custom extensions directory at %s", custom);
            return roots;
        }
        Enabled enabled = readLocalExtensions(hybris);
        List<Path> extDirs = new ArrayList<>();
        try (Stream<Path> s = Files.walk(custom, 4)) {
            s.filter(p -> p.getFileName().toString().equals("extensioninfo.xml")).forEach(p -> extDirs.add(p.getParent()));
        }
        for (Path ext : extDirs) {
            String name = extensionName(ext);
            if (!enabled.allows(name, ext)) {
                Log.debug("skipping %s (not in localextensions.xml)", name);
                continue;
            }
            Path src = ext.resolve("src");
            if (Files.isDirectory(src)) {
                roots.add(new Root(name, src, ext.resolve("classes"), ext, false));
            }
            Path webSrc = ext.resolve("web/src");
            if (Files.isDirectory(webSrc)) {
                roots.add(new Root(name + ":web", webSrc, ext.resolve("web/webroot/WEB-INF/classes"), ext, true));
            }
            // Addons keep their storefront code under acceleratoraddon/web/src, backoffice modules under backoffice/src.
            Path addonSrc = ext.resolve("acceleratoraddon/web/src");
            if (Files.isDirectory(addonSrc)) {
                roots.add(new Root(name + ":addon", addonSrc, ext.resolve("acceleratoraddon/web/webroot/WEB-INF/classes"), ext, true));
            }
            Path boSrc = ext.resolve("backoffice/src");
            if (Files.isDirectory(boSrc)) {
                roots.add(new Root(name + ":backoffice", boSrc, ext.resolve("backoffice/classes"), ext, true));
            }
        }
        return roots;
    }

    private static String extensionName(Path extDir) {
        try {
            Document d = parse(extDir.resolve("extensioninfo.xml"));
            NodeList l = d.getElementsByTagName("extension");
            if (l.getLength() > 0) {
                String n = ((Element) l.item(0)).getAttribute("name");
                if (!n.isEmpty()) return n;
            }
        } catch (Exception ignored) {
            // fall through to directory name
        }
        return extDir.getFileName().toString();
    }

    private record Enabled(boolean all, Set<String> names, Set<Path> dirs, Set<Path> autoloadParents) {
        boolean allows(String name, Path ext) {
            return all || names.contains(name) || dirs.contains(ext.toAbsolutePath().normalize())
                    || autoloadParents.contains(ext.toAbsolutePath().normalize().getParent());
        }
    }

    private static Enabled readLocalExtensions(Path hybris) {
        Path f = hybris.resolve("config/localextensions.xml");
        if (!Files.isRegularFile(f)) return new Enabled(true, Set.of(), Set.of(), Set.of());
        try {
            Document d = parse(f);
            Set<String> names = new HashSet<>();
            Set<Path> dirs = new HashSet<>();
            Set<Path> parents = new HashSet<>();
            NodeList exts = d.getElementsByTagName("extension");
            for (int i = 0; i < exts.getLength(); i++) {
                Element e = (Element) exts.item(i);
                if (!e.getAttribute("name").isEmpty()) names.add(e.getAttribute("name"));
                if (!e.getAttribute("dir").isEmpty()) dirs.add(resolve(hybris, e.getAttribute("dir")));
            }
            NodeList paths = d.getElementsByTagName("path");
            for (int i = 0; i < paths.getLength(); i++) {
                Element e = (Element) paths.item(i);
                if (e.getAttribute("dir").isEmpty()) continue;
                if ("false".equals(e.getAttribute("autoload"))) continue;
                parents.add(resolve(hybris, e.getAttribute("dir")));
            }
            return new Enabled(false, names, dirs, parents);
        } catch (Exception e) {
            Log.warn("cannot parse %s (%s); watching every custom extension", f, e.getMessage());
            return new Enabled(true, Set.of(), Set.of(), Set.of());
        }
    }

    private static Path resolve(Path hybris, String dir) {
        String bin = hybris.resolve("bin").toString();
        String s = dir.replace("${HYBRIS_BIN_DIR}", bin)
                .replace("${platformhome}", bin + "/platform")
                .replace("${HYBRIS_CONFIG_DIR}", hybris.resolve("config").toString());
        return Path.of(s).toAbsolutePath().normalize();
    }

    private static Document parse(Path f) throws Exception {
        DocumentBuilderFactory dbf = DocumentBuilderFactory.newInstance();
        dbf.setFeature("http://apache.org/xml/features/disallow-doctype-decl", true);
        dbf.setFeature("http://xml.org/sax/features/external-general-entities", false);
        return dbf.newDocumentBuilder().parse(f.toFile());
    }
}
