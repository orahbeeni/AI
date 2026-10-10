package hotdrop.daemon;

import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;

final class Config {
    Path hybris;
    final List<Root> manualRoots = new ArrayList<>();
    /** Directories (not searched recursively) whose *-spring.xml files are watched for bean changes. */
    final List<Path> springDirs = new ArrayList<>();
    /** Directories whose message bundles (.properties) are watched. */
    final List<Path> messageDirs = new ArrayList<>();
    /** Opt-in: run *.impex files that carry a '# hotdrop-on-save' line through HAC when they are saved. */
    boolean impex;
    String hacUrl = "https://localhost:9002/hac";
    String hacUser = "admin";
    String hacPassword = System.getenv().getOrDefault("HOTDROP_HAC_PASSWORD", "nimda");
    final List<Path> impexDirs = new ArrayList<>();
    boolean spring = true;
    final List<Path> extraClasspath = new ArrayList<>();
    Path home = Path.of(System.getProperty("user.home"), ".hotdrop");
    int debounceMs = 40;
    boolean index = true;
    /** auto: native events where they are fast (Linux, Windows), polling on macOS; native | poll force a choice. */
    String watch = "auto";
    int pollMs = 100;

    Path agentsDir() {
        return home.resolve("agents");
    }

    Path platformHome() {
        return hybris == null ? null : hybris.resolve("bin").resolve("platform");
    }
}
