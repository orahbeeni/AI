#!/bin/sh
# Creates a throwaway git repository that is stopped in the middle of a merge
# with several kinds of conflicts, for trying out the conflict editor.
#
#   scripts/make-conflict-repo.sh [directory]      (default: /tmp/gitgoing-conflict)
#
# Then open it in gitgoing and use "Resolve…" on each conflicted file.
set -eu

DIR="${1:-/tmp/gitgoing-conflict}"
rm -rf "$DIR"
mkdir -p "$DIR"
cd "$DIR"

git init -q
git config user.name "Test User"
git config user.email "test@example.com"
git checkout -q -b main

# --- base commit ----------------------------------------------------------
cat > app.txt <<'TXT'
title: My App
version: 1.0

greeting: hello

farewell: goodbye

colour: red

size: medium

footer: thanks
TXT
printf 'only on main\n' > deleted-by-feature.txt
printf 'shared notes\n' > notes.txt
git add .
git commit -q -m "base"

# --- feature branch -------------------------------------------------------
git checkout -q -b feature
cat > app.txt <<'TXT'
title: My Great App
version: 1.0

greeting: hi there

farewell: goodbye

colour: blue

size: medium

footer: thanks for visiting
TXT
git rm -q deleted-by-feature.txt
printf 'feature edit\n' > notes.txt
printf 'created on feature\nline two from feature\n' > both-added.txt
git add .
git commit -q -m "feature: change title, greeting, colour, footer; delete a file"

# --- main branch ----------------------------------------------------------
git checkout -q main
cat > app.txt <<'TXT'
title: Our App
version: 1.0

greeting: hello

farewell: see you later

colour: green

size: large

footer: thanks
TXT
printf 'main edit to the file feature deleted\n' > deleted-by-feature.txt
printf 'created on main\nline two from main\nline three from main\n' > both-added.txt
printf 'main edit\n' > notes.txt
git add .
git commit -q -m "main: change title, farewell, colour, size; edit the file feature deleted"

# --- merge (stops with conflicts) ------------------------------------------
git merge feature >/dev/null 2>&1 || true

echo "Created $DIR"
git -C "$DIR" status --short
cat <<EOM

Open it with:   gitgoing "$DIR"
or straight into the editor:   gitgoing "$DIR" --resolve app.txt

What to expect:
  app.txt              2 conflicts (title, colour) plus changes that merge on their own
  notes.txt            both sides edited the same line
  both-added.txt       both sides added the file with different content
  deleted-by-feature.txt   deleted on one side, edited on the other
EOM
