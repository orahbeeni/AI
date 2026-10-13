#!/bin/sh
# Creates a repository where squashing NON-consecutive commits is predicted to conflict.
#
#   scripts/make-squash-conflict-repo.sh [directory]   (default: /tmp/gitgoing-squash-conflict)
#
# Select "Create settings" and "Enable debug mode" (Cmd/Ctrl-click), skipping the commit
# between them: the middle commit edits the same lines, so moving it will conflict.
# Selecting "Create settings" and "Add README" instead (different files) is conflict-free.
set -eu

DIR="${1:-/tmp/gitgoing-squash-conflict}"
rm -rf "$DIR"
mkdir -p "$DIR"
cd "$DIR"
git init -q
git config user.name "Test User"
git config user.email "test@example.com"
git checkout -q -b main

printf 'base\n' > base.txt
git add . && git commit -q -m "Initial commit"

printf 'debug=false\n' > settings.txt
git add . && git commit -q -m "Create settings"

printf 'debug=false\ntimeout=30\n' > settings.txt
git add . && git commit -q -m "Add timeout setting"

printf 'debug=true\ntimeout=30\n' > settings.txt
git add . && git commit -q -m "Enable debug mode"

printf '# Readme\n' > README.md
git add . && git commit -q -m "Add README"

echo "Created $DIR:"
git log --oneline
echo
echo "Open it:  gitgoing \"$DIR\""
