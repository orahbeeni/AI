package hotdrop.protocol;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;

/**
 * Framed binary protocol between the HotDrop daemon and the in-server agent.
 * Frame: [int length][byte type][payload]; length counts the type byte plus the payload.
 * No dependencies on purpose: this class is compiled into the agent, which runs inside the Hybris JVM.
 */
public final class Wire {
    private Wire() {}

    public static final int PROTOCOL_VERSION = 1;

    public static final byte HELLO = 1;
    public static final byte HELLO_OK = 2;
    public static final byte INVENTORY_REQ = 3;
    public static final byte INVENTORY = 4;
    public static final byte REDEFINE = 5;
    public static final byte RESULT = 6;
    public static final byte ERROR = 7;
    public static final byte PING = 8;
    public static final byte PONG = 9;
    /** Daemon -> agent: a line of text for the server console (broken file, held class); answered with PONG. */
    public static final byte NOTICE = 10;

    /** Daemon -> agent: apply the bean changes found in one Spring XML file; answered with SPRING_RESULT. */
    public static final byte SPRING = 11;
    public static final byte SPRING_RESULT = 12;

    /** Daemon -> agent: a message bundle (.properties) changed; clear the MessageSource caches. Answered with PONG. */
    public static final byte CLEAR_MESSAGES = 13;

    public static final byte BEAN_ADDED = 0;
    public static final byte BEAN_UPDATED = 1;
    public static final byte BEAN_FAILED = 2;
    /** No running Spring context has loaded this file (yet). */
    public static final byte BEAN_NO_CONTEXT = 3;

    public static final byte SWAPPED = 0;
    public static final byte REJECTED = 1;
    public static final byte NOT_LOADED = 2;

    private static final int MAX_FRAME = 256 * 1024 * 1024;

    public record Frame(byte type, byte[] payload) {}

    public record AgentInfo(int protocol, long pid, String vendor, String vmVersion, String javaHome,
                            String javaVersion, boolean redefineSupported, boolean enhancedRedefine,
                            long startMillis, String platformHome) {}

    public record LoaderInfo(long id, long parentId, String type, String name, List<String> urls) {}

    /** One class to redefine. {@code scope} is a directory; only loaded copies whose code source lies under it are touched. */
    public record ClassEntry(String name, String scope, byte[] bytes) {}

    public record ClassResult(String name, byte status, String message, int copies) {}

    /** A bean whose properties changed ({@code props} empty: the whole util:list / set / map is replaced). */
    public record BeanChange(String id, List<String> props) {}

    /** What changed in one Spring XML file. {@code known} are the bean ids of the previous version of the file. */
    public record SpringChange(String file, List<String> added, List<BeanChange> changed, List<String> known) {}

    public record BeanResult(String id, byte status, String message) {}

    public record BatchResult(long batchId, boolean fallback, long nanos, List<ClassResult> results) {}

    public static void write(DataOutputStream out, byte type, byte[] payload) throws IOException {
        out.writeInt(payload.length + 1);
        out.writeByte(type);
        out.write(payload);
        out.flush();
    }

    public static Frame read(DataInputStream in) throws IOException {
        int len = in.readInt();
        if (len < 1 || len > MAX_FRAME) {
            throw new IOException("bad frame length " + len);
        }
        byte type = in.readByte();
        byte[] payload = new byte[len - 1];
        in.readFully(payload);
        return new Frame(type, payload);
    }

    public static final class Out {
        private final ByteArrayOutputStream bos = new ByteArrayOutputStream();
        private final DataOutputStream d = new DataOutputStream(bos);

        public Out str(String s) throws IOException {
            byte[] b = (s == null ? "" : s).getBytes(StandardCharsets.UTF_8);
            d.writeInt(b.length);
            d.write(b);
            return this;
        }

        public Out bytes(byte[] b) throws IOException { d.writeInt(b.length); d.write(b); return this; }
        public Out i32(int v) throws IOException { d.writeInt(v); return this; }
        public Out i64(long v) throws IOException { d.writeLong(v); return this; }
        public Out i8(byte v) throws IOException { d.writeByte(v); return this; }
        public Out bool(boolean v) throws IOException { d.writeBoolean(v); return this; }
        public byte[] done() { return bos.toByteArray(); }
    }

    public static final class In {
        private final DataInputStream d;

        public In(byte[] payload) { this.d = new DataInputStream(new ByteArrayInputStream(payload)); }

        public String str() throws IOException {
            int n = d.readInt();
            if (n < 0 || n > MAX_FRAME) throw new IOException("bad string length " + n);
            byte[] b = new byte[n];
            d.readFully(b);
            return new String(b, StandardCharsets.UTF_8);
        }

        public byte[] bytes() throws IOException {
            int n = d.readInt();
            if (n < 0 || n > MAX_FRAME) throw new IOException("bad byte[] length " + n);
            byte[] b = new byte[n];
            d.readFully(b);
            return b;
        }

        public int i32() throws IOException { return d.readInt(); }
        public long i64() throws IOException { return d.readLong(); }
        public byte i8() throws IOException { return d.readByte(); }
        public boolean bool() throws IOException { return d.readBoolean(); }
    }

    // ---- codecs ----

    public static byte[] encodeHello(String token) throws IOException {
        return new Out().i32(PROTOCOL_VERSION).str(token).done();
    }

    public static byte[] encodeAgentInfo(AgentInfo a) throws IOException {
        return new Out().i32(a.protocol()).i64(a.pid()).str(a.vendor()).str(a.vmVersion()).str(a.javaHome())
                .str(a.javaVersion()).bool(a.redefineSupported()).bool(a.enhancedRedefine())
                .i64(a.startMillis()).str(a.platformHome()).done();
    }

    public static AgentInfo decodeAgentInfo(byte[] p) throws IOException {
        In in = new In(p);
        return new AgentInfo(in.i32(), in.i64(), in.str(), in.str(), in.str(), in.str(), in.bool(), in.bool(),
                in.i64(), in.str());
    }

    public static byte[] encodeInventory(List<LoaderInfo> loaders) throws IOException {
        Out o = new Out().i32(loaders.size());
        for (LoaderInfo l : loaders) {
            o.i64(l.id()).i64(l.parentId()).str(l.type()).str(l.name()).i32(l.urls().size());
            for (String u : l.urls()) o.str(u);
        }
        return o.done();
    }

    public static List<LoaderInfo> decodeInventory(byte[] p) throws IOException {
        In in = new In(p);
        int n = in.i32();
        List<LoaderInfo> list = new ArrayList<>(n);
        for (int i = 0; i < n; i++) {
            long id = in.i64();
            long parent = in.i64();
            String type = in.str();
            String name = in.str();
            int nu = in.i32();
            List<String> urls = new ArrayList<>(nu);
            for (int j = 0; j < nu; j++) urls.add(in.str());
            list.add(new LoaderInfo(id, parent, type, name, urls));
        }
        return list;
    }

    public static byte[] encodeRedefine(long batchId, List<ClassEntry> entries) throws IOException {
        Out o = new Out().i64(batchId).i32(entries.size());
        for (ClassEntry e : entries) o.str(e.name()).str(e.scope()).bytes(e.bytes());
        return o.done();
    }

    public static long redefineBatchId(byte[] p) throws IOException { return new In(p).i64(); }

    public static List<ClassEntry> decodeRedefine(byte[] p) throws IOException {
        In in = new In(p);
        in.i64();
        int n = in.i32();
        List<ClassEntry> list = new ArrayList<>(n);
        for (int i = 0; i < n; i++) list.add(new ClassEntry(in.str(), in.str(), in.bytes()));
        return list;
    }

    public static byte[] encodeResult(BatchResult r) throws IOException {
        Out o = new Out().i64(r.batchId()).bool(r.fallback()).i64(r.nanos()).i32(r.results().size());
        for (ClassResult c : r.results()) o.str(c.name()).i8(c.status()).str(c.message()).i32(c.copies());
        return o.done();
    }

    public static BatchResult decodeResult(byte[] p) throws IOException {
        In in = new In(p);
        long id = in.i64();
        boolean fallback = in.bool();
        long nanos = in.i64();
        int n = in.i32();
        List<ClassResult> list = new ArrayList<>(n);
        for (int i = 0; i < n; i++) list.add(new ClassResult(in.str(), in.i8(), in.str(), in.i32()));
        return new BatchResult(id, fallback, nanos, list);
    }

    public static byte[] encodeSpring(SpringChange c) throws IOException {
        Out o = new Out().str(c.file()).i32(c.added().size());
        for (String a : c.added()) o.str(a);
        o.i32(c.changed().size());
        for (BeanChange b : c.changed()) {
            o.str(b.id()).i32(b.props().size());
            for (String p : b.props()) o.str(p);
        }
        o.i32(c.known().size());
        for (String k : c.known()) o.str(k);
        return o.done();
    }

    public static SpringChange decodeSpring(byte[] p) throws IOException {
        In in = new In(p);
        String file = in.str();
        int na = in.i32();
        List<String> added = new ArrayList<>(na);
        for (int i = 0; i < na; i++) added.add(in.str());
        int nc = in.i32();
        List<BeanChange> changed = new ArrayList<>(nc);
        for (int i = 0; i < nc; i++) {
            String id = in.str();
            int np = in.i32();
            List<String> props = new ArrayList<>(np);
            for (int j = 0; j < np; j++) props.add(in.str());
            changed.add(new BeanChange(id, props));
        }
        int nk = in.i32();
        List<String> known = new ArrayList<>(nk);
        for (int i = 0; i < nk; i++) known.add(in.str());
        return new SpringChange(file, added, changed, known);
    }

    public static byte[] encodeSpringResult(List<BeanResult> results) throws IOException {
        Out o = new Out().i32(results.size());
        for (BeanResult r : results) o.str(r.id()).i8(r.status()).str(r.message());
        return o.done();
    }

    public static List<BeanResult> decodeSpringResult(byte[] p) throws IOException {
        In in = new In(p);
        int n = in.i32();
        List<BeanResult> list = new ArrayList<>(n);
        for (int i = 0; i < n; i++) list.add(new BeanResult(in.str(), in.i8(), in.str()));
        return list;
    }
}
