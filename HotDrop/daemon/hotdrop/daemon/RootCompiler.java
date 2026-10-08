package hotdrop.daemon;

import com.sun.source.util.JavacTask;
import com.sun.source.util.TaskEvent;
import com.sun.source.util.TaskListener;
import com.sun.source.util.TreePath;
import com.sun.source.util.Trees;

import javax.tools.Diagnostic;
import javax.tools.DiagnosticCollector;
import javax.tools.FileObject;
import javax.tools.ForwardingJavaFileManager;
import javax.tools.JavaCompiler;
import javax.tools.JavaFileManager;
import javax.tools.JavaFileObject;
import javax.tools.SimpleJavaFileObject;
import javax.tools.StandardJavaFileManager;
import javax.tools.StandardLocation;
import javax.tools.ToolProvider;
import javax.lang.model.element.TypeElement;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.OutputStream;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;
import java.util.TreeSet;
import java.util.concurrent.ConcurrentHashMap;

/**
 * One warm javac per root: the file manager (and its classpath index) lives as long as the classpath does.
 * Output goes to memory only; the scheduler alone decides what is written to disk or sent to the server.
 */
final class RootCompiler {
    record Diag(Path file, long line, long col, String message) {
        @Override
        public String toString() {
            return (file == null ? "" : file.getFileName() + ":") + line + ":" + col + ": " + message;
        }
    }

    static final class FileResult {
        final Path file;
        final List<Diag> errors = new ArrayList<>();
        final Set<String> declared = new TreeSet<>();
        final Set<String> refs = new TreeSet<>();
        final Map<String, String> abiParts = new TreeMap<>();
        final Map<String, byte[]> classes = new LinkedHashMap<>();
        boolean analyzed;

        FileResult(Path file) {
            this.file = file;
        }

        String abi() {
            return Abi.sha1(abiParts.toString().getBytes(StandardCharsets.UTF_8));
        }
    }

    static final class Output {
        final Map<Path, FileResult> files = new LinkedHashMap<>();
        /** Files outside the compiled set that failed to compile (pulled in from the source path). */
        final Map<Path, List<Diag>> foreignErrors = new LinkedHashMap<>();
        final List<String> unattributed = new ArrayList<>();
        boolean anyErrors;
        long nanos;
    }

    private static final class StringSource extends SimpleJavaFileObject {
        final Path path;
        final String content;

        StringSource(Path path, String content) {
            super(path.toUri(), Kind.SOURCE);
            this.path = path;
            this.content = content;
        }

        @Override
        public CharSequence getCharContent(boolean ignoreEncodingErrors) {
            return content;
        }
    }

    private static final class MemClass extends SimpleJavaFileObject {
        final String binaryName;
        final Path source;
        final ByteArrayOutputStream bytes = new ByteArrayOutputStream();

        MemClass(String binaryName, Path source) {
            super(URI.create("mem:///" + binaryName.replace('.', '/') + ".class"), Kind.CLASS);
            this.binaryName = binaryName;
            this.source = source;
        }

        @Override
        public OutputStream openOutputStream() {
            return bytes;
        }
    }

    /** Caches classpath/JDK package listings, which dominate javac start-up on a 1000-jar classpath. */
    private static final class CachingFileManager extends ForwardingJavaFileManager<StandardJavaFileManager> {
        final Map<String, List<JavaFileObject>> cache = new ConcurrentHashMap<>();
        final List<MemClass> outputs = new ArrayList<>();

        CachingFileManager(StandardJavaFileManager fm) {
            super(fm);
        }

        @Override
        public Iterable<JavaFileObject> list(Location loc, String pkg, Set<JavaFileObject.Kind> kinds, boolean recurse)
                throws IOException {
            String n = loc.getName();
            if (loc == StandardLocation.CLASS_PATH || loc == StandardLocation.PLATFORM_CLASS_PATH
                    || n.startsWith("SYSTEM_MODULES")) {
                String key = n + "|" + pkg + "|" + kinds + "|" + recurse;
                List<JavaFileObject> hit = cache.get(key);
                if (hit == null) {
                    hit = new ArrayList<>();
                    for (JavaFileObject o : super.list(loc, pkg, kinds, recurse)) hit.add(o);
                    cache.put(key, hit);
                }
                return hit;
            }
            return super.list(loc, pkg, kinds, recurse);
        }

        void invalidatePackage(String pkg) {
            String needle = "|" + pkg + "|";
            for (Iterator<String> it = cache.keySet().iterator(); it.hasNext(); ) {
                if (it.next().contains(needle)) it.remove();
            }
        }

        @Override
        public JavaFileObject getJavaFileForOutput(Location loc, String className, JavaFileObject.Kind kind,
                                                   FileObject sibling) throws IOException {
            if (loc == StandardLocation.CLASS_OUTPUT && kind == JavaFileObject.Kind.CLASS) {
                MemClass mc = new MemClass(className, sibling == null ? null : pathOf(sibling));
                outputs.add(mc);
                return mc;
            }
            return super.getJavaFileForOutput(loc, className, kind, sibling);
        }
    }

    private final JavaCompiler javac = ToolProvider.getSystemJavaCompiler();
    private final StandardJavaFileManager std;
    private final CachingFileManager fm;
    private final List<String> baseOptions;
    final List<Path> classpath;

    RootCompiler(List<Path> classpath, List<Path> sourcepath, List<String> baseOptions) throws IOException {
        if (javac == null) {
            throw new IllegalStateException("No system Java compiler: run HotDrop on a JDK, not a JRE");
        }
        this.classpath = List.copyOf(classpath);
        this.baseOptions = List.copyOf(baseOptions);
        this.std = javac.getStandardFileManager(null, Locale.ENGLISH, StandardCharsets.UTF_8);
        this.std.setLocationFromPaths(StandardLocation.CLASS_PATH, classpath);
        this.std.setLocationFromPaths(StandardLocation.SOURCE_PATH, sourcepath);
        this.fm = new CachingFileManager(std);
    }

    void invalidatePackage(String pkg) {
        fm.invalidatePackage(pkg);
    }

    /**
     * Compiles {@code files}. With {@code generate} false javac stops after flow analysis (used to build the index).
     * Javac keeps attributing every class after the first error so all broken files are found in one run.
     */
    synchronized Output compile(List<Path> files, boolean generate) {
        long t0 = System.nanoTime();
        Output out = new Output();
        fm.outputs.clear();

        List<StringSource> units = new ArrayList<>();
        for (Path p : files) {
            try {
                units.add(new StringSource(p, readSource(p)));
                out.files.put(p, new FileResult(p));
            } catch (IOException e) {
                out.unattributed.add("cannot read " + p + ": " + e.getMessage());
                out.anyErrors = true;
            }
        }
        if (units.isEmpty()) {
            out.nanos = System.nanoTime() - t0;
            return out;
        }

        List<String> opts = new ArrayList<>(baseOptions);
        opts.add(generate ? "-XDshould-stop.ifError=FLOW" : "-XDshould-stop.at=FLOW");
        DiagnosticCollector<JavaFileObject> diags = new DiagnosticCollector<>();
        try {
            JavacTask task = (JavacTask) javac.getTask(null, fm, diags, opts, null, units);
            Trees trees = Trees.instance(task);
            task.addTaskListener(new TaskListener() {
                @Override
                public void finished(TaskEvent e) {
                    if (e.getKind() != TaskEvent.Kind.ANALYZE) return;
                    try {
                        analysed(e, trees, out);
                    } catch (RuntimeException ex) {
                        Log.debug("analysis hook failed: %s", ex);
                    }
                }
            });
            Boolean ok = task.call();
            if (ok != null && !ok) out.anyErrors = true;
        } catch (IllegalArgumentException | IllegalStateException ex) {
            out.unattributed.add("javac could not run: " + ex.getMessage());
            out.anyErrors = true;
        } catch (RuntimeException ex) {
            out.unattributed.add("javac crashed: " + ex);
            out.anyErrors = true;
        }

        for (Diagnostic<? extends JavaFileObject> d : diags.getDiagnostics()) {
            if (d.getKind() != Diagnostic.Kind.ERROR) continue;
            out.anyErrors = true;
            Path p = d.getSource() == null ? null : pathOf(d.getSource());
            Diag diag = new Diag(p, d.getLineNumber(), d.getColumnNumber(),
                    d.getMessage(Locale.ENGLISH).strip().replaceAll("\\s*\\R\\s*", " | "));
            FileResult fr = p == null ? null : out.files.get(p);
            if (fr != null) {
                fr.errors.add(diag);
            } else if (p != null) {
                out.foreignErrors.computeIfAbsent(p, k -> new ArrayList<>()).add(diag);
            } else {
                out.unattributed.add(diag.message());
            }
        }

        for (MemClass mc : fm.outputs) {
            FileResult fr = mc.source == null ? null : out.files.get(mc.source);
            if (fr != null) fr.classes.put(mc.binaryName, mc.bytes.toByteArray());
        }
        fm.outputs.clear();
        out.nanos = System.nanoTime() - t0;
        return out;
    }

    /** Editors on Windows may hold a file exclusively while saving; wait a moment before giving up. */
    private static String readSource(Path p) throws IOException {
        IOException last = null;
        for (int attempt = 0; attempt < 6; attempt++) {
            try {
                return new String(Files.readAllBytes(p), StandardCharsets.UTF_8);
            } catch (IOException e) {
                last = e;
                try {
                    Thread.sleep(15L * (attempt + 1));
                } catch (InterruptedException ie) {
                    Thread.currentThread().interrupt();
                    break;
                }
            }
        }
        throw last;
    }

    private static void analysed(TaskEvent e, Trees trees, Output out) {
        JavaFileObject sf = e.getSourceFile();
        TypeElement te = e.getTypeElement();
        if (sf == null || te == null) return;
        FileResult fr = out.files.get(pathOf(sf));
        if (fr == null) return;
        fr.analyzed = true;
        String qn = te.getQualifiedName().toString();
        fr.declared.add(qn);
        fr.abiParts.put(qn, Abi.hash(te));
        TreePath path = trees.getPath(te);
        if (path != null) new RefScanner(trees).scan(path, fr.refs);
    }

    static Path pathOf(FileObject fo) {
        if (fo instanceof StringSource s) return s.path;
        if (fo instanceof MemClass m) return m.source;
        URI u = fo.toUri();
        if ("file".equals(u.getScheme())) return Path.of(u);
        return null;
    }

    static Set<Path> keys(Map<Path, ?> m) {
        return new LinkedHashSet<>(m.keySet());
    }
}
