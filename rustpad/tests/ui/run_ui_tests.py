#!/usr/bin/env python3
"""Headless UI regression tests.

Each case is a script for the app's built-in input replayer (see src/debug.rs)
plus `# expect` lines. Cases run under Xvfb (X11 backend) with an isolated config dir.

Usage: tests/ui/run_ui_tests.py [name-substring ...]
Env:   RUSTPAD_BIN=path/to/rustpad   (default target/debug/rustpad)
"""
import json, os, shutil, subprocess, sys, tempfile, pathlib

ROOT = pathlib.Path(__file__).resolve().parents[2]
BIN = os.environ.get("RUSTPAD_BIN", str(ROOT / "target/debug/rustpad"))
CASES = pathlib.Path(__file__).parent / "cases"


def run_case(path: pathlib.Path) -> tuple[bool, str]:
    work = pathlib.Path(tempfile.mkdtemp(prefix="rp-ui-"))
    home = work / "home"
    home.mkdir()
    script_lines, expects, files, keep_home = [], [], [], False
    for line in path.read_text().splitlines():
        if line.startswith("# expect "):
            expects.append(line[len("# expect "):])
        elif line.startswith("# file "):
            files.append(line[len("# file "):])
        elif line.startswith("# session "):
            keep_home = True
        elif line.startswith("# ---"):
            script_lines.append(line[2:])
        elif line.startswith("#") or not line.strip():
            continue
        else:
            script_lines.append(line.replace("$OUT", str(work)))
    # materialise files: "# file name :: content with \n escapes"
    args = []
    for f in files:
        name, content = f.split(" :: ", 1)
        p = work / name
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(content.encode().decode("unicode_escape").encode("latin1"))
        args.append(str(p))
    # `# --- second run` splits the script into two app launches sharing one config dir
    runs, cur = [], []
    for l in script_lines:
        if l.strip() == "--- second run":
            runs.append(cur)
            cur = []
        else:
            cur.append(l)
    runs.append(cur)
    env = dict(os.environ)
    env.pop("WAYLAND_DISPLAY", None)
    for i, lines in enumerate(runs):
        script = work / f"script{i}.txt"
        script.write_text("\n".join(lines) + "\nexit\n")
        env.update(WINIT_UNIX_BACKEND="x11", HOME=str(home), XDG_CONFIG_HOME=str(home / ".config"),
                   LIBGL_ALWAYS_SOFTWARE="1", RUSTPAD_SCRIPT=str(script))
        try:
            r = subprocess.run(["xvfb-run", "-a", "-s", "-screen 0 1280x800x24", BIN, *(args if i == 0 or "# args-again" in path.read_text() else [])],
                               env=env, timeout=90, capture_output=True)
        except subprocess.TimeoutExpired:
            return False, "timeout"
    problems = []
    for e in expects:
        kind, _, rest = e.partition(" ")
        if kind == "dump":      # dump <file> == <python-escaped text>
            if " == " in rest:
                fname, _, want = rest.partition(" == ")
            else:
                fname, want = rest[:-3].strip(), ""
            want = want.encode().decode("unicode_escape")
            got_p = work / fname
            got = got_p.read_text() if got_p.exists() else "<missing>"
            if got != want:
                problems.append(f"dump {fname}: want {want!r} got {got!r}")
        elif kind == "bytes":   # bytes <file> == hex
            fname, _, want = rest.partition(" == ")
            got_p = work / fname
            got = got_p.read_bytes().hex() if got_p.exists() else "<missing>"
            if got != want.strip():
                problems.append(f"bytes {fname}: want {want} got {got}")
        elif kind == "state":   # state <file> key=json-value ...
            fname, _, conds = rest.partition(" ")
            sp = work / fname
            if not sp.exists():
                problems.append(f"state {fname} missing")
                continue
            st = json.loads(sp.read_text())
            for cond in conds.split(" ;; "):
                k, _, v = cond.partition("=")
                want = json.loads(v)
                if st.get(k) != want:
                    problems.append(f"state {fname}.{k}: want {want!r} got {st.get(k)!r}")
        elif kind in ("present", "absent"):   # present|absent <file> :: <text> (checked in the raw file bytes)
            fname, _, text = rest.partition(" :: ")
            got_p = work / fname
            raw = got_p.read_bytes() if got_p.exists() else b""
            if (text.encode() in raw) != (kind == "present"):
                problems.append(f"{kind} {fname}: {text!r}")
        elif kind == "exists":
            if not (work / rest.strip()).exists():
                problems.append(f"missing {rest}")
    ok = not problems
    if ok or not os.environ.get("KEEP"):
        shutil.rmtree(work, ignore_errors=True)
    return ok, "; ".join(problems)


def main():
    if not shutil.which("xvfb-run"):
        print("xvfb-run not found; skipping UI tests")
        return 0
    filt = sys.argv[1:]
    cases = sorted(CASES.glob("*.txt"))
    failed = 0
    for c in cases:
        if filt and not any(f in c.name for f in filt):
            continue
        ok, msg = run_case(c)
        print(("PASS " if ok else "FAIL ") + c.name + ("" if ok else "  -> " + msg))
        failed += 0 if ok else 1
    print(f"\n{failed} failed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
