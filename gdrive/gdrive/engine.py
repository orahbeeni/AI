"""Two-way mirror sync between a Drive and a local folder.

For every item we remember what we last synced (content md5 + local size/mtime).
That lets us tell "remote changed", "local changed" and "both changed" apart:

  remote only -> download          local only -> upload
  both        -> keep both, local copy is renamed "<name> (conflicted copy <date>)"
  deleted     -> trashed on the other side (Drive trash is recoverable)
"""
import fcntl
import hashlib
import json
import logging
import os
import shutil
import subprocess
import time
from datetime import datetime, timezone

from googleapiclient.errors import HttpError

from .api import FOLDER, NotFound

log = logging.getLogger("gdrive")

NATIVE = "application/vnd.google-apps."
LINK_EXT = {
    "document": ".gdoc", "spreadsheet": ".gsheet", "presentation": ".gslides",
    "drawing": ".gdraw", "form": ".gform", "shortcut": ".glink", "site": ".gsite",
    "map": ".gmap", "jam": ".gjam",
}
IGNORE_NAMES = {".DS_Store", "Thumbs.db", "desktop.ini", ".directory"}
RESCAN_SECONDS = 3600
SETTLE_NS = 2_000_000_000  # don't upload files modified in the last 2s (still being written)


class SafetyStop(Exception):
    pass


class Busy(Exception):
    pass


def is_native(it):
    return bool(it["mime"]) and it["mime"].startswith(NATIVE) and it["mime"] != FOLDER


def ignored(name):
    return (name in IGNORE_NAMES or name.startswith(("~$", ".~lock.", ".gdrive-tmp-", ".goutputstream-"))
            or name.endswith(("~", ".swp", ".swx", ".tmp", ".part", ".crdownload")))


def md5_of(path):
    h = hashlib.md5()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def iso(ns):
    return datetime.fromtimestamp(ns / 1e9, timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.%f")[:-3] + "Z"


def parse_iso(s):
    return datetime.fromisoformat(s.replace("Z", "+00:00")).timestamp()


def still_writing(st):
    return 0 <= time.time_ns() - st.st_mtime_ns < SETTLE_NS


def files_under(files, d):
    """{path relative to d: (size, mtime_ns)} for scanned files below directory d."""
    pre = d + "/"
    return {p[len(pre):]: (st.st_size, st.st_mtime_ns) for p, st in files.items() if p.startswith(pre)}


def depth(rel):
    return rel.count("/")


class Engine:
    def __init__(self, drive, root, db, root_id, notify=None, status=None, force=False, download_only=False):
        self.drive, self.db, self.root = drive, db, str(root)
        self.notify = notify or (lambda title, msg: None)
        self.status = status or (lambda state, detail="": None)
        self.force = force
        self.download_only = download_only   # keep a copy only: never change anything on Drive
        # one Engine syncs one pair: the Drive folder `root_id` <-> the local folder `root`
        self.root_id = root_id
        if db.get("root_id") not in (None, root_id):
            raise ValueError("state database belongs to a different Drive folder")
        db.set("root_id", root_id)
        self.errors = []
        self.trash_local = shutil.which("gio") is not None

    # ---------- helpers ----------
    def attempt(self, what, fn):
        """Run one sync operation; a failure is recorded and does not stop the others."""
        try:
            return fn()
        except (SafetyStop, Busy, KeyboardInterrupt):
            raise
        except Exception as e:  # noqa: BLE001
            log.exception("failed: %s", what)
            self.errors.append(f"{what}: {e}")
            return None

    def discard(self, path):
        """Remove a local file because Drive deleted it. Goes to the desktop Trash when possible."""
        if self.trash_local:
            try:
                subprocess.run(["gio", "trash", path], check=True, timeout=30,
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                return
            except (OSError, subprocess.SubprocessError):
                pass
        os.remove(path)

    def abs(self, rel):
        return os.path.join(self.root, rel)

    def local_name(self, it):
        name = (it["name"] or "_").replace("/", "_")
        if name in (".", ".."):
            name = "_"
        if is_native(it):
            ext = LINK_EXT.get(it["mime"][len(NATIVE):], ".glink")
            if not name.endswith(ext):
                name += ext
        return name

    def remote_rel(self, it):
        """Where this item belongs inside the local folder, or None if trashed / outside the synced Drive folder."""
        if it["id"] == self.root_id:
            return None
        return self._remote_rel(it)

    def _remote_rel(self, it):
        parts, cur = [], it
        for _ in range(200):
            if cur["trashed"]:
                return None
            parts.append(self.local_name(cur))
            p = cur["parent"]
            if p == self.root_id:
                return "/".join(reversed(parts))
            if p is None:
                return None
            cur = self.db.item(p) or self.fetch(p)
            if cur is None:
                return None
        return None

    def fetch(self, id):
        try:
            self.ingest(self.drive.get(id))
        except NotFound:
            self.db.upsert(id, parent=None, trashed=1, is_dir=1)
        return self.db.item(id)

    def ingest(self, f):
        mime = f.get("mimeType", "")
        link = f.get("webViewLink")
        if mime == NATIVE + "shortcut":
            link = "https://drive.google.com/open?id=" + f.get("shortcutDetails", {}).get("targetId", f["id"])
        self.db.upsert(
            f["id"], parent=(f.get("parents") or [None])[0], name=f.get("name"),
            is_dir=int(mime == FOLDER), mime=mime, rmd5=f.get("md5Checksum"),
            rsize=int(f.get("size") or 0), rmtime=f.get("modifiedTime"), link=link,
            trashed=int(bool(f.get("trashed"))))

    def unique(self, rel, id):
        other = self.db.by_path(rel)
        if other is None or other["id"] == id:
            return rel
        stem, ext = os.path.splitext(rel)
        return f"{stem} ({id[:6]}){ext}"

    def conflict_name(self, rel):
        stem, ext = os.path.splitext(rel)
        base = f"{stem} (conflicted copy {datetime.now():%Y-%m-%d %H%M%S})"
        cand, n = base + ext, 1
        while os.path.lexists(self.abs(cand)):
            n += 1
            cand = f"{base} {n}{ext}"
        return cand

    def set_conflict(self, rel):
        new = self.conflict_name(rel)
        os.rename(self.abs(rel), self.abs(new))
        self.notify("Sync conflict", f"Both copies of '{os.path.basename(rel)}' changed. Kept yours as '{os.path.basename(new)}'.")
        log.warning("conflict: %s -> %s", rel, new)

    def record_local(self, id, rel, md5):
        st = os.stat(self.abs(rel))
        self.db.upsert(id, path=rel, md5=md5, l_size=st.st_size, l_mtime_ns=st.st_mtime_ns)

    def local_changed(self, it):
        try:
            st = os.stat(self.abs(it["path"]))
        except OSError:
            return False
        if (st.st_size, st.st_mtime_ns) == (it["l_size"], it["l_mtime_ns"]):
            return False
        if md5_of(self.abs(it["path"])) == it["md5"]:
            self.db.upsert(it["id"], l_size=st.st_size, l_mtime_ns=st.st_mtime_ns)
            return False
        return True

    # ---------- remote -> local ----------
    def pull(self, relist=False):
        tok = self.db.get("page_token")
        touched = set()
        if tok is None:
            self.status("syncing", "Reading your Drive...")
            new_tok = self.drive.start_token()
            seen = set()
            for f in self.drive.list_tree(self.root_id):
                self.ingest(f)
                touched.add(f["id"])
                seen.add(f["id"])
            if relist:  # after an expired token we may have missed deletions: anything not listed is gone
                for r in self.db.all():
                    if r["id"] not in seen and r["id"] != self.root_id and not r["trashed"]:
                        self.db.upsert(r["id"], trashed=1)
                        touched.add(r["id"])
        else:
            new_tok = tok
            try:
                changes = list(self.drive.changes(tok))  # apply nothing until the whole feed arrived
            except HttpError as e:
                if e.resp.status not in (400, 410):
                    raise
                log.warning("change token expired; re-reading the whole Drive")
                self.db.set("page_token", None)
                return self.pull(relist=True)
            for fid, f in changes:
                if fid is None:
                    new_tok = f
                    break
                if f is None:
                    if self.db.item(fid):
                        self.db.upsert(fid, trashed=1)
                else:
                    self.ingest(f)
                touched.add(fid)
        if tok is not None and time.time() - float(self.db.get("last_scan", "0")) > RESCAN_SECONDS:
            # safety net: re-read the folder tree in case the change feed missed something
            for f in self.drive.list_tree(self.root_id):
                self.ingest(f)
                touched.add(f["id"])
            self.db.set("last_scan", str(time.time()))
        elif tok is None:
            self.db.set("last_scan", str(time.time()))
        self.db.commit()
        top = self.db.item(self.root_id) or self.fetch(self.root_id)
        if top["trashed"]:
            raise SafetyStop("The Drive folder was deleted or trashed, or is no longer accessible. "
                             "Syncing it is stopped and your local files were left untouched.")
        touched |= set(json.loads(self.db.get("retry", "[]")))
        # a folder that moved/vanished changes the fate of everything below it
        sweep = any(self.db.item(i)["is_dir"] for i in touched if self.db.item(i))
        if sweep and tok is not None:
            touched |= {r["id"] for r in self.db.tracked()}
        self.reconcile_many(touched)
        self.db.set("page_token", new_tok)

    def reconcile_many(self, ids):
        plan = []
        for id in ids:
            it = self.db.item(id)
            if it is not None:
                plan.append((id, it["path"], self.remote_rel(it)))
        removals = sorted((p for p in plan if p[2] is None), key=lambda p: -depth(p[1] or ""))
        places = sorted((p for p in plan if p[2] is not None), key=lambda p: depth(p[2]))
        retry = set()
        for n, (id, _cur, _want) in enumerate(removals + places):
            try:
                self.reconcile(id)
            except Exception as e:  # keep going; try again next cycle
                log.exception("failed on %s", id)
                self.errors.append(str(e))
                retry.add(id)
            if n % 50 == 0:
                self.db.commit()
        self.db.set("retry", json.dumps(sorted(retry)))
        self.db.commit()

    def reconcile(self, id):
        it = self.db.item(id)
        want = self.remote_rel(it)
        if want is None:
            return self.remove_local(it)
        want = self.unique(want, id)
        if it["is_dir"]:
            self.place_dir(it, want)
        elif is_native(it):
            self.place_link(it, want)
        else:
            self.place_file(it, want)

    def remove_local(self, it):
        """Drop the local copy. Trashed on Drive -> keep local-only edits (re-uploaded as new).
        Merely deselected -> keep tracking until pending edits are uploaded, then remove."""
        cur = it["path"]
        deselected = not it["trashed"]
        if cur and os.path.lexists(self.abs(cur)):
            p = self.abs(cur)
            if it["is_dir"]:
                try:
                    os.rmdir(p)
                except OSError:
                    if deselected:
                        raise RuntimeError(f"{cur}: still has local files, will retry")
            elif is_native(it) or not self.local_changed(it):
                self.discard(p)
            elif deselected:
                raise RuntimeError(f"{cur}: local edits pending upload, will retry")
        if deselected:  # keep the remote metadata so re-selecting the folder is instant
            self.db.upsert(it["id"], path=None, md5=None, l_size=None, l_mtime_ns=None)
        else:
            self.db.delete(it["id"])

    def make_room(self, rel, id):
        """Something untracked sits where `rel` should go: move it aside."""
        if os.path.lexists(self.abs(rel)):
            owner = self.db.by_path(rel)
            if owner is None or owner["id"] != id:
                self.set_conflict(rel)

    def rename_local(self, it, want):
        cur = it["path"]
        if cur and cur != want and os.path.lexists(self.abs(cur)):
            self.make_room(want, it["id"])
            os.makedirs(os.path.dirname(self.abs(want)), exist_ok=True)
            os.rename(self.abs(cur), self.abs(want))
            if it["is_dir"]:
                self.db.move_prefix(cur, want)
            self.db.upsert(it["id"], path=want)
            return True
        return False

    def place_dir(self, it, want):
        if not self.rename_local(it, want):
            if os.path.isfile(self.abs(want)):
                self.set_conflict(want)
        os.makedirs(self.abs(want), exist_ok=True)
        self.db.upsert(it["id"], path=want)

    def place_link(self, it, want):
        self.rename_local(it, want)
        p = self.abs(want)
        if not os.path.exists(p):
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, "w") as f:
                json.dump({"url": it["link"], "doc_id": it["id"]}, f)
        self.db.upsert(it["id"], path=want)

    def place_file(self, it, want):
        self.rename_local(it, want)
        it = self.db.item(it["id"])
        dst = self.abs(want)
        if os.path.lexists(dst):
            if it["path"] == want:
                if it["rmd5"] == it["md5"]:
                    return  # remote content unchanged
                if self.local_changed(it):
                    self.set_conflict(want)
            elif md5_of(dst) == it["rmd5"]:
                return self.record_local(it["id"], want, it["rmd5"])  # already identical: adopt
            else:
                self.set_conflict(want)
        self.download(it, want)

    def download(self, it, want):
        dst = self.abs(want)
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        tmp = os.path.join(os.path.dirname(dst), f".gdrive-tmp-{it['id'][:12]}")
        self.status("syncing", f"Downloading {os.path.basename(want)}")
        try:
            self.drive.download(it["id"], tmp)
            got = md5_of(tmp)
            if it["rmd5"] and got != it["rmd5"]:
                raise IOError(f"checksum mismatch downloading {want}")
            if it["rmtime"]:
                t = parse_iso(it["rmtime"])
                os.utime(tmp, (t, t))
            os.replace(tmp, dst)
        finally:
            if os.path.exists(tmp):
                os.remove(tmp)
        self.record_local(it["id"], want, got)

    # ---------- local -> remote ----------
    def scan(self):
        dirs, files = set(), {}
        def walk(rel):
            with os.scandir(self.abs(rel) if rel else self.root) as it:
                for e in it:
                    r = f"{rel}/{e.name}" if rel else e.name
                    if e.is_symlink():
                        continue
                    if e.is_dir(follow_symlinks=False):
                        dirs.add(r)
                        walk(r)
                    elif e.is_file(follow_symlinks=False):
                        files[r] = e.stat(follow_symlinks=False)
        walk("")
        return dirs, files

    def parent_of(self, rel):
        d = os.path.dirname(rel)
        if not d:
            return self.root_id
        row = self.db.by_path(d)
        return row["id"] if row else None

    def remote_state(self, row):
        """Ask Drive for the item's current state right before we change it.
        'same' = still what we last synced, 'gone' = deleted remotely, 'changed' = someone edited it
        since we pulled (we record the new state so the next pull resolves it as an edit/conflict)."""
        try:
            cur = self.drive.get(row["id"])
        except NotFound:
            return "gone"
        if cur.get("trashed") or (not row["is_dir"] and cur.get("md5Checksum") != row["rmd5"]):
            self.ingest(cur)
            self.db.commit()
            return "changed"
        return "same"

    def dir_safe_to_trash(self, row):
        """Trashing a folder on Drive trashes everything inside it. Only do that if every item
        in it is known to us and was synced here (nothing new or never-downloaded inside)."""
        by_parent = {}
        for r in self.db.all():
            by_parent.setdefault(r["parent"], []).append(r)
        stack = [row["id"]]
        while stack:
            for r in by_parent.get(stack.pop(), []):
                if not r["trashed"] and r["path"] is None:
                    return False
                stack.append(r["id"])
        known = {r["id"] for r in self.db.all()}
        return all(k in known for k in self.drive.child_ids(row["id"]))

    def detect_dir_renames(self, dirs, files, tracked):
        """A vanished folder whose files all reappear unchanged (same relative path, size and mtime)
        inside a new folder is a rename/move: do it as one move so the folder keeps its Drive identity."""
        new = [d for d in dirs if d not in tracked and not ignored(os.path.basename(d))]
        missing = sorted((r for p, r in tracked.items() if r["is_dir"] and p not in dirs
                          and not os.path.lexists(self.abs(p))), key=lambda r: depth(r["path"]))
        changed = False
        for m in missing:
            mp = m["path"]
            members = {r["path"][len(mp) + 1:]: (r["l_size"], r["l_mtime_ns"]) for r in self.db.tracked()
                       if r["path"].startswith(mp + "/") and not r["is_dir"] and not is_native(r)}
            if not members:
                continue
            for n in new:
                if os.path.basename(n) == os.path.basename(mp) and os.path.dirname(n) == os.path.dirname(mp):
                    continue
                under = files_under(files, n)
                if all(under.get(rel_) == sig for rel_, sig in members.items()):
                    pid = self.parent_of(n)
                    if pid is None:
                        continue
                    def do(m=m, n=n, pid=pid):
                        f = self.drive.move(m["id"], os.path.basename(n), pid, m["parent"])
                        self.ingest(f)
                        self.db.move_prefix(m["path"], n)
                        self.db.upsert(m["id"], path=n)
                        self.db.commit()
                    self.attempt(f"rename folder {mp} -> {n}", do)
                    if (self.db.item(m["id"]) or {"path": None})["path"] == n:
                        changed = True
                        new.remove(n)
                    break
        return changed

    def push(self):
        dirs, files = self.scan()
        tracked = {r["path"]: r for r in self.db.tracked()}
        if self.detect_dir_renames(dirs, files, tracked):
            tracked = {r["path"]: r for r in self.db.tracked()}
        present = dirs | set(files)
        new_dirs = sorted((d for d in dirs if d not in tracked and not ignored(os.path.basename(d))), key=depth)
        new_files = [f for f in files if f not in tracked and not ignored(os.path.basename(f))]
        # "missing" means truly absent: a symlink or odd file sitting at the path is not a deletion
        missing = [r for p, r in tracked.items() if p not in present and not os.path.lexists(self.abs(p))]
        if len(tracked) >= 5 and not present and not self.force:
            raise SafetyStop("The sync folder is empty but Drive has files that were synced into it. "
                             "Is the disk unmounted? Nothing was deleted. "
                             "Run 'gdrive sync --force' if you really emptied it.")
        if len(missing) > 20 and len(missing) > len(tracked) / 2 and not self.force:
            raise SafetyStop(f"{len(missing)} of {len(tracked)} synced items are missing locally; "
                             "refusing to trash them on Drive. Is the folder unmounted? "
                             "Run 'gdrive sync --force' if this is intended.")

        def new_dir(d):
            pid = self.parent_of(d)
            if pid is None:
                return
            f = self.drive.mkdir(os.path.basename(d), pid)
            self.ingest(f)
            self.db.upsert(f["id"], path=d)
            self.db.commit()
        for d in new_dirs:
            self.attempt(f"create folder {d}", lambda d=d: new_dir(d))

        # a missing file whose content shows up elsewhere is a rename/move, not delete + upload
        gone = {}
        for r in missing:
            if not r["is_dir"] and not is_native(r) and r["md5"]:
                gone.setdefault((r["l_size"], r["md5"]), []).append(r)
        sizes = {k[0] for k in gone}
        moved = set()

        def move(rel, match, pid):
            f = self.drive.move(match["id"], os.path.basename(rel), pid, match["parent"])
            self.ingest(f)
            self.record_local(match["id"], rel, match["md5"])
            self.db.commit()
            moved.add(match["id"])
        for rel in list(new_files):
            st = files[rel]
            if st.st_size not in sizes or still_writing(st):
                continue
            key = (st.st_size, md5_of(self.abs(rel)))
            pid = self.parent_of(rel)
            if gone.get(key) and pid:
                match = gone[key].pop(0)
                if self.attempt(f"move {rel}", lambda: move(rel, match, pid)) is None and match["id"] in moved:
                    new_files.remove(rel)
                elif match["id"] in moved:
                    new_files.remove(rel)
                else:
                    gone[key].insert(0, match)
        missing = [m for m in missing if m["id"] not in moved]

        def update(rel, row, st):
            self.status("syncing", f"Uploading {os.path.basename(rel)}")
            if row["rmd5"] != row["md5"] or self.remote_state(row) != "same":
                log.info("%s changed on Drive too; resolving on next pull", rel)
                return
            f = self.drive.upload_update(row["id"], self.abs(rel), iso(st.st_mtime_ns))
            self.ingest(f)
            self.db.upsert(row["id"], md5=f.get("md5Checksum"), l_size=st.st_size, l_mtime_ns=st.st_mtime_ns)
            self.db.commit()
        for rel, row in tracked.items():
            if rel in files and not row["is_dir"] and not is_native(row):
                st = files[rel]
                if (st.st_size, st.st_mtime_ns) == (row["l_size"], row["l_mtime_ns"]) or still_writing(st):
                    continue
                if self.local_changed(row):
                    self.attempt(f"upload {rel}", lambda rel=rel, row=row, st=st: update(rel, row, st))

        def upload(rel, st, pid):
            self.status("syncing", f"Uploading {os.path.basename(rel)}")
            f = self.drive.upload_new(self.abs(rel), os.path.basename(rel), pid, iso(st.st_mtime_ns))
            self.ingest(f)
            self.db.upsert(f["id"], path=rel, md5=f.get("md5Checksum"), l_size=st.st_size, l_mtime_ns=st.st_mtime_ns)
            self.db.commit()
        for rel in sorted(new_files, key=depth):
            st = files[rel]
            pid = self.parent_of(rel)
            if pid is not None and not still_writing(st):
                self.attempt(f"upload {rel}", lambda rel=rel, st=st, pid=pid: upload(rel, st, pid))

        # deletions, shallowest first so a trashed folder covers its contents
        trashed_dirs = []
        for row in sorted(missing, key=lambda r: depth(r["path"])):
            self.attempt(f"delete {row['path']}", lambda row=row: self.propagate_delete(row, trashed_dirs))
        self.db.commit()

    def propagate_delete(self, row, trashed_dirs):
        path = row["path"]
        if any(path.startswith(d + "/") for d in trashed_dirs):
            self.db.delete(row["id"])
            return
        if is_native(row):          # deleting a .gdoc stub never deletes the document itself
            self.db.upsert(row["id"], path=None)
        else:
            if row["rmd5"] != row["md5"] and not row["is_dir"]:
                return                              # unapplied remote edit: let the next pull win
            state = self.remote_state(row)
            if state == "changed":
                return
            if state == "same":
                if row["is_dir"] and not self.dir_safe_to_trash(row):
                    log.warning("not trashing %s: it contains items that were never synced here", path)
                    self.db.upsert(row["id"], path=None)
                    return
                self.drive.trash(row["id"])
            if row["is_dir"]:
                trashed_dirs.append(path)
                self.db.delete_under(path)
            self.db.delete(row["id"])
        self.db.commit()

    # ---------- one full pass ----------
    def cycle(self, wait=True):
        """One sync pass. Only one process may sync at a time (GUI, daemon and CLI share a lock)."""
        if not os.path.isdir(self.root):
            if self.db.get("page_token") is not None:  # it existed before: moved, renamed or unmounted?
                raise SafetyStop(f"The local folder {self.root} is missing (moved, renamed or unmounted?). "
                                 "Nothing was changed on Drive.")
            os.makedirs(self.root)                      # first sync creates it
        with open(self.db.file + ".lock", "w") as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | (0 if wait else fcntl.LOCK_NB))
            except BlockingIOError:
                raise Busy("Another Google Drive sync is already running.")
            self.errors = []
            self.pull()
            if not self.download_only:
                self.push()
            return self.errors
