"""In-memory stand-in for gdrive.api.Drive, with a change feed."""
import hashlib
import itertools

from gdrive.api import FOLDER, NotFound

_ids = itertools.count(1)


class FakeDrive:
    def __init__(self):
        self.files = {}
        self.log = []          # change feed: list of file ids
        self.root = "root0"
        self.files[self.root] = dict(id=self.root, name="My Drive", mimeType=FOLDER)
        self.calls = []

    # -- test helpers (act as "another device") --
    def _put(self, name, parent, mime, data=None, mtime="2026-01-01T00:00:00.000Z"):
        id = f"id{next(_ids):06d}"
        f = dict(id=id, name=name, mimeType=mime, parents=[parent], trashed=False,
                 modifiedTime=mtime, webViewLink=f"https://drive.google.com/x/{id}")
        if data is not None:
            f["_data"] = data
            f["md5Checksum"] = hashlib.md5(data).hexdigest()
            f["size"] = str(len(data))
        self.files[id] = f
        self.log.append(id)
        return f

    def remote_add(self, name, data=b"", parent=None, mime="application/octet-stream"):
        return self._put(name, parent or self.root, mime, data)["id"]

    def remote_dir(self, name, parent=None):
        return self._put(name, parent or self.root, FOLDER)["id"]

    def remote_doc(self, name):
        return self._put(name, self.root, "application/vnd.google-apps.document")["id"]

    def remote_edit(self, id, data):
        f = self.files[id]
        f["_data"] = data
        f["md5Checksum"] = hashlib.md5(data).hexdigest()
        f["size"] = str(len(data))
        f["modifiedTime"] = "2026-02-02T00:00:00.000Z"
        self.log.append(id)

    def remote_trash(self, id):
        self.files[id]["trashed"] = True
        self.log.append(id)

    def remote_rename(self, id, name=None, parent=None):
        if name:
            self.files[id]["name"] = name
        if parent:
            self.files[id]["parents"] = [parent]
        self.log.append(id)

    def live(self):
        """Like real Drive: anything inside a trashed folder reads as trashed too."""
        def ok(f):
            while f["id"] != self.root:
                if f["trashed"]:
                    return False
                f = self.files[f["parents"][0]]
            return True
        return {f["id"]: f for f in self.files.values() if f["id"] != self.root and ok(f)}

    def pub(self, f):
        return {k: v for k, v in f.items() if not k.startswith("_")}

    # -- Drive interface --
    def root_id(self):
        return self.root

    def get(self, id):
        if id not in self.files:
            raise NotFound(id)
        return self.pub(self.files[id])

    def start_token(self):
        return str(len(self.log))

    def list_all(self):
        for f in self.live().values():
            yield self.pub(f)

    def changes(self, token):
        n = int(token)
        for id in self.log[n:]:
            yield id, self.pub(self.files[id])
        yield None, str(len(self.log))

    def download(self, id, dest, progress=None):
        self.calls.append(("download", id))
        open(dest, "wb").write(self.files[id]["_data"])

    def child_ids(self, parent):
        return [i for i, f in self.files.items() if f.get("parents") == [parent] and not f["trashed"]]  # direct children

    def list_tree(self, root):
        stack = [root]
        while stack:
            pid = stack.pop()
            for i, f in list(self.files.items()):
                if f.get("parents") == [pid] and not f["trashed"]:
                    yield self.pub(f)
                    if f["mimeType"].endswith("folder"):
                        stack.append(i)

    def computers(self):
        return [{"id": i, "name": f["name"]} for i, f in self.files.items()
                if f["mimeType"].endswith("folder") and not f.get("parents") and i != self.root]

    def remote_computer(self, name):
        i = f"comp{next(_ids):06d}"
        self.files[i] = dict(id=i, name=name, mimeType=FOLDER, trashed=False, modifiedTime="2026-01-01T00:00:00.000Z")
        return i

    def mkdir(self, name, parent):
        self.calls.append(("mkdir", name))
        return self.pub(self._put(name, parent, FOLDER))

    def upload_new(self, path, name, parent, mtime):
        self.calls.append(("upload_new", name))
        return self.pub(self._put(name, parent, "application/octet-stream", open(path, "rb").read(), mtime))

    def upload_update(self, id, path, mtime):
        self.calls.append(("upload_update", id))
        self.remote_edit(id, open(path, "rb").read())
        return self.pub(self.files[id])

    def move(self, id, name, new_parent, old_parent):
        self.calls.append(("move", name))
        self.remote_rename(id, name, new_parent)
        return self.pub(self.files[id])

    def trash(self, id):
        self.calls.append(("trash", id))
        self.remote_trash(id)
