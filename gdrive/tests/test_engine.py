import json
import os
import time

import pytest

from gdrive.engine import Engine, SafetyStop
from gdrive.state import DB
from tests.fake_drive import FakeDrive


@pytest.fixture
def env(tmp_path, monkeypatch):
    monkeypatch.setattr("gdrive.engine.SETTLE_NS", 0)
    root = tmp_path / "Drive"
    root.mkdir()
    drive = FakeDrive()
    db = DB(tmp_path / "state.db")
    eng = Engine(drive, root, db, drive.root)
    return drive, root, eng


def names(drive):
    return sorted(f["name"] for f in drive.live().values())


def write(root, rel, data):
    p = root / rel
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_bytes(data)
    return p


def test_initial_download_and_tree(env):
    drive, root, eng = env
    d = drive.remote_dir("Photos")
    drive.remote_add("a.txt", b"hello", parent=d)
    drive.remote_add("top.bin", b"\x00\x01")
    eng.cycle()
    assert (root / "Photos/a.txt").read_bytes() == b"hello"
    assert (root / "top.bin").read_bytes() == b"\x00\x01"


def test_google_doc_becomes_link_file(env):
    drive, root, eng = env
    id = drive.remote_doc("Plan")
    eng.cycle()
    link = json.loads((root / "Plan.gdoc").read_text())
    assert link["doc_id"] == id and link["url"].startswith("https://")
    # editing/removing the stub must not upload anything weird
    assert not any(c[0] == "upload_new" for c in drive.calls)


def test_upload_new_and_nested(env):
    drive, root, eng = env
    write(root, "Work/Reports/q1.txt", b"numbers")
    eng.cycle()
    assert names(drive) == ["Reports", "Work", "q1.txt"]
    eng.cycle()  # second pass is a no-op (no echo re-uploads)
    assert [c for c in drive.calls if c[0] in ("upload_new", "upload_update")] == [("upload_new", "q1.txt")]


def test_local_edit_uploads_remote_edit_downloads(env):
    drive, root, eng = env
    rid = drive.remote_add("n.txt", b"v1")
    eng.cycle()
    p = write(root, "n.txt", b"v2 local")
    os.utime(p, (time.time() + 5, time.time() + 5))
    eng.cycle()
    assert drive.files[rid]["_data"] == b"v2 local"
    drive.remote_edit(rid, b"v3 remote")
    eng.cycle()
    assert p.read_bytes() == b"v3 remote"


def test_conflict_keeps_both(env):
    drive, root, eng = env
    rid = drive.remote_add("c.txt", b"base")
    eng.cycle()
    p = write(root, "c.txt", b"mine!")
    os.utime(p, (time.time() + 5, time.time() + 5))
    drive.remote_edit(rid, b"theirs")
    eng.cycle()
    assert p.read_bytes() == b"theirs"
    copies = [f for f in os.listdir(root) if "conflicted copy" in f]
    assert len(copies) == 1 and (root / copies[0]).read_bytes() == b"mine!"
    assert any(f["_data"] == b"mine!" for f in drive.live().values() if "_data" in f)  # copy uploaded


def test_remote_delete_removes_local_but_keeps_edited(env):
    drive, root, eng = env
    a = drive.remote_add("a.txt", b"1")
    b = drive.remote_add("b.txt", b"2")
    eng.cycle()
    p = write(root, "b.txt", b"edited")
    os.utime(p, (time.time() + 5, time.time() + 5))
    drive.remote_trash(a)
    drive.remote_trash(b)
    eng.cycle()
    assert not (root / "a.txt").exists()
    assert (root / "b.txt").read_bytes() == b"edited"      # not lost
    assert "b.txt" in names(drive)                          # re-uploaded as new


def test_local_delete_trashes_remote(env):
    drive, root, eng = env
    drive.remote_add("a.txt", b"1")
    drive.remote_add("b.txt", b"2")
    eng.cycle()
    (root / "a.txt").unlink()
    eng.cycle()
    assert names(drive) == ["b.txt"]


def test_local_rename_is_a_move_not_reupload(env):
    drive, root, eng = env
    rid = drive.remote_add("old.txt", b"same content")
    drive.remote_dir("Dest")
    eng.cycle()
    os.rename(root / "old.txt", root / "Dest" / "new.txt")
    eng.cycle()
    f = drive.files[rid]
    assert f["name"] == "new.txt" and f["parents"] == [next(i for i, x in drive.files.items() if x["name"] == "Dest")]
    assert not any(c[0] == "upload_new" and c[1] == "new.txt" for c in drive.calls)


def test_remote_rename_and_folder_move(env):
    drive, root, eng = env
    d = drive.remote_dir("A")
    fid = drive.remote_add("f.txt", b"x", parent=d)
    eng.cycle()
    drive.remote_rename(d, name="B")
    eng.cycle()
    assert (root / "B/f.txt").read_bytes() == b"x" and not (root / "A").exists()
    drive.remote_rename(fid, name="g.txt", parent=drive.root)
    eng.cycle()
    assert (root / "g.txt").exists() and not (root / "B/f.txt").exists()


def test_local_dir_rename(env):
    drive, root, eng = env
    d = drive.remote_dir("A")
    drive.remote_add("f.txt", b"x", parent=d)
    eng.cycle()
    os.rename(root / "A", root / "Z")
    eng.cycle()
    live = drive.live()
    assert sorted(f["name"] for f in live.values()) == ["Z", "f.txt"]
    eng.cycle()


def test_untracked_identical_file_is_adopted_not_duplicated(env):
    drive, root, eng = env
    drive.remote_add("same.txt", b"abc")
    write(root, "same.txt", b"abc")
    eng.cycle()
    assert [f for f in os.listdir(root)] == ["same.txt"]
    assert not any(c[0] in ("download", "upload_new") for c in drive.calls)


def test_temp_files_ignored(env):
    drive, root, eng = env
    write(root, "~$doc.docx", b"x")
    write(root, ".doc.swp", b"x")
    write(root, "real.txt", b"x")
    eng.cycle()
    assert names(drive) == ["real.txt"]


def test_duplicate_remote_names_do_not_clobber(env):
    drive, root, eng = env
    drive.remote_add("dup.txt", b"one")
    drive.remote_add("dup.txt", b"two")
    eng.cycle()
    assert sorted(f for f in os.listdir(root)) == sorted(f for f in os.listdir(root))
    assert len(os.listdir(root)) == 2


def test_safety_stop_when_folder_emptied(env):
    drive, root, eng = env
    for i in range(30):
        drive.remote_add(f"f{i}.txt", b"x")
    eng.cycle()
    for f in os.listdir(root):
        os.remove(root / f)
    with pytest.raises(SafetyStop):
        eng.cycle()
    assert len(drive.live()) == 30
    eng.force = True
    eng.cycle()
    assert len(drive.live()) == 0


def test_empty_file_and_slash_in_name(env):
    drive, root, eng = env
    drive.remote_add("a/b.txt", b"")
    write(root, "empty.txt", b"")
    eng.cycle()
    assert (root / "a_b.txt").exists()
    assert "empty.txt" in names(drive)


