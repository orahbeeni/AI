#!/bin/sh
# Creates a throwaway repository stopped in the middle of a merge, with a mix of
# conflicts for trying out the conflict editor (A / B / Output).
#
#   scripts/make-conflict-playground.sh [directory]   (default: ./conflict-playground)
#
# Run it again at any time to start over.
set -eu

DIR="${1:-./conflict-playground}"
rm -rf "$DIR"
mkdir -p "$DIR"
cd "$DIR"
DIR="$(pwd)"

git init -q
git config user.name "Test User"
git config user.email "test@example.com"
git config core.autocrlf false
git checkout -q -b dev

# --- base ------------------------------------------------------------------
cat > styles.css <<'TXT'
/* Reset */
html, body, div, span, applet, object, iframe,
h1, h2, h3, h4, h5, h6, p, blockquote, pre,
a, abbr, acronym, address, big, cite, code,
small, strike, strong, sub, sup, tt, var,
b, u, i, center,
dl, dt, dd, ol, ul, li,
fieldset, form, label, legend,
table, caption, tbody, tfoot, thead, tr, th, td,
article, aside, canvas, details, embed,
figure, figcaption, footer, header, hgroup,
menu, nav, output, ruby, section, summary,
time, mark, audio, video {
    margin: 0;
    padding: 0;
    border: 0;
    font-size: 100%;
    font: inherit;
    vertical-align: baseline;
}
/* HTML5 display-role reset for older browsers */
article, aside, details, figcaption, figure,
footer, header, hgroup, menu, nav, section {
    display: block;
}
body {
    line-height: 1;
    color: #333333;
}
ol, ul {
    list-style: none;
}
TXT

cat > OrderService.java <<'TXT'
package za.example.orders;

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

    public String describe(Order order) {
        return "Order " + order.getId();
    }
}
TXT

cat > config.json <<'TXT'
{
  "name": "playground",
  "port": 8080,
  "timeout": 30,
  "features": ["search"]
}
TXT

printf 'old notes\nkeep this line\n' > legacy.txt
printf 'line one\r\nline two\r\nline three\r\n' > windows-crlf.txt
git add .
git commit -q -m "base"

# --- merge-conflict branch (B / theirs) --------------------------------------
git checkout -q -b merge-conflict
sed -i.bak 's/    margin: 0;/    margin: 20;/; s/    padding: 0;/    padding: 20;/; s/    border: 0;/    border: 20;/' styles.css
sed -i.bak 's/    color: #333333;/    color: #222222;\n    font-family: sans-serif;/' styles.css
sed -i.bak 's/    list-style: none;/    list-style: none;\n    margin-left: 0;/' styles.css   # merges on its own
cat > OrderService.java <<'TXT'
package za.example.orders;

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

    public String describe(Order order) {
        return "Order " + order.getId();
    }
}
TXT
cat > config.json <<'TXT'
{
  "name": "playground",
  "port": 9090,
  "timeout": 30,
  "features": ["search", "export"]
}
TXT
git rm -q legacy.txt
printf 'line one\r\nline TWO from merge-conflict\r\nline three\r\n' > windows-crlf.txt
printf '# Playground\n\nWritten on merge-conflict.\n' > README.md
rm -f ./*.bak
git add .
git commit -q -m "merge-conflict: bigger padding, stricter validation, drop cancel(), new port"

# --- dev branch (A / ours) ---------------------------------------------------
git checkout -q dev
sed -i.bak 's/    margin: 0;/    margin: 10;/; s/    padding: 0;/    padding: 10;/; s/    border: 0;/    border: 10;/' styles.css
sed -i.bak 's/    color: #333333;/    color: #111111;/' styles.css
sed -i.bak 's/^\/\* Reset \*\//\/* Reset (dev) *\//' styles.css   # merges on its own
cat > OrderService.java <<'TXT'
package za.example.orders;

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

    public String describe(Order order) {
        return "Order " + order.getId();
    }
}
TXT
cat > config.json <<'TXT'
{
  "name": "playground",
  "port": 8443,
  "timeout": 30,
  "features": ["search", "audit"]
}
TXT
printf 'old notes, updated on dev\nkeep this line\n' > legacy.txt
printf 'line one\r\nline TWO from dev\r\nline three\r\n' > windows-crlf.txt
printf '# Playground\n\nWritten on dev.\nWith a second line.\n' > README.md
rm -f ./*.bak
git add .
git commit -q -m "dev: smaller padding, null check, audit cancel(), TLS port"

# --- merge: stops with conflicts ---------------------------------------------
git merge merge-conflict >/dev/null 2>&1 || true

echo "Created $DIR"
echo
git status --short
cat <<EOM

Open it:                 gitgoing "$DIR"
Or go straight in:       gitgoing "$DIR" --resolve styles.css

A = dev (ours), B = merge-conflict (theirs). What to try:
  styles.css         like your GitKraken screenshot: margin/padding/border 10 vs 20 (base 0),
                     plus a body colour conflict where B has an extra line.
                     The header comment (dev) and list margin (B) merge on their own.
  OrderService.java  several conflicts of different sizes: an import, MAX_ITEMS,
                     a line only one side added, validation rewritten differently,
                     and cancel(): edited on dev, deleted on merge-conflict.
  config.json        two conflicts close together (port, features).
  windows-crlf.txt   CRLF line endings: they should survive saving.
  README.md          added on both branches with different text.
  legacy.txt         edited on dev, deleted on merge-conflict (whole-file choice).

Start over any time by running this script again.
EOM
