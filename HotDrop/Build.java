import javax.tools.JavaCompiler;
import javax.tools.ToolProvider;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.stream.Stream;

/**
 * Builds build/hotdrop-agent.jar and build/hotdrop-daemon.jar using only the JDK, identically on Linux, macOS and Windows.
 * Run from the repository root:  java Build.java
 */
public class Build {
    public static void main(String[] args) throws Exception {
        Path root = Path.of("").toAbsolutePath();
        if (!Files.isDirectory(root.resolve("protocol")) || !Files.isDirectory(root.resolve("daemon"))) {
            System.err.println("run this from the HotDrop repository root (the directory containing protocol/, agent/, daemon/)");
            System.exit(2);
        }
        Path build = root.resolve("build");
        if (Files.exists(build)) {
            try (Stream<Path> s = Files.walk(build)) {
                s.sorted(Comparator.reverseOrder()).forEach(p -> p.toFile().delete());
            }
        }
        Path protocolOut = build.resolve("classes/protocol");
        Path agentOut = build.resolve("classes/agent");
        Path daemonOut = build.resolve("classes/daemon");
        for (Path p : List.of(protocolOut, agentOut, daemonOut)) Files.createDirectories(p);

        javac(root.resolve("protocol"), protocolOut, null, "-Xlint:all");
        javac(root.resolve("agent"), agentOut, protocolOut, "-Xlint:all");
        javac(root.resolve("daemon"), daemonOut, protocolOut, "-Xlint:all,-processing");

        jar(build.resolve("hotdrop-agent.jar"), build.resolve("agent.mf"),
                "Premain-Class: hotdrop.agent.HotDropAgent\nAgent-Class: hotdrop.agent.HotDropAgent\n"
                        + "Can-Redefine-Classes: true\nCan-Retransform-Classes: true\n", agentOut, protocolOut);
        jar(build.resolve("hotdrop-daemon.jar"), build.resolve("daemon.mf"),
                "Main-Class: hotdrop.daemon.Main\n", daemonOut, protocolOut);
        System.out.println("built: " + build.resolve("hotdrop-agent.jar") + " " + build.resolve("hotdrop-daemon.jar"));
    }

    static void javac(Path srcDir, Path out, Path classpath, String lint) throws IOException {
        List<String> opts = new ArrayList<>(List.of("--release", "17", lint, "-encoding", "UTF-8", "-d", out.toString()));
        if (classpath != null) opts.addAll(List.of("-cp", classpath.toString()));
        try (Stream<Path> s = Files.walk(srcDir)) {
            s.filter(p -> p.toString().endsWith(".java")).forEach(p -> opts.add(p.toString()));
        }
        JavaCompiler jc = ToolProvider.getSystemJavaCompiler();
        if (jc == null) throw new IllegalStateException("run on a JDK (no system Java compiler found)");
        if (jc.run(null, null, null, opts.toArray(new String[0])) != 0) {
            System.err.println("compilation of " + srcDir + " failed");
            System.exit(1);
        }
    }

    static void jar(Path jarFile, Path manifest, String manifestText, Path... classDirs) throws IOException {
        Files.writeString(manifest, manifestText);
        List<String> a = new ArrayList<>(List.of("cfm", jarFile.toString(), manifest.toString()));
        for (Path d : classDirs) a.addAll(List.of("-C", d.toString(), "."));
        var jar = java.util.spi.ToolProvider.findFirst("jar").orElseThrow(() -> new IllegalStateException("jar tool not found; use a JDK"));
        if (jar.run(System.out, System.err, a.toArray(new String[0])) != 0) {
            System.err.println("jar " + jarFile + " failed");
            System.exit(1);
        }
    }
}
