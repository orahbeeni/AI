"""Drives `wifishare server` and `wifishare pair` through real terminals and approves both."""
import os, pty, re, select, signal, sys, time

BIN, S, C = sys.argv[1], sys.argv[2], sys.argv[3]
bufs = {}


def spawn(args):
    pid, fd = pty.fork()
    if pid == 0:
        os.execv(BIN, [BIN] + args)
    bufs[fd] = b""
    return pid, fd


def wait_for(fd, pattern, timeout=15):
    end = time.time() + timeout
    while time.time() < end:
        m = re.search(pattern.encode(), bufs[fd])
        if m:
            return m
        r, _, _ = select.select([fd], [], [], 0.2)
        if r:
            try:
                bufs[fd] += os.read(fd, 4096)
            except OSError:
                break
    sys.exit(f"timeout waiting for {pattern!r}; got:\n{bufs[fd].decode(errors='replace')}")


spid, sfd = spawn(["--config-dir", S, "server", "--no-input"])
wait_for(sfd, "listening on port")
cpid, cfd = spawn(["--config-dir", C, "pair", "127.0.0.1"])
client_code = wait_for(cfd, r"Code: (\d{3} \d{3})").group(1).decode()
server_code = wait_for(sfd, r"Code: (\d{3} \d{3})").group(1).decode()
print("client code", client_code, "server code", server_code)
assert client_code == server_code, "codes differ"
wait_for(cfd, r"Is it the same\? \[y/N\]")
os.write(cfd, b"y\n")
wait_for(sfd, r"if they match\? \[y/N\]")
os.write(sfd, b"y\n")
wait_for(cfd, "Paired with")
wait_for(sfd, "paired with")
print("PAIRED OK")
os.kill(spid, signal.SIGINT)
os.waitpid(spid, 0)
os.waitpid(cpid, 0)
