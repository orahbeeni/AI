package hotdrop.daemon;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStreamReader;
import java.io.PrintWriter;
import java.net.InetAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.nio.file.attribute.PosixFilePermissions;
import java.security.MessageDigest;
import java.security.SecureRandom;
import java.util.Properties;

/** Loopback command socket used by the CLI and the IDE plugin: STATUS, FLUSH, PAUSE, RESUME, RESCAN, STOP. */
final class Control {
    private final Daemon daemon;
    private final ServerSocket server;
    private final String token;
    private final Path file;

    Control(Daemon daemon, Path home) throws IOException {
        this.daemon = daemon;
        this.server = new ServerSocket(0, 8, InetAddress.getLoopbackAddress());
        byte[] raw = new byte[24];
        new SecureRandom().nextBytes(raw);
        StringBuilder sb = new StringBuilder();
        for (byte b : raw) sb.append(String.format("%02x", b));
        this.token = sb.toString();
        Files.createDirectories(home);
        this.file = home.resolve("daemon.properties");
        Properties p = new Properties();
        p.setProperty("port", String.valueOf(server.getLocalPort()));
        p.setProperty("token", token);
        p.setProperty("pid", String.valueOf(ProcessHandle.current().pid()));
        Path tmp = Files.createTempFile(home, "daemon", ".tmp");
        try {
            try {
                Files.setPosixFilePermissions(tmp, PosixFilePermissions.fromString("rw-------"));
            } catch (UnsupportedOperationException ignored) {
                // not POSIX
            }
            try (var w = Files.newBufferedWriter(tmp)) {
                p.store(w, "HotDrop daemon");
            }
            Files.move(tmp, file, StandardCopyOption.REPLACE_EXISTING);
        } finally {
            Files.deleteIfExists(tmp);
        }
        Thread t = new Thread(this::loop, "hotdrop-control");
        t.setDaemon(true);
        t.start();
    }

    void cleanup() {
        try {
            server.close();
            Files.deleteIfExists(file);
        } catch (IOException ignored) {
            // exiting
        }
    }

    private void loop() {
        while (!server.isClosed()) {
            try (Socket s = server.accept()) {
                s.setSoTimeout(3000);
                BufferedReader in = new BufferedReader(new InputStreamReader(s.getInputStream(), StandardCharsets.UTF_8));
                PrintWriter out = new PrintWriter(s.getOutputStream(), true, StandardCharsets.UTF_8);
                String line = in.readLine();
                if (line == null) continue;
                int sp = line.indexOf(' ');
                String given = sp < 0 ? line : line.substring(0, sp);
                if (!MessageDigest.isEqual(given.getBytes(StandardCharsets.UTF_8), token.getBytes(StandardCharsets.UTF_8))) {
                    out.println("unauthorised");
                    continue;
                }
                String cmd = sp < 0 ? "" : line.substring(sp + 1).trim();
                out.print(handle(cmd));
                out.flush();
            } catch (IOException e) {
                if (!server.isClosed()) Log.debug("control: %s", e.getMessage());
            }
        }
    }

    private String handle(String cmd) {
        String[] parts = cmd.split(" ", 2);
        switch (parts[0].toUpperCase()) {
            case "STATUS":
                return daemon.engine.status();
            case "FLUSH":
                daemon.submit(parts.length > 1 ? Path.of(parts[1]) : null, true);
                return "flushed\n";
            case "PAUSE":
                daemon.pause(true);
                return "paused\n";
            case "RESUME":
                daemon.pause(false);
                return "resumed\n";
            case "RESCAN":
                daemon.rescan();
                return "rescanning\n";
            case "STOP":
                daemon.stop();
                return "stopping\n";
            default:
                return "unknown command: " + cmd + "\n";
        }
    }
}
