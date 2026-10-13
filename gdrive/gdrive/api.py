"""Thin wrapper over the Drive v3 API. The sync engine only talks to this interface."""
import os
import re
import threading

from googleapiclient.discovery import build
from googleapiclient.errors import HttpError
from googleapiclient.http import MediaFileUpload, MediaIoBaseDownload

from . import auth

FIELDS = ("id,name,mimeType,parents,md5Checksum,size,modifiedTime,trashed,"
          "webViewLink,shortcutDetails")
FOLDER = "application/vnd.google-apps.folder"
COMPUTERS = "Computers"   # pseudo-folder in our paths for Drive-for-desktop backups
RETRIES = 5


class NotFound(Exception):
    pass


def _x(req):
    try:
        return req.execute(num_retries=RETRIES)
    except HttpError as e:
        if e.resp.status == 404:
            raise NotFound(str(e))
        raise


class Drive:
    def __init__(self, creds=None):
        self.creds = creds or auth.credentials()
        self._local = threading.local()

    @property
    def svc(self):
        # httplib2 is not thread safe: one client per thread
        if not hasattr(self._local, "svc"):
            self._local.svc = build("drive", "v3", credentials=self.creds, cache_discovery=False)
        return self._local.svc

    def root_id(self):
        return _x(self.svc.files().get(fileId="root", fields="id"))["id"]

    def get(self, id):
        return _x(self.svc.files().get(fileId=id, fields=FIELDS))

    def start_token(self):
        return _x(self.svc.changes().getStartPageToken())["startPageToken"]

    def list_all(self):
        tok = None
        while True:
            r = _x(self.svc.files().list(
                q="trashed=false", pageSize=1000, pageToken=tok, corpora="user",
                fields=f"nextPageToken,files({FIELDS})"))
            yield from r.get("files", [])
            tok = r.get("nextPageToken")
            if not tok:
                return

    def changes(self, token):
        """Yield (file_id, file_or_None) for every change; the final value yielded
        under file_id=None carries the next token."""
        while token:
            r = _x(self.svc.changes().list(
                pageToken=token, pageSize=1000, includeRemoved=True,
                fields=f"nextPageToken,newStartPageToken,changes(fileId,removed,file({FIELDS}))"))
            for c in r.get("changes", []):
                yield c["fileId"], (None if c.get("removed") else c.get("file"))
            if r.get("nextPageToken"):
                token = r["nextPageToken"]
            else:
                yield None, r["newStartPageToken"]
                return

    def download(self, id, dest_path, progress=None):
        with open(dest_path, "wb") as fh:
            dl = MediaIoBaseDownload(fh, self.svc.files().get_media(fileId=id), chunksize=8 << 20)
            done = False
            while not done:
                st, done = dl.next_chunk(num_retries=RETRIES)
                if progress and st:
                    progress(st.progress())

    def mkdir(self, name, parent):
        return _x(self.svc.files().create(
            body={"name": name, "mimeType": FOLDER, "parents": [parent]}, fields=FIELDS))

    def upload_new(self, path, name, parent, mtime_iso):
        media = self._media(path)
        return self._resumable(self.svc.files().create(
            body={"name": name, "parents": [parent], "modifiedTime": mtime_iso},
            media_body=media, fields=FIELDS))

    def upload_update(self, id, path, mtime_iso):
        media = self._media(path)
        return self._resumable(self.svc.files().update(
            fileId=id, body={"modifiedTime": mtime_iso}, media_body=media, fields=FIELDS))

    @staticmethod
    def _media(path):
        # resumable uploads reject empty files
        empty = os.path.getsize(path) == 0
        return MediaFileUpload(path, resumable=not empty, chunksize=8 << 20)

    @staticmethod
    def _resumable(req):
        if not req.resumable:
            return _x(req)
        resp = None
        while resp is None:
            _, resp = req.next_chunk(num_retries=RETRIES)
        return resp

    def move(self, id, name, new_parent, old_parent):
        return _x(self.svc.files().update(
            fileId=id, body={"name": name}, addParents=new_parent,
            removeParents=old_parent, fields=FIELDS))

    def trash(self, id):
        try:
            _x(self.svc.files().update(fileId=id, body={"trashed": True}, fields="id"))
        except NotFound:
            pass

    def subfolders(self, parent_id):
        q = f"'{parent_id}' in parents and mimeType='{FOLDER}' and trashed=false"
        out, tok = [], None
        while True:
            r = _x(self.svc.files().list(q=q, pageSize=1000, pageToken=tok, orderBy="name",
                                         fields="nextPageToken,files(id,name)"))
            out += r.get("files", [])
            tok = r.get("nextPageToken")
            if not tok:
                return out

    def find_folder(self, path):
        """Resolve 'Work/Projects' or 'Computers/my mac/Documents' to a folder id, or None."""
        parts = [p for p in path.split("/") if p]
        if parts and parts[0] == COMPUTERS:
            match = [c for c in self.computers() if c["name"].replace("/", "_") == (parts[1] if len(parts) > 1 else None)]
            if not match:
                return None
            cur, parts = match[0]["id"], parts[2:]
        else:
            cur = self.root_id()
        for part in parts:
            match = [f for f in self.subfolders(cur) if f["name"].replace("/", "_") == part]
            if not match:
                return None
            cur = match[0]["id"]
        return cur

    def child_ids(self, parent_id):
        out, tok = [], None
        while True:
            r = _x(self.svc.files().list(q=f"'{parent_id}' in parents and trashed=false", pageSize=1000,
                                         pageToken=tok, fields="nextPageToken,files(id)"))
            out += [f["id"] for f in r.get("files", [])]
            tok = r.get("nextPageToken")
            if not tok:
                return out

    def list_tree(self, root_id):
        """Every non-trashed item below a folder, found by walking its children (no whole-Drive listing)."""
        queue = [root_id]
        while queue:
            pid, tok = queue.pop(), None
            while True:
                r = _x(self.svc.files().list(
                    q=f"'{pid}' in parents and trashed=false", pageSize=1000, pageToken=tok,
                    fields=f"nextPageToken,files({FIELDS})"))
                for f in r.get("files", []):
                    yield f
                    if f.get("mimeType") == FOLDER:
                        queue.append(f["id"])
                tok = r.get("nextPageToken")
                if not tok:
                    break

    def computers(self):
        """Folders backed up from computers by Drive for desktop: owned by me and parentless (not in My Drive)."""
        out, tok = [], None
        while True:
            r = _x(self.svc.files().list(
                q=f"mimeType='{FOLDER}' and trashed=false and 'me' in owners", pageSize=1000, pageToken=tok,
                orderBy="name", fields="nextPageToken,files(id,name,parents)"))
            out += [f for f in r.get("files", []) if not f.get("parents")]
            tok = r.get("nextPageToken")
            if not tok:
                return [{"id": f["id"], "name": f["name"]} for f in out]



def parse_folder_ref(text):
    """Accepts a drive.google.com link or a bare folder id; returns the id (or None)."""
    text = text.strip()
    m = re.search(r"/(?:folders|computers)/([\w-]{10,})", text) or re.search(r"[?&]id=([\w-]{10,})", text)
    if m:
        return m.group(1)
    return text if re.fullmatch(r"[\w-]{10,}", text) else None
