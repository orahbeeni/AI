"""SQLite state: what we last synced, so changes can be told apart from echoes."""
import sqlite3

COLS = ("id parent name is_dir mime rmd5 rsize rmtime link trashed "
        "path md5 l_size l_mtime_ns").split()


class DB:
    def __init__(self, file):
        file = str(file)
        self.file = file
        self.c = sqlite3.connect(file, check_same_thread=False)
        self.c.row_factory = sqlite3.Row
        self.c.executescript("""
        CREATE TABLE IF NOT EXISTS items(
          id TEXT PRIMARY KEY, parent TEXT, name TEXT, is_dir INT, mime TEXT,
          rmd5 TEXT, rsize INT, rmtime TEXT, link TEXT, trashed INT DEFAULT 0,
          path TEXT,            -- where the local copy lives (NULL = not materialised)
          md5 TEXT,             -- content md5 at last sync
          l_size INT, l_mtime_ns INT);
        CREATE INDEX IF NOT EXISTS items_path ON items(path);
        CREATE TABLE IF NOT EXISTS meta(k TEXT PRIMARY KEY, v TEXT);
        """)

    # meta
    def get(self, k, default=None):
        r = self.c.execute("SELECT v FROM meta WHERE k=?", (k,)).fetchone()
        return r["v"] if r else default

    def set(self, k, v):
        self.c.execute("INSERT OR REPLACE INTO meta VALUES(?,?)", (k, v))
        self.c.commit()

    # items
    def item(self, id):
        return self.c.execute("SELECT * FROM items WHERE id=?", (id,)).fetchone()

    def by_path(self, path):
        return self.c.execute("SELECT * FROM items WHERE path=?", (path,)).fetchone()

    def tracked(self):
        return self.c.execute("SELECT * FROM items WHERE path IS NOT NULL").fetchall()

    def all(self):
        return self.c.execute("SELECT * FROM items").fetchall()

    def count(self):
        return self.c.execute("SELECT COUNT(*) FROM items").fetchone()[0]

    def upsert(self, id, **kw):
        if self.item(id) is None:
            self.c.execute("INSERT INTO items(id) VALUES(?)", (id,))
        if kw:
            sets = ",".join(f"{k}=?" for k in kw)
            self.c.execute(f"UPDATE items SET {sets} WHERE id=?", (*kw.values(), id))

    def delete(self, id):
        self.c.execute("DELETE FROM items WHERE id=?", (id,))

    def move_prefix(self, old, new):
        """A directory was renamed on disk: fix paths of everything below it."""
        self.c.execute(
            "UPDATE items SET path=? || substr(path, ?) WHERE path LIKE ? ESCAPE '\\'",
            (new, len(old) + 1, old.replace("\\", "\\\\").replace("%", "\\%").replace("_", "\\_") + "/%"))

    def delete_under(self, prefix):
        esc = prefix.replace("\\", "\\\\").replace("%", "\\%").replace("_", "\\_")
        self.c.execute("DELETE FROM items WHERE path LIKE ? ESCAPE '\\'", (esc + "/%",))

    def commit(self):
        self.c.commit()
