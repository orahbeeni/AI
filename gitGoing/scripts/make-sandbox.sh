#!/bin/sh
# Creates a throwaway project for trying out gitgoing: some history, branches, a tag,
# a stash, a local "origin" to push/pull/fetch against, uncommitted changes of every
# kind, and a merge stopped on a conflict (clash/b into clash/a).
#
#   scripts/make-sandbox.sh [directory]   (default: ./sandbox)
#
# Run it again at any time to start over. The remote lives next to it as <directory>-origin.git.
set -eu

DIR="${1:-./sandbox}"
rm -rf "$DIR" "$DIR-origin.git"
mkdir -p "$DIR"
DIR="$(cd "$DIR" && pwd)"
ORIGIN="$DIR-origin.git"

git init -q --bare "$ORIGIN"
cd "$DIR"
git init -q -b main
git config user.name "Omar"
git config user.email "omar@example.com"
git config core.autocrlf false
git remote add origin "$ORIGIN"

n=0
# commit <message> [author name] : commit everything, a day later than the previous one
commit() {
    n=$((n + 1))
    d="2026-0$((1 + n / 28))-$(printf '%02d' $((1 + n % 28)))T10:$(printf '%02d' $((n % 60))):00"
    who="${2:-Omar}"
    git add -A
    GIT_AUTHOR_NAME="$who" GIT_AUTHOR_EMAIL="$(echo "$who" | tr 'A-Z ' 'a-z.')@example.com" \
    GIT_AUTHOR_DATE="$d" GIT_COMMITTER_DATE="$d" git commit -q -m "$1"
}

# --- main: a small web shop ----------------------------------------------------
mkdir -p src/shop src/web docs tests
cat > README.md <<'TXT'
# Sandbox shop

A made-up project for trying out gitgoing.
TXT
printf 'target/\n*.log\n.env\n' > .gitignore
commit "Initial commit"

cat > src/shop/Cart.java <<'TXT'
package shop;

import java.util.ArrayList;
import java.util.List;

public class Cart {
    private final List<Item> items = new ArrayList<>();

    public void add(Item item) {
        items.add(item);
    }

    public int count() {
        return items.size();
    }
}
TXT
commit "Add Cart"

cat > src/shop/Item.java <<'TXT'
package shop;

public record Item(String sku, String name, long priceCents) {}
TXT
commit "Add Item"

cat > src/web/index.html <<'TXT'
<!doctype html>
<html>
<head><title>Sandbox shop</title><link rel="stylesheet" href="style.css"></head>
<body>
  <h1>Sandbox shop</h1>
  <ul id="items"></ul>
  <script src="app.js"></script>
</body>
</html>
TXT
cat > src/web/style.css <<'TXT'
body {
    font-family: sans-serif;
    margin: 2em;
    color: #333333;
}
h1 {
    border-bottom: 1px solid #cccccc;
}
TXT
cat > src/web/app.js <<'TXT'
const items = [];

function render() {
    const ul = document.getElementById('items');
    ul.innerHTML = items.map(i => `<li>${i.name}</li>`).join('');
}

render();
TXT
commit "Add the web page" "Sam Ndlovu"

cat > docs/api.md <<'TXT'
# API

| Method | Path        | Description          |
|--------|-------------|----------------------|
| GET    | /items      | List all items       |
| POST   | /cart       | Add an item to cart  |
TXT
commit "Document the API" "Sam Ndlovu"

cat >> src/shop/Cart.java <<'TXT'
TXT
sed -i.bak 's/    public int count() {/    public long totalCents() {\n        return items.stream().mapToLong(Item::priceCents).sum();\n    }\n\n    public int count() {/' src/shop/Cart.java
rm -f src/shop/Cart.java.bak
commit "Cart: total price"

git tag -a v0.1 -m "First release"

# --- feature/login: merged into main with a merge commit -----------------------
git checkout -q -b feature/login
cat > src/shop/Login.java <<'TXT'
package shop;

public class Login {
    public boolean check(String user, String password) {
        return !user.isBlank() && password.length() >= 8;
    }
}
TXT
commit "Login: basic check" "Thandi Mokoena"
printf '\nSee also: login rules in Login.java\n' >> docs/api.md
commit "Login: mention in the docs" "Thandi Mokoena"
git checkout -q main
n=$((n + 1))
GIT_COMMITTER_DATE="2026-02-10T12:00:00" git merge -q --no-ff feature/login -m "Merge branch 'feature/login'"

# --- feature/search: not merged, a few commits ahead ---------------------------
git checkout -q -b feature/search
cat > src/shop/Search.java <<'TXT'
package shop;

import java.util.List;

public class Search {
    public List<Item> find(List<Item> all, String text) {
        return all.stream().filter(i -> i.name().toLowerCase().contains(text.toLowerCase())).toList();
    }
}
TXT
commit "Search: by name"
sed -i.bak 's/const items = \[\];/const items = [];\nlet query = "";/' src/web/app.js && rm -f src/web/app.js.bak
commit "Search: remember the query in the page"
printf '| GET    | /search     | Find items by name   |\n' >> docs/api.md
commit "Search: document it"

# --- bugfix/typo: one commit on its own branch ---------------------------------
git checkout -q main
git checkout -q -b bugfix/typo
sed -i.bak 's/A made-up project/A made-up sample project/' README.md && rm -f README.md.bak
commit "Fix README wording" "Sam Ndlovu"
git checkout -q main

# --- a release branch and a few more commits on main ---------------------------
git branch release/1.0
cat > tests/CartTest.java <<'TXT'
package shop;

class CartTest {
    void emptyCartHasNoItems() {
        assert new Cart().count() == 0;
    }
}
TXT
commit "Add a first test"
printf '.idea/\n.vscode/\n' >> .gitignore
commit "Ignore editor folders"

# --- files the clash branches will edit differently ------------------------------
cat > src/shop/OrderService.java <<'TXT'
package shop;

import java.util.List;

public class OrderService {

    private static final int MAX_ITEMS = 50;

    public Order create(Customer customer, List<Item> items) {
        validate(items);
        Order order = new Order(customer);
        order.addAll(items);
        return order;
    }

    private void validate(List<Item> items) {
        if (items.size() > MAX_ITEMS) {
            throw new IllegalArgumentException("Too many items");
        }
    }

    public void cancel(Order order) {
        order.setStatus(Status.CANCELLED);
    }
}
TXT
cat > config.json <<'TXT'
{
  "name": "sandbox",
  "port": 8080,
  "timeout": 30,
  "features": ["search"]
}
TXT
cat > docs/report.sql <<'TXT'
-- Report queries
SELECT id, name FROM customers WHERE active = 1;
SELECT id, total FROM orders WHERE status = 'OPEN';
SELECT COUNT(*) FROM audit_log;
TXT
# a tall file, for the vertical scroll bars: 90 numbered lines
i=1; : > docs/tall.txt
while [ $i -le 90 ]; do printf 'line %02d: nothing special happens on this line\n' $i >> docs/tall.txt; i=$((i + 1)); done
printf 'line one\r\nline two\r\nline three\r\n' > docs/windows-crlf.txt
commit "Add order service, config, report queries and a tall file"

# --- two branches that conflict when merged ------------------------------------
git checkout -q -b clash/a
cat > src/web/style.css <<'TXT'
body {
    font-family: 'Georgia', 'Palatino Linotype', 'Book Antiqua', Palatino, 'Times New Roman', 'DejaVu Serif', 'Liberation Serif', 'Noto Serif', 'Source Serif Pro', 'Iowan Old Style', 'Apple Garamond', Baskerville, serif;
    margin: 1em;
    color: #111111;
}
h1 {
    border-bottom: 1px solid #cccccc;
}
TXT
sed -i.bak 's/"Sandbox shop"//' src/web/index.html && rm -f src/web/index.html.bak
cat > src/shop/OrderService.java <<'TXT'
package shop;

import java.util.List;
import java.util.Objects;

public class OrderService {

    private static final int MAX_ITEMS = 75;

    public Order create(Customer customer, List<Item> items) {
        Objects.requireNonNull(customer, "customer");
        validate(items);
        Order order = new Order(customer);
        order.addAll(items);
        return order;
    }

    private void validate(List<Item> items) {
        if (items.size() > MAX_ITEMS) {
            throw new TooManyItemsException(items.size(), MAX_ITEMS);
        }
    }

    public void cancel(Order order) {
        order.setStatus(Status.CANCELLED);
        audit.log("cancelled " + order.getId());
    }
}
TXT
cat > config.json <<'TXT'
{
  "name": "sandbox",
  "port": 8443,
  "timeout": 30,
  "features": ["search", "audit"]
}
TXT
cat > docs/report.sql <<'TXT'
-- Report queries
SELECT id, name, email FROM customers WHERE active = 1;
SELECT id, total FROM orders WHERE status = 'OPEN';
SELECT COUNT(*) FROM audit_log WHERE actor IS NOT NULL;
TXT
sed -i.bak -e 's/^line 12: .*/line 12: edited on clash\/a/' -e 's/^line 13: .*/line 13: also edited on clash\/a/' -e 's/^line 45: .*/line 45: edited on clash\/a/' -e 's/^line 78: .*/line 78: edited on clash\/a/' docs/tall.txt
printf 'line one\r\nline TWO from a\r\nline three\r\n' > docs/windows-crlf.txt
rm -f docs/*.bak
commit "Clash A: serif look" "Thandi Mokoena"
git checkout -q main
git checkout -q -b clash/b
cat > src/web/style.css <<'TXT'
body {
    font-family: 'SFMono-Regular', Menlo, Monaco, Consolas, 'Liberation Mono', 'Courier New', 'DejaVu Sans Mono', 'Ubuntu Mono', 'Fira Code', 'JetBrains Mono', 'Source Code Pro', 'Roboto Mono', 'IBM Plex Mono', 'Cascadia Code', 'Hack', monospace; /* picked for wide screens, with a very long explanation that keeps going and going so that this line is much wider than the window */
    margin: 3em;
    color: #222222;
}
h1 {
    border-bottom: 3px solid #cccccc;
}
TXT
cat > src/shop/OrderService.java <<'TXT'
package shop;

import java.util.List;

public class OrderService {

    private static final int MAX_ITEMS = 100;

    public Order create(Customer customer, List<Item> items) {
        validate(items);
        Order order = new Order(customer);
        order.addAll(items);
        order.setSource("web");
        return order;
    }

    private void validate(List<Item> items) {
        if (items.isEmpty()) {
            throw new IllegalArgumentException("No items");
        }
        if (items.size() > MAX_ITEMS) {
            throw new IllegalArgumentException("Too many items: " + items.size());
        }
    }
}
TXT
cat > config.json <<'TXT'
{
  "name": "sandbox",
  "port": 9090,
  "timeout": 30,
  "features": ["search", "export"]
}
TXT
cat > docs/report.sql <<'TXT'
-- Report queries
SELECT c.id, c.name, c.email, c.phone, c.created_at, c.updated_at, a.street, a.city, a.postal_code, a.country, o.id AS order_id, o.total, o.currency, o.status, o.placed_at FROM customers c LEFT JOIN addresses a ON a.customer_id = c.id LEFT JOIN orders o ON o.customer_id = c.id WHERE c.active = 1 AND o.status IN ('OPEN', 'PAID', 'SHIPPED') ORDER BY o.placed_at DESC;
SELECT id, total FROM orders WHERE status = 'OPEN';
SELECT COUNT(*) FROM audit_log WHERE created_at > NOW() - INTERVAL '30 days' AND actor IS NOT NULL AND action IN ('LOGIN', 'LOGOUT', 'CANCEL', 'REFUND', 'EXPORT') GROUP BY actor, action HAVING COUNT(*) > 5 ORDER BY 1 DESC;
TXT
sed -i.bak -e 's/^line 12: .*/line 12: edited on clash\/b/' -e 's/^line 14: .*/line 14: edited on clash\/b/' -e 's/^line 45: .*/line 45: edited on clash\/b, differently/' -e 's/^line 60: .*/line 60: edited on clash\/b/' docs/tall.txt
printf 'line one\r\nline TWO from b\r\nline three\r\n' > docs/windows-crlf.txt
rm -f docs/*.bak
commit "Clash B: monospace look" "Sam Ndlovu"
git checkout -q main

# --- remote: push, then make origin differ from local ---------------------------
git push -q -u origin main feature/login feature/search release/1.0 --tags 2>/dev/null
# someone else pushes to main (a fetch/pull has something to bring in) ...
other="$(mktemp -d)"
git clone -q "$ORIGIN" "$other/c"
(
    cd "$other/c"
    git config user.name "Thandi Mokoena"; git config user.email "thandi@example.com"
    printf '\n## Contributing\n\nOpen a pull request.\n' >> README.md
    git commit -q -am "Add contributing notes"
    git push -q origin main
)
rm -rf "$other"
# ... and local main is ahead too (a push has something to send).
printf 'node_modules/\n' >> .gitignore
commit "Ignore node_modules"
git fetch -q

# --- a stash ---------------------------------------------------------------------
printf '\n/* work in progress */\n' >> src/web/style.css
git stash push -q -m "half-done styling"

# --- stop in the middle of a merge: clash/b into clash/a ------------------
git checkout -q clash/a
git merge clash/b >/dev/null 2>&1 || true

# --- uncommitted changes of every kind -----------------------------------------
# modified, with two separate hunks (try staging them one at a time)
sed -i.bak 's/        items.add(item);/        if (item == null) {\n            throw new IllegalArgumentException("item");\n        }\n        items.add(item);/' src/shop/Cart.java
sed -i.bak 's/    public int count() {/    \/** How many items are in the cart. *\/\n    public int count() {/' src/shop/Cart.java
rm -f src/shop/Cart.java.bak
# modified and already staged
printf '\n| DELETE | /cart/{sku} | Remove an item      |\n' >> docs/api.md
git add docs/api.md
# deleted
git rm -q --cached src/web/app.js && rm src/web/app.js
# new, not tracked
printf 'TODO\n- checkout page\n- payments\n' > TODO.txt
mkdir -p scratch
# a very long line, for the horizontal scrolling
{ printf 'start:'; i=0; while [ $i -lt 60 ]; do printf ' column%03d-abcdefghij' $i; i=$((i + 1)); done; printf ' :end\n'; } > scratch/long-line.txt
rm -f ./*.bak

echo "Created $DIR (remote: $ORIGIN)"
echo
git status --short
cat <<EOM

Open it:   gitgoing "$DIR"

What is in there:
  history      ~12 commits by Omar, Sam Ndlovu and Thandi Mokoena, a merge commit, tag v0.1
  branches     main, feature/login (merged), feature/search (3 ahead), bugfix/typo,
               release/1.0, clash/a + clash/b (you start on clash/a)
  remote       origin: main is 1 behind and 1 ahead (fetch / pull / push), 3 branches pushed
  stash        "half-done styling"
  changes      Cart.java modified in two places (stage hunks), docs/api.md staged,
               src/web/app.js deleted, TODO.txt and scratch/ untracked (a long line in there)

It starts in the middle of a merge of clash/b into clash/a, with these conflicts to try in the
conflict editor (abort the merge to get back to a normal state):
  src/web/style.css          long font lines: tick "Scroll A and B separately" to get the bars
  src/shop/OrderService.java several conflicts of different sizes (one is edited vs. deleted lines)
  config.json                two conflicts close together
  docs/report.sql            very long lines on B, short ones on A
  docs/tall.txt              90 lines with conflicts at lines 12, 45 (and clean merges at 14, 60, 78)
  docs/windows-crlf.txt      CRLF line endings, which should survive saving

Things to try: stage a hunk, commit, amend, change a commit's date, create/rename a branch
with spaces, pin branches, search with "Only matches", stash + checkout.

Start over any time by running this script again.
EOM
