#!/bin/sh
# Creates a small repository with a linear history, for trying out selecting
# several commits and squashing them (and interactive rebase).
#
#   scripts/make-history-repo.sh [directory]      (default: /tmp/gitgoing-history)
set -eu

DIR="${1:-/tmp/gitgoing-history}"
rm -rf "$DIR"
mkdir -p "$DIR"
cd "$DIR"
git init -q
git config user.name "Test User"
git config user.email "test@example.com"
git checkout -q -b main

n=0
for msg in "Initial commit" "Add login page" "Fix typo in login page" "Add logout button" \
           "Fix logout button colour" "Add profile page" "Fix profile page layout" "Update README"; do
  n=$((n + 1))
  printf '%s\n' "$msg" > "file$n.txt"
  git add .
  git commit -q -m "$msg"
done

echo "Created $DIR with $n commits:"
git log --oneline
echo
echo "Open it:  gitgoing \"$DIR\""
echo "Then Cmd/Ctrl-click (or Shift-click) several consecutive commits, e.g. the three about the login/logout"
echo "buttons, and use \"Squash into one commit…\" in the right panel."
