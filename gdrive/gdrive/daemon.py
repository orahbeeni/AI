"""Runs the engine in the background: polls Drive, watches the folder, reports status."""
import json
import logging
import os
import shutil
import subprocess
import threading
import time

from . import config
from .api import Drive
from .engine import Busy, Engine, SafetyStop
from .state import DB

log = logging.getLogger("gdrive")


def notify(title, msg):
    if shutil.which("notify-send"):
        subprocess.Popen(["notify-send", "-i", "folder-remote", "-a", "Google Drive", title, msg])


class Daemon:
    def __init__(self, force=False):
        self.settings = config.load()
        config.DATA_DIR.mkdir(parents=True, exist_ok=True)
        config.PAIRS_DIR.mkdir(parents=True, exist_ok=True)
        self.wake = threading.Event()
        self.stop = threading.Event()
        self.paused = False
        self.state, self.detail, self.last_sync = "starting", "", None
        self.force = force
        self.on_status = lambda: None
        self.drive = None
        self.engines = {}     # pair id -> Engine
        self.blocked = {}     # pair id -> why it was stopped for safety (cleared by Resume)
        self.observer, self.watches = None, {}

    def set_status(self, state, detail=""):
        self.state, self.detail = state, detail
        try:
            config.STATUS.write_text(json.dumps(
                {"state": state, "detail": detail, "last_sync": self.last_sync,
                 "paused": self.paused, "time": time.time()}))
        except OSError:
            pass
        self.on_status()

    def summary(self):
        return {"idle": self.detail or "Up to date", "syncing": self.detail or "Syncing...",
                "paused": "Paused", "error": "Problem: " + self.detail,
                "starting": "Starting..."}.get(self.state, self.state)

    def sync_now(self):
        self.wake.set()

    def toggle_pause(self):
        self.paused = not self.paused
        if not self.paused:
            self.blocked.clear()
        self.set_status("paused" if self.paused else "idle")
        self.wake.set()

    # ----- local change watching -----
    def _sync_watches(self, pairs):
        try:
            from watchdog.events import FileSystemEventHandler
            from watchdog.observers import Observer
        except ImportError:
            return
        if self.observer is None:
            daemon = self

            class H(FileSystemEventHandler):
                def on_any_event(self, e):
                    daemon.debounce()
            self.handler = H()
            self.observer = Observer()
            self.observer.daemon = True
            self.observer.start()
        want = {config.local_dir(p) for p in pairs if os.path.isdir(config.local_dir(p))}
        for d in want - set(self.watches):
            self.watches[d] = self.observer.schedule(self.handler, d, recursive=True)
        for d in set(self.watches) - want:
            self.observer.unschedule(self.watches.pop(d))

    def debounce(self):
        # coalesce bursts of events into one pass ~3s after the last one
        if getattr(self, "_t", None):
            self._t.cancel()
        self._t = threading.Timer(3.0, self.wake.set)
        self._t.daemon = True
        self._t.start()

    # ----- the sync loop -----
    def _engine(self, pair):
        eng = self.engines.get(pair["id"])
        if eng is None:
            if self.drive is None:
                self.drive = Drive()
            eng = Engine(self.drive, config.local_dir(pair), DB(config.db_path(pair)), pair["id"],
                         notify=notify, status=self.set_status, force=self.force,
                         download_only=pair.get("mode") == "download")
            self.engines[pair["id"]] = eng
        return eng

    def sync_all(self):
        pairs = self.settings["pairs"]
        for gone in set(self.engines) - {p["id"] for p in pairs}:
            del self.engines[gone]
        problems = []
        for pair in pairs:
            label = pair["path"]
            if pair["id"] in self.blocked:
                problems.append(f"{label}: {self.blocked[pair['id']]}")
                continue
            try:
                errors = self._engine(pair).cycle()
                if errors:
                    problems.append(f"{label}: {len(errors)} item(s) failed; will retry")
            except Busy:
                log.info("another sync is running; skipping %s", label)
            except SafetyStop as e:
                log.error("%s: %s", label, e)
                self.blocked[pair["id"]] = str(e)
                notify(f"Sync stopped for '{label}'", str(e))
                problems.append(f"{label}: {e}")
            except Exception as e:  # noqa: BLE001  (auth, network...) - retry next time
                log.exception("sync failed for %s", label)
                self.engines.pop(pair["id"], None)
                self.drive = None
                problems.append(f"{label}: {str(e)[:150]}")
        return problems

    def run(self):
        while not self.stop.is_set():
            self.settings = config.load()  # picks up folder changes without a restart
            pairs = self.settings["pairs"]
            if not pairs:
                self.set_status("idle", "No folders yet: choose one to sync")
            elif not self.paused:
                self._sync_watches(pairs)
                self.set_status("syncing")
                problems = self.sync_all()
                if problems:
                    self.set_status("error", "; ".join(problems)[:300])
                else:
                    self.last_sync = time.time()
                    self.set_status("idle", "Up to date")
            self.wake.wait(self.settings["poll_seconds"])
            self.wake.clear()
