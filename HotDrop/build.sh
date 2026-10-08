#!/usr/bin/env sh
# Thin wrapper; the build itself is plain Java so it behaves the same on Linux, macOS and Windows (java Build.java).
cd "$(dirname "$0")" && exec java Build.java "$@"
