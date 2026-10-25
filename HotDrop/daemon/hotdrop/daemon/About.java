package hotdrop.daemon;

import java.io.IOException;
import java.io.InputStream;
import java.util.Properties;

/** The build number stamped into the jar by Build.java, for {@code hotdrop about}. */
final class About {
    private About() {}

    static void print() {
        Properties p = load();
        String commit = p.getProperty("commit", "");
        System.out.println("HotDrop");
        System.out.println("Build " + p.getProperty("number", "unknown") + (commit.isEmpty() ? "" : " (" + commit + ")"));
        System.out.println("Built " + p.getProperty("time", "unknown"));
        System.out.println("Java " + System.getProperty("java.version") + " (" + System.getProperty("os.name") + " " + System.getProperty("os.arch") + ")");
    }

    private static Properties load() {
        Properties p = new Properties();
        try (InputStream in = About.class.getResourceAsStream("/hotdrop-build.properties")) {
            if (in != null) p.load(in);
        } catch (IOException ignored) {
            // an unstamped jar just reports "unknown"
        }
        return p;
    }
}
