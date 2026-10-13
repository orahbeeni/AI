"""Adversarial tests: every one of these is a way to lose data if the engine is wrong."""
import os
import time

import pytest

from gdrive.engine import Busy, Engine, SafetyStop
from gdrive.state import DB
from tests.fake_drive import FakeDrive


@pytest.fixture
def env(tmp_path, monkeypatch):
    monkeypatch.setattr("gdrive.engine.SETTLE_NS", 0)
    root = tmp_path / "Drive"
    root.mkdir()
    drive = FakeDrive()
    eng = Engine(drive, root, DB(tmp_path / "state.db"), drive.root)
    eng.trash_local = False
    return drive, root, eng


def write(root, rel, data, bump=True):
    p = root / rel
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_bytes(data)
    if bump:
        t = time.time() + 5
        os.utime(p, (t, t))
    return p


def trashed(drive):
    return [f["name"] for f in drive.files.values() if f.get("trashed")]


def test_deleting_google_doc_stub_does_not_trash_the_doc(env):
    drive, root, eng = env
    drive.remote_doc("Thesis")
    eng.cycle()
    (root / "Thesis.gdoc").unlink()
    eng.cycle()
    assert "Thesis" in {f["name"] for f in drive.live().values()}


def test_remote_edit_between_pull_and_push_is_not_overwritten(env):
    drive, root, eng = env
    rid = drive.remote_add("n.txt", b"v1")
    eng.cycle()
    write(root, "n.txt", b"local edit")
    real = drive.upload_update
    # another device edits the file right after our pull, before our upload
    drive.upload_update = lambda *a, **k: (_ for _ in ()).throw(AssertionError("must not overwrite"))
    drive.remote_edit(rid, b"remote edit")
    eng.cycle()   # pull sees the remote edit -> conflict (not a blind upload)
    drive.upload_update = real
    contents = {f["_data"] for f in drive.live().values() if "_data" in f}
    assert b"remote edit" in contents and b"local edit" in contents


def test_race_remote_edit_after_pull_skips_upload(env):
    drive, root, eng = env
    rid = drive.remote_add("n.txt", b"v1")
    eng.cycle()
    write(root, "n.txt", b"local edit")
    orig_pull = eng.pull

    def racing_pull():
        orig_pull()
        drive.remote_edit(rid, b"remote edit")   # lands after pull, before push
    eng.pull = racing_pull
    eng.cycle()
    assert drive.files[rid]["_data"] == b"remote edit"     # not clobbered
    eng.pull = orig_pull
    eng.cycle()
    contents = {f["_data"] for f in drive.live().values() if "_data" in f}
    assert {b"remote edit", b"local edit"} <= contents


def test_race_remote_edit_after_pull_skips_trash(env):
    drive, root, eng = env
    rid = drive.remote_add("n.txt", b"v1")
    eng.cycle()
    (root / "n.txt").unlink()
    orig_pull = eng.pull

    def racing_pull():
        orig_pull()
        drive.remote_edit(rid, b"important new remote edit")
    eng.pull = racing_pull
    eng.cycle()
    assert not drive.files[rid]["trashed"]
    eng.pull = orig_pull
    eng.cycle()
    assert (root / "n.txt").read_bytes() == b"important new remote edit"   # edit wins over delete


def test_symlink_replacing_a_file_does_not_trash_it(env):
    drive, root, eng = env
    rid = drive.remote_add("a.txt", b"data")
    eng.cycle()
    (root / "a.txt").unlink()
    os.symlink("/etc/hostname", root / "a.txt")
    eng.cycle()
    assert not drive.files[rid]["trashed"]


def test_one_bad_file_does_not_block_other_uploads(env):
    drive, root, eng = env
    write(root, "bad.txt", b"x")
    write(root, "good.txt", b"y")
    real = drive.upload_new

    def flaky(path, name, *a, **k):
        if name == "bad.txt":
            raise RuntimeError("boom")
        return real(path, name, *a, **k)
    drive.upload_new = flaky
    errors = eng.cycle()
    assert "good.txt" in {f["name"] for f in drive.live().values()}
    assert errors


def test_crash_mid_upload_does_not_duplicate(env):
    """Even if a crash loses the DB write, the change feed + adoption of identical files heals it."""
    drive, root, eng = env
    write(root, "a.txt", b"a")
    write(root, "b.txt", b"b")
    real = drive.upload_new
    n = {"c": 0}

    def crashy(*a, **k):
        n["c"] += 1
        if n["c"] == 2:
            raise KeyboardInterrupt
        return real(*a, **k)
    drive.upload_new = crashy
    with pytest.raises(KeyboardInterrupt):
        eng.cycle()
    drive.upload_new = real
    eng.db.c.rollback()           # what a hard crash does to uncommitted work
    eng.cycle()
    names = [f["name"] for f in drive.live().values()]
    assert sorted(names) == ["a.txt", "b.txt"]               # no duplicate "a.txt"


def test_emptied_sync_folder_is_refused(env):
    drive, root, eng = env
    for n in "abcdef":
        drive.remote_add(f"{n}.txt", b"x")
    eng.cycle()
    for f in os.listdir(root):
        os.remove(root / f)
    with pytest.raises(SafetyStop):
        eng.cycle()
    assert len(drive.live()) == 6


def test_second_sync_process_is_refused(env):
    drive, root, eng = env
    other = Engine(drive, root, DB(eng.db.file), drive.root)
    import fcntl
    with open(eng.db.file + ".lock", "w") as fh:
        fcntl.flock(fh, fcntl.LOCK_EX)
        with pytest.raises(Busy):
            other.cycle(wait=False)


def test_expired_page_token_recovers_by_relisting(env):
    from googleapiclient.errors import HttpError
    import httplib2
    drive, root, eng = env
    drive.remote_add("a.txt", b"1")
    eng.cycle()
    gone = drive.remote_add("gone.txt", b"2")
    eng.cycle()
    assert (root / "gone.txt").exists()
    drive.remote_trash(gone)
    drive.remote_add("new.txt", b"3")
    real = drive.changes

    def expired(tok):
        raise HttpError(httplib2.Response({"status": 410}), b"gone")
    drive.changes = expired
    eng.cycle()
    drive.changes = real
    assert (root / "new.txt").exists() and not (root / "gone.txt").exists()
    assert (root / "a.txt").exists()


def test_remote_deletions_go_to_local_trash_when_available(env, monkeypatch):
    drive, root, eng = env
    sent = []
    monkeypatch.setattr(eng, "discard", lambda p: (sent.append(p), os.remove(p)))
    a = drive.remote_add("a.txt", b"1")
    eng.cycle()
    drive.remote_trash(a)
    eng.cycle()
    assert len(sent) == 1 and sent[0].endswith("a.txt")


def test_never_a_permanent_delete_api_exists():
    from gdrive import api
    src = open(api.__file__).read()
    assert ".delete(" not in src and "emptyTrash" not in src


def test_local_folder_rename_is_one_move_and_keeps_folder_identity(env):
    drive, root, eng = env
    d = drive.remote_dir("Old")
    sub = drive.remote_dir("Sub", parent=d)
    a = drive.remote_add("a.txt", b"aaa", parent=d)
    b = drive.remote_add("b.txt", b"bbb", parent=sub)
    eng.cycle()
    os.rename(root / "Old", root / "New")
    drive.calls.clear()
    eng.cycle()
    assert drive.files[d]["name"] == "New" and not drive.files[d]["trashed"]
    assert drive.files[a]["parents"] == [d] and drive.files[b]["parents"] == [sub]
    assert [c[0] for c in drive.calls] == ["move"]          # no mkdir / trash / re-upload
    assert (root / "New/Sub/b.txt").read_bytes() == b"bbb"
    eng.cycle()
    assert [c[0] for c in drive.calls] == ["move"]


# ---- sync pairs: one Drive folder <-> one chosen local folder ----
def pair_env(tmp_path, drive, folder_id, dest, name=None):
    from gdrive import config
    p = config.make_pair(folder_id, name or drive.files[folder_id]["name"], str(dest))
    return Engine(drive, config.local_dir(p), DB(tmp_path / f"{folder_id}.db"), folder_id), config.local_dir(p)


def test_pair_syncs_only_that_folder_into_the_chosen_parent(tmp_path, monkeypatch):
    monkeypatch.setattr("gdrive.engine.SETTLE_NS", 0)
    drive = FakeDrive()
    work = drive.remote_dir("Work")
    proj = drive.remote_dir("Projects", parent=work)
    sub = drive.remote_dir("Deep", parent=proj)
    drive.remote_add("plan.txt", b"plan", parent=proj)
    drive.remote_add("x.txt", b"x", parent=sub)
    drive.remote_add("sibling.txt", b"no", parent=work)
    drive.remote_add("top.txt", b"no")
    eng, local = pair_env(tmp_path, drive, proj, tmp_path / "Elsewhere" / "Docs")
    eng.trash_local = False
    eng.cycle()
    assert local == str(tmp_path / "Elsewhere/Docs/Projects")            # created inside the chosen parent
    assert sorted(os.listdir(local)) == ["Deep", "plan.txt"]
    assert open(local + "/Deep/x.txt", "rb").read() == b"x"
    write_ = lambda rel, d: (os.makedirs(os.path.dirname(f"{local}/{rel}"), exist_ok=True), open(f"{local}/{rel}", "wb").write(d))
    write_("new.txt", b"new")
    time.sleep(0.01)
    eng.cycle()
    n = next(f for f in drive.live().values() if f["name"] == "new.txt")
    assert n["parents"] == [proj]                                         # uploads land in the pair's folder


def test_two_pairs_go_to_two_different_places(tmp_path, monkeypatch):
    monkeypatch.setattr("gdrive.engine.SETTLE_NS", 0)
    drive = FakeDrive()
    a, b = drive.remote_dir("Photos"), drive.remote_dir("Taxes")
    drive.remote_add("p.jpg", b"jpg", parent=a)
    drive.remote_add("t.pdf", b"pdf", parent=b)
    e1, l1 = pair_env(tmp_path, drive, a, tmp_path / "disk1")
    e2, l2 = pair_env(tmp_path, drive, b, tmp_path / "home/docs")
    for e in (e1, e2):
        e.trash_local = False
        e.cycle()
    assert open(l1 + "/p.jpg", "rb").read() == b"jpg" and open(l2 + "/t.pdf", "rb").read() == b"pdf"
    assert not os.path.exists(l1 + "/t.pdf") and not os.path.exists(l2 + "/p.jpg")


def test_deleting_or_moving_the_pair_folder_never_touches_drive(tmp_path, monkeypatch):
    import shutil
    monkeypatch.setattr("gdrive.engine.SETTLE_NS", 0)
    drive = FakeDrive()
    f = drive.remote_dir("Proj")
    for i in range(3):
        drive.remote_add(f"f{i}.txt", b"x", parent=f)
    eng, local = pair_env(tmp_path, drive, f, tmp_path / "P")
    eng.trash_local = False
    eng.cycle()
    shutil.move(local, local + "-moved")                                  # or unmounted, or renamed
    with pytest.raises(SafetyStop):
        eng.cycle()
    assert len(drive.live()) == 4 and not os.path.exists(local)            # not recreated, nothing trashed


def test_trashed_drive_folder_stops_sync_and_keeps_local_files(tmp_path, monkeypatch):
    monkeypatch.setattr("gdrive.engine.SETTLE_NS", 0)
    drive = FakeDrive()
    f = drive.remote_dir("Proj")
    drive.remote_add("keep.txt", b"keep me", parent=f)
    eng, local = pair_env(tmp_path, drive, f, tmp_path / "P")
    eng.trash_local = False
    eng.cycle()
    drive.remote_trash(f)
    with pytest.raises(SafetyStop):
        eng.cycle()
    assert open(local + "/keep.txt", "rb").read() == b"keep me"


def test_item_moved_out_of_the_pair_folder_on_drive_leaves_local(tmp_path, monkeypatch):
    monkeypatch.setattr("gdrive.engine.SETTLE_NS", 0)
    drive = FakeDrive()
    f = drive.remote_dir("Proj")
    fid = drive.remote_add("a.txt", b"a", parent=f)
    eng, local = pair_env(tmp_path, drive, f, tmp_path / "P")
    eng.trash_local = False
    eng.cycle()
    drive.remote_rename(fid, parent=drive.root)
    eng.cycle()
    assert not os.path.exists(local + "/a.txt")
    assert not drive.files[fid]["trashed"]                                # still safe on Drive


def test_state_db_cannot_be_reused_for_another_folder(tmp_path):
    drive = FakeDrive()
    a, b = drive.remote_dir("A"), drive.remote_dir("B")
    Engine(drive, tmp_path / "A", DB(tmp_path / "s.db"), a)
    with pytest.raises(ValueError):
        Engine(drive, tmp_path / "B", DB(tmp_path / "s.db"), b)


def test_overlapping_pairs_are_refused():
    from gdrive import config
    one = config.make_pair("1", "Work", "/home/me/Docs")                  # -> /home/me/Docs/Work
    assert config.check_new_pair([one], config.make_pair("2", "Photos", "/home/me/Docs"))is None
    assert "already" in config.check_new_pair([one], config.make_pair("1", "Work", "/elsewhere"))
    assert "overlaps" in config.check_new_pair([one], config.make_pair("3", "Work/Sub", "/x"))      # nested on Drive
    assert "overlaps" in config.check_new_pair([one], config.make_pair("4", "Archive", "/home/me/Docs/Work"))  # inside a synced dir


# ---- Computers (parentless backup roots) and download-only mode ----
def test_computers_backup_folder_syncs_like_any_pair(tmp_path, monkeypatch):
    monkeypatch.setattr("gdrive.engine.SETTLE_NS", 0)
    drive = FakeDrive()
    comp = drive.remote_computer("my mac shoprite")
    docs = drive.remote_dir("Documents", parent=comp)
    drive.remote_add("a.txt", b"mac file", parent=docs)
    assert drive.computers() == [{"id": comp, "name": "my mac shoprite"}]
    eng, local = pair_env(tmp_path, drive, comp, tmp_path / "Backups")
    eng.trash_local = False
    eng.cycle()
    assert open(local + "/Documents/a.txt", "rb").read() == b"mac file"
    assert local.endswith("Backups/my mac shoprite")


def test_download_only_never_changes_drive(tmp_path, monkeypatch):
    monkeypatch.setattr("gdrive.engine.SETTLE_NS", 0)
    drive = FakeDrive()
    comp = drive.remote_computer("mac")
    d = drive.remote_dir("Docs", parent=comp)
    fid = drive.remote_add("keep.txt", b"original", parent=d)
    gone = drive.remote_add("gone.txt", b"x", parent=d)
    eng, local = pair_env(tmp_path, drive, comp, tmp_path / "B")
    eng.download_only, eng.trash_local = True, False
    eng.cycle()
    drive.calls.clear()
    # local: edit, delete, add, rename, new folder
    p = local + "/Docs/keep.txt"
    open(p, "wb").write(b"edited locally")
    os.utime(p, (time.time() + 9, time.time() + 9))
    os.remove(local + "/Docs/gone.txt")
    open(local + "/Docs/new.txt", "wb").write(b"new")
    os.makedirs(local + "/Newdir")
    os.rename(local + "/Docs", local + "/Renamed")
    eng.cycle()
    eng.cycle()
    assert drive.calls == []                                       # not a single call, read or write
    assert drive.files[fid]["_data"] == b"original" and not drive.files[gone]["trashed"]
    # remote edits still arrive; a clash keeps the local version as a conflicted copy
    drive.remote_edit(fid, b"remote v2")
    eng.cycle()
    assert any(open(os.path.join(dp, n), "rb").read() == b"edited locally" for dp, _, fs in os.walk(local) for n in fs)
    assert [c for c in drive.calls if c[0] != "download"] == []     # only reads: still no writes


def test_rescan_catches_items_the_change_feed_missed(tmp_path, monkeypatch):
    monkeypatch.setattr("gdrive.engine.SETTLE_NS", 0)
    drive = FakeDrive()
    comp = drive.remote_computer("mac")
    drive.remote_add("a.txt", b"a", parent=comp)
    eng, local = pair_env(tmp_path, drive, comp, tmp_path / "B")
    eng.trash_local = False
    eng.cycle()
    drive.remote_add("late.txt", b"late", parent=comp)
    drive.log.clear()                                               # feed "forgets" the change
    drive.changes = lambda tok: iter([(None, "0")])
    eng.cycle()
    assert not os.path.exists(local + "/late.txt")                  # feed alone misses it
    monkeypatch.setattr("gdrive.engine.RESCAN_SECONDS", -1)         # ...the periodic rescan finds it
    eng.cycle()
    assert open(local + "/late.txt", "rb").read() == b"late"


def test_parse_folder_ref():
    from gdrive.api import parse_folder_ref
    i = "1AbCdEfGhIjKlMnOpQrStUvWxYz_-0123"
    assert parse_folder_ref(f"https://drive.google.com/drive/folders/{i}?usp=sharing") == i
    assert parse_folder_ref(f"https://drive.google.com/drive/computers/{i}") == i
    assert parse_folder_ref(f"https://drive.google.com/open?id={i}") == i
    assert parse_folder_ref(i) == i
    assert parse_folder_ref("not a link") is None
