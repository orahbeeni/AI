package hotdrop.daemon;

import hotdrop.protocol.Wire;
import hotdrop.protocol.Wire.AgentInfo;
import hotdrop.protocol.Wire.BatchResult;
import hotdrop.protocol.Wire.ClassEntry;
import hotdrop.protocol.Wire.Frame;
import hotdrop.protocol.Wire.LoaderInfo;

import java.io.BufferedInputStream;
import java.io.BufferedOutputStream;
import java.io.Closeable;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.IOException;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.Socket;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.Properties;
import java.util.stream.Stream;

/** Connection to the agent inside the Hybris JVM. Reconnects on demand, because the server comes and goes. */
final class AgentLink implements Closeable {
    private final Path agentsDir;
    private final Path platformHome;

    private Socket socket;
    private DataInputStream in;
    private DataOutputStream out;
    private AgentInfo info;
    private int generation;
    private List<LoaderInfo> inventory;

    AgentLink(Path agentsDir, Path platformHome) {
        this.agentsDir = agentsDir;
        this.platformHome = platformHome;
    }

    synchronized boolean connected() {
        return socket != null;
    }

    synchronized AgentInfo info() {
        return info;
    }

    /** Incremented on every new connection; callers use it to drop caches built from a previous JVM. */
    synchronized int generation() {
        return generation;
    }

    /** Connects if needed and checks liveness. Returns true when an agent is available. */
    synchronized boolean poll() {
        if (socket != null) {
            try {
                Wire.write(out, Wire.PING, new byte[0]);
                Frame f = Wire.read(in);
                if (f.type() == Wire.PONG) return true;
            } catch (IOException e) {
                // fall through: connection is dead
            }
            Log.info("agent disconnected (server stopped?)");
            drop();
        }
        return tryConnect();
    }

    private boolean tryConnect() {
        if (!Files.isDirectory(agentsDir)) return false;
        List<Path> files = new ArrayList<>();
        try (Stream<Path> s = Files.list(agentsDir)) {
            s.filter(p -> p.toString().endsWith(".properties")).forEach(files::add);
        } catch (IOException e) {
            return false;
        }
        List<Properties> candidates = new ArrayList<>();
        for (Path f : files) {
            Properties p = new Properties();
            try (var r = Files.newBufferedReader(f)) {
                p.load(r);
                long pid = Long.parseLong(p.getProperty("pid", "-1"));
                if (ProcessHandle.of(pid).map(ProcessHandle::isAlive).orElse(false)) {
                    if (platformHome == null || sameDir(platformHome, p.getProperty("platformHome"))) candidates.add(p);
                } else {
                    Files.deleteIfExists(f);
                }
            } catch (IOException | RuntimeException e) {
                Log.debug("ignoring %s: %s", f, e.getMessage());
            }
        }
        candidates.sort(Comparator.comparingLong((Properties p) -> Long.parseLong(p.getProperty("startMillis", "0"))).reversed());
        for (Properties p : candidates) {
            if (connect(p)) return true;
        }
        return false;
    }

    private static boolean sameDir(Path expected, String actual) {
        if (actual == null || actual.isEmpty()) return false;
        try {
            return Files.isSameFile(expected, Path.of(actual));
        } catch (IOException | RuntimeException e) {
            return false;
        }
    }

    private boolean connect(Properties p) {
        Socket s = new Socket();
        try {
            s.connect(new InetSocketAddress(InetAddress.getLoopbackAddress(), Integer.parseInt(p.getProperty("port"))), 1500);
            s.setTcpNoDelay(true);
            s.setSoTimeout(30_000);
            DataInputStream i = new DataInputStream(new BufferedInputStream(s.getInputStream()));
            DataOutputStream o = new DataOutputStream(new BufferedOutputStream(s.getOutputStream()));
            Wire.write(o, Wire.HELLO, Wire.encodeHello(p.getProperty("token")));
            Frame f = Wire.read(i);
            if (f.type() != Wire.HELLO_OK) {
                s.close();
                return false;
            }
            this.socket = s;
            this.in = i;
            this.out = o;
            this.info = Wire.decodeAgentInfo(f.payload());
            this.inventory = null;
            this.generation++;
            Log.info("agent connected: pid %d, %s %s, enhanced redefinition %s", info.pid(), info.vendor(),
                    info.javaVersion(), info.enhancedRedefine() ? "ON" : "off");
            return true;
        } catch (IOException | RuntimeException e) {
            try {
                s.close();
            } catch (IOException ignored) {
                // closing a failed socket
            }
            return false;
        }
    }

    private void drop() {
        try {
            if (socket != null) socket.close();
        } catch (IOException ignored) {
            // already dead
        }
        socket = null;
        info = null;
        inventory = null;
    }

    synchronized List<LoaderInfo> inventory() {
        if (socket == null) return null;
        if (inventory != null) return inventory;
        try {
            Wire.write(out, Wire.INVENTORY_REQ, new byte[0]);
            Frame f = Wire.read(in);
            if (f.type() != Wire.INVENTORY) throw new IOException("unexpected reply " + f.type());
            inventory = Wire.decodeInventory(f.payload());
            return inventory;
        } catch (IOException e) {
            Log.warn("agent inventory failed: %s", e.getMessage());
            drop();
            return null;
        }
    }

    synchronized void refreshInventory() {
        inventory = null;
    }

    /** Sends one atomic batch. Returns null when no agent is reachable. */
    synchronized BatchResult redefine(long batchId, List<ClassEntry> entries) {
        if (socket == null && !tryConnect()) return null;
        try {
            Wire.write(out, Wire.REDEFINE, Wire.encodeRedefine(batchId, entries));
            Frame f = Wire.read(in);
            if (f.type() != Wire.RESULT) throw new IOException("unexpected reply " + f.type());
            return Wire.decodeResult(f.payload());
        } catch (IOException e) {
            Log.warn("agent connection lost during swap: %s", e.getMessage());
            drop();
            return null;
        }
    }

    /** Asks the server to apply bean changes from a Spring XML file. Null when no agent is reachable. */
    synchronized List<Wire.BeanResult> spring(Wire.SpringChange change) {
        if (socket == null && !tryConnect()) return null;
        try {
            Wire.write(out, Wire.SPRING, Wire.encodeSpring(change));
            Frame f = Wire.read(in);
            if (f.type() == Wire.ERROR) {
                return List.of(new Wire.BeanResult("(agent)", Wire.BEAN_FAILED,
                        "the agent does not support Spring changes; rebuild it with the same version as the daemon"));
            }
            if (f.type() != Wire.SPRING_RESULT) throw new IOException("unexpected reply " + f.type());
            return Wire.decodeSpringResult(f.payload());
        } catch (IOException e) {
            Log.warn("agent connection lost while applying Spring changes: %s", e.getMessage());
            drop();
            return null;
        }
    }

    /** Prints a line in the server's console (the Hybris log). Best effort: no agent, no message. */
    synchronized void notice(String text) {
        if (socket == null) return;
        try {
            Wire.write(out, Wire.NOTICE, new Wire.Out().str(text).done());
            Wire.read(in);
        } catch (IOException e) {
            Log.debug("notice failed: %s", e.getMessage());
            drop();
        }
    }

    @Override
    public synchronized void close() {
        drop();
    }
}
