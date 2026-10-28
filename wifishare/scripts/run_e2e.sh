#!/usr/bin/env bash
# Pairs a throwaway server and client on this machine (port 24990), then runs e2e.sh on them.
# Usage: scripts/run_e2e.sh [path/to/wifishare]   (default: target/release/wifishare)
set -eu
cd "$(dirname "$0")/.."
BIN=$(realpath "${1:-target/release/wifishare}")
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/S" "$T/C" "$T/SC" "$T/CC"
for d in S C; do printf 'port = 24990\nname = "test-%s"\n[network]\ntimeout_ms = 3000\n' "$d" >"$T/$d/config.toml"; done
python3 -I scripts/pair_test.py "$BIN" "$T/S" "$T/C"
bash scripts/e2e.sh "$BIN" "$T"
