#!/bin/sh
# Creates a working repository whose branch is already pushed to a local "remote",
# for trying out squash/amend followed by a force push.
#
#   scripts/make-force-push-repo.sh [directory]    (default: /tmp/gitgoing-forcepush)
#
# Then open <directory>/work in gitgoing, squash or amend a pushed commit, and push.
set -eu

DIR="${1:-/tmp/gitgoing-forcepush}"
rm -rf "$DIR"
mkdir -p "$DIR"
cd "$DIR"

git init -q --bare remote.git
git clone -q remote.git work 2>/dev/null
cd work
git config user.name "Test User"
git config user.email "test@example.com"
git checkout -q -b main

n=0
for msg in "Initial commit" "Add search" "Fix search bug" "Add export" "Fix export bug" "Update docs"; do
  n=$((n + 1))
  printf '%s\n' "$msg" > "file$n.txt"
  git add .
  git commit -q -m "$msg"
done
git push -q -u origin main

echo "Created $DIR"
echo "  work/        your clone (open this in gitgoing)"
echo "  remote.git   the 'server' it pushes to"
echo
git log --oneline
echo
echo "Open it:  gitgoing \"$DIR/work\""
echo "Try: select 'Add search' + 'Fix search bug' and squash them, then use Push (it will be rejected),"
echo "     then the Force push button, or the Push drop-down > Force push (with lease)."
