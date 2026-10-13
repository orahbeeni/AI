#!/bin/sh
# Creates a repository with many branches, tags and a remote, for trying out
# hiding and showing branches in the graph ("Viewing X/Y" in the sidebar).
#
#   scripts/make-branches-repo.sh [directory]      (default: /tmp/gitgoing-branches)
set -eu

DIR="${1:-/tmp/gitgoing-branches}"
rm -rf "$DIR"
mkdir -p "$DIR"
cd "$DIR"

git init -q --bare remote.git
git clone -q remote.git work 2>/dev/null
cd work
git config user.name "Test User"
git config user.email "test@example.com"
git checkout -q -b main

commit() {
  f=$(printf '%s' "$1" | tr -c 'A-Za-z0-9' '_')
  printf '%s\n' "$1" > "$f.txt"
  git add .
  git commit -q -m "$1"
}

commit "Initial commit"
commit "Set up project"
git tag v1.0

for name in feature/login feature/search feature/export bugfix/crash bugfix/typo release/1.0 experiment; do
  git checkout -q -b "$name" main
  commit "$name: first change"
  commit "$name: second change"
  git checkout -q main
  commit "main moves on after $name"
done
git tag v1.1

git push -q origin --all
git push -q origin --tags
git checkout -q main

echo "Created $DIR (open $DIR/work)"
git branch -a | head -30
echo
echo "Try: hover a branch in the sidebar and click its eye; right-click for 'Show only this'; 'Show all' resets."
