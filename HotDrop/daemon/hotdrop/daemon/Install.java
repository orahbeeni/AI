package hotdrop.daemon;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Scanner;

/** "hotdrop install": adds the -javaagent option to local.properties so HotDrop starts with the server. */
final class Install {
    private static final String MARK = "# HotDrop";

    private Install() {}

    static int run(Config cfg, boolean yes, boolean remove) throws IOException {
        Path hybris = cfg.hybris;
        if (hybris == null) {
            Up.Server s = Up.findServer(null);
            if (s != null) hybris = s.hybris();
        }
        if (hybris == null) {
            System.err.println("install: pass --hybris <dir> (or start the server so it can be found)");
            return 2;
        }
        // CCv2 projects keep developer settings in config/local-config/*.properties; classic ones in config/local.properties.
        Path classic = hybris.resolve("config").resolve("local.properties");
        Path localConfig = hybris.resolve("config").resolve("local-config");
        Path file = Files.isDirectory(localConfig) ? localConfig.resolve("99-local.properties") : classic;
        if (remove && !hasHotDropLines(file) && hasHotDropLines(classic)) file = classic;
        if (!Files.isRegularFile(file)) {
            if (remove || !Files.isDirectory(file.getParent())) {
                System.err.println("install: " + file + " not found");
                return 2;
            }
            Files.createFile(file);
        }
        if (!remove && !file.equals(classic) && hasHotDropLines(classic)) {
            System.out.println("note: " + classic + " also has HotDrop lines; run 'hotdrop install --remove' first to take those out");
        }
        Path agent;
        try {
            agent = Path.of(Main.class.getProtectionDomain().getCodeSource().getLocation().toURI()).resolveSibling("hotdrop-agent.jar");
        } catch (java.net.URISyntaxException e) {
            throw new IOException(e);
        }
        String opt = "-javaagent:" + agent.toAbsolutePath().toString().replace('\\', '/');

        List<String> before = Files.readAllLines(file);
        List<String> after = new ArrayList<>();
        boolean found = false;
        for (int i = 0; i < before.size(); i++) {
            String l = before.get(i);
            if (l.startsWith(MARK) || l.contains("hotdrop-agent.jar")) {
                found = true;
                continue; // drop our previous block; re-added below unless removing
            }
            after.add(l);
        }
        if (!remove) {
            if (!after.isEmpty() && !after.get(after.size() - 1).isBlank()) after.add("");
            after.add(MARK + " - starts the watcher together with the server (remove these lines, or run 'hotdrop install --remove')");
            after.add("tomcat.javaoptions=" + opt);
            // Debug start (ystartDebug) uses tomcat.debugjavaoptions INSTEAD of tomcat.javaoptions, so keep its default value.
            String debugDefault = platformDefault(hybris, "tomcat.debugjavaoptions");
            if (debugDefault != null) after.add("tomcat.debugjavaoptions=" + debugDefault + " " + opt);
            else System.err.println("note: could not read the default tomcat.debugjavaoptions; debug start will not load HotDrop on its own");
        } else if (!found) {
            System.out.println("nothing to remove: no HotDrop lines in " + file);
            return 0;
        }
        for (String key : new String[] {"tomcat.javaoptions", "tomcat.debugjavaoptions"}) {
            for (String l : after) {
                if (l.startsWith(key + "=") && !l.contains("hotdrop-agent.jar") && !remove) {
                    System.err.println("install: " + file + " already sets " + key + ":\n  " + l
                            + "\nAdd '" + opt + "' to that value by hand (space separated) and run 'ant server'.");
                    return 1;
                }
            }
        }
        System.out.println(file + (remove ? " - HotDrop lines to be removed:" : " - lines to be added:"));
        if (remove) {
            for (String l : before) if (l.contains("hotdrop-agent.jar") || l.startsWith(MARK)) System.out.println("  - " + l);
        } else {
            for (int i = Math.max(0, after.size() - 4); i < after.size(); i++) System.out.println("  + " + after.get(i));
        }
        if (!yes) {
            System.out.print("write this change? [y/N] ");
            Scanner in = new Scanner(System.in);
            String a = in.hasNextLine() ? in.nextLine() : "";
            if (!a.trim().equalsIgnoreCase("y")) {
                System.out.println("not changed");
                return 1;
            }
        }
        Path backup = file.resolveSibling(file.getFileName() + ".hotdrop-backup");
        Files.copy(file, backup, java.nio.file.StandardCopyOption.REPLACE_EXISTING);
        Files.write(file, after);
        System.out.println("done (backup: " + backup + ").");
        System.out.println("next: run 'ant server' once so wrapper.conf is regenerated, then (re)start Hybris.");
        return 0;
    }

    private static boolean hasHotDropLines(Path f) throws IOException {
        if (!Files.isRegularFile(f)) return false;
        for (String l : Files.readAllLines(f)) if (l.contains("hotdrop-agent.jar")) return true;
        return false;
    }

    /** The value Hybris uses when local.properties does not set the key (last uncommented line in the platform file). */
    private static String platformDefault(Path hybris, String key) throws IOException {
        Path f = hybris.resolve("bin").resolve("platform").resolve("project.properties");
        if (!Files.isRegularFile(f)) return null;
        String value = null;
        for (String l : Files.readAllLines(f)) {
            if (l.startsWith(key + "=")) value = l.substring(key.length() + 1).trim();
        }
        return value;
    }
}
