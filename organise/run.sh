#!/bin/bash
# Builds Organise if anything changed since the last build, then starts it.
# Your data lives in ~/Organise. To try it on a separate folder: ORGANISE_DIR=/some/folder ./run.sh
set -e
cd "$(dirname "$0")"
BIN=target/release/organise
if [ ! -x "$BIN" ] || [ -n "$(find app/src app/src-tauri/src app/src-tauri/tauri.conf.json core/src -newer "$BIN" -print -quit)" ]; then
  [ -d app/node_modules ] || (cd app && npm install)
  (cd app && npx tauri build --no-bundle)
fi
exec "$BIN" "$@"
