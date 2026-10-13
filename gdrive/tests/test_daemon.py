import threading
import time

from gdrive import config, daemon
from tests.fake_drive import FakeDrive


def test_daemon_syncs_both_ways_and_reacts_to_local_changes(tmp_path, monkeypatch):
    fake = FakeDrive()
    docs = fake.remote_dir("Docs")
    fake.remote_add("hello.txt", b"hi", parent=docs)
    pair = {"id": docs, "path": "Docs", "name": "Docs", "dest": str(tmp_path / "Parent")}
    folder = tmp_path / "Parent" / "Docs"
    monkeypatch.setattr(config, "DATA_DIR", tmp_path / "data")
    monkeypatch.setattr(config, "PAIRS_DIR", tmp_path / "data/pairs")
    monkeypatch.setattr(config, "STATUS", tmp_path / "data/status.json")
    monkeypatch.setattr(config, "load", lambda: {"poll_seconds": 1, "default_dest": str(tmp_path), "pairs": [pair]})
    monkeypatch.setattr(daemon, "Drive", lambda: fake)
    monkeypatch.setattr("gdrive.engine.SETTLE_NS", 0)
    d = daemon.Daemon()
    threading.Thread(target=d.run, daemon=True).start()
    try:
        for _ in range(50):
            if (folder / "hello.txt").exists():
                break
            time.sleep(0.1)
        assert (folder / "hello.txt").read_bytes() == b"hi"
        (folder / "new.txt").write_text("from linux")
        for _ in range(80):
            if any(f["name"] == "new.txt" for f in fake.live().values()):
                break
            time.sleep(0.1)
        assert any(f["name"] == "new.txt" for f in fake.live().values())
        assert d.state in ("idle", "syncing")
    finally:
        d.stop.set()
        d.wake.set()
