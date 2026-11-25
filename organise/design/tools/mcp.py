"""Tiny stdio MCP client for Sketchframe: mcp.py <project.json> <calls.json> [outdir]
calls.json = [{"name": tool, "arguments": {...}, "save": "file.png"?}, ...]"""
import json, subprocess, sys, base64, os
proj, calls_file = sys.argv[1], sys.argv[2]
outdir = sys.argv[3] if len(sys.argv) > 3 else "."
calls = json.load(open(calls_file))
p = subprocess.Popen([os.path.expanduser("~/.local/bin/sketchframe"), "--mcp", "--file", proj],
                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
def rpc(i, method, params):
    p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": i, "method": method, "params": params}) + "\n"); p.stdin.flush()
    return json.loads(p.stdout.readline())
rpc(0, "initialize", {"protocolVersion": "2025-06-18"})
for i, c in enumerate(calls, 1):
    r = rpc(i, "tools/call", {"name": c["name"], "arguments": c.get("arguments", {})})
    res = r.get("result", {})
    for item in res.get("content", []):
        if item["type"] == "text":
            t = item["text"]
            print(f"[{c['name']}] {'ERROR ' if res.get('isError') else ''}{t if len(t) < 4000 or c.get('full') else t[:4000] + '…'}")
        elif item["type"] == "image":
            fn = os.path.join(outdir, c.get("save", f"img{i}.png"))
            open(fn, "wb").write(base64.b64decode(item["data"]))
            print(f"[{c['name']}] image -> {fn}")
    if "error" in r: print("RPC error", r["error"])
p.stdin.close(); p.wait(timeout=30)
