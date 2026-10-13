"""Paths and user settings."""
import json
import os
from pathlib import Path

CONFIG_DIR = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config")) / "gdrive-linux"
DATA_DIR = Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local/share")) / "gdrive-linux"
CLIENT_SECRET = CONFIG_DIR / "client_secret.json"
TOKEN = CONFIG_DIR / "token.json"
SETTINGS = CONFIG_DIR / "settings.json"
PAIRS_DIR = DATA_DIR / "pairs"
STATUS = DATA_DIR / "status.json"

DEFAULTS = {
    "poll_seconds": 30,
    "default_dest": str(Path.home()),   # where the folder picker starts
    # Each synced Drive folder is a "pair":
    #   {"id": <Drive folder id>, "path": "Work/Projects", "name": "Projects", "dest": "/home/me/Documents"}
    # and syncs to <dest>/<name>.
    "pairs": [],
}


def load():
    s = dict(DEFAULTS)
    try:
        s.update(json.loads(SETTINGS.read_text()))
    except (OSError, ValueError):
        pass
    return s


def save(s):
    CONFIG_DIR.mkdir(parents=True, exist_ok=True)
    SETTINGS.write_text(json.dumps(s, indent=2))


def local_dir(pair):
    return os.path.join(pair["dest"], pair["name"])


def db_path(pair):
    return PAIRS_DIR / f"{pair['id']}.db"


def _norm(p):
    return os.path.normpath(os.path.abspath(os.path.expanduser(p)))


def _nested(a, b):
    a, b = _norm(a), _norm(b)
    return a == b or a.startswith(b + os.sep) or b.startswith(a + os.sep)


def check_new_pair(pairs, new):
    """Return a human-readable reason this pair can't be added, or None if it's fine.
    Overlaps are refused: two pairs touching the same files (locally or on Drive) would fight."""
    for p in pairs:
        if p["id"] == new["id"]:
            return f"'{p['path']}' is already being synced to {local_dir(p)}."
        a, b = new["path"].strip("/") + "/", p["path"].strip("/") + "/"
        if a.startswith(b) or b.startswith(a):
            return f"'{new['path']}' overlaps '{p['path']}', which is already synced."
        if _nested(local_dir(new), local_dir(p)):
            return f"{local_dir(new)} overlaps {local_dir(p)}, which is already a synced folder."
    return None


def make_pair(folder_id, drive_path, dest, mode="mirror"):
    """mode: "mirror" = two-way sync, "download" = keep a copy only (never changes Drive)."""
    drive_path = drive_path.strip("/")
    return {"id": folder_id, "path": drive_path, "name": drive_path.split("/")[-1].replace("/", "_"),
            "dest": _norm(dest), "mode": mode}


def mode_label(pair):
    return "download only" if pair.get("mode") == "download" else "two-way"
