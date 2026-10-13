"""Randomised two-sided stress test.

Random creates/edits/deletes/renames happen on the local folder AND on (fake) Drive between sync
passes. Afterwards we require:
  1. both sides converge to the identical tree, and
  2. no content that should survive was lost. Content may only disappear if it was
     overwritten or deleted by its own side's user - never because of the sync.
"""
import os
import random
import time

import pytest

from gdrive.engine import Engine, SafetyStop
from gdrive.state import DB
from tests.fake_drive import FakeDrive


def remote_tree(fd, base=None):
    """{relpath: bytes} for live non-native files under folder `base` on the fake Drive."""
    out = {}
    live = fd.live()
    base = base or fd.root

    def path(f):
        parts = [f["name"]]
        p = f["parents"][0]
        while p != base:
            if p == fd.root:
                return None            # not inside the synced folder
            parent = fd.files[p]
            if parent["trashed"]:
                return None
            parts.append(parent["name"])
            p = parent["parents"][0]
        return "/".join(reversed(parts))
    for f in live.values():
        if "_data" in f:
            rp = path(f)
            if rp:
                out[rp] = f["_data"]
    return out


def local_tree(root):
    out = {}
    for dp, _dn, fn in os.walk(root):
        for n in fn:
            full = os.path.join(dp, n)
            out[os.path.relpath(full, root)] = open(full, "rb").read()
    return out


class Sim:
    def __init__(self, tmp, seed, pair=False):
        self.rnd = random.Random(seed)
        self.root = tmp / "Elsewhere" / "Pair"
        self.fd = FakeDrive()
        self.base = self.fd.root
        self.outside = {}
        if pair:   # sync one Drive folder; a neighbour folder must stay untouched
            self.base = self.fd.remote_dir("Pair")
            other = self.fd.remote_dir("Outside")
            for n in "xyz":
                i = self.fd.remote_add(f"{n}.txt", f"outside-{n}".encode(), parent=other)
                self.outside[i] = f"outside-{n}".encode()
        else:
            self.root = tmp / "Drive"
        self.root.mkdir(parents=True)
        self.eng = Engine(self.fd, self.root, DB(tmp / "s.db"), self.base)
        self.eng.trash_local = False
        self.n = 0
        self.clock = time.time() + 100
        self.required = set()

    def fresh(self, kind):
        self.n += 1
        return f"{kind}{self.n}"

    # ----- local ops -----
    def local_files(self):
        return sorted(local_tree(self.root))

    def local_dirs(self):
        return [""] + sorted(os.path.relpath(os.path.join(dp, d), self.root)
                             for dp, dn, _ in os.walk(self.root) for d in dn)

    def l_write(self, rel, data):
        p = self.root / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(data)
        self.clock += 1
        os.utime(p, (self.clock, self.clock))

    def local_op(self):
        r, files = self.rnd, self.local_files()
        op = r.choice(["create", "create", "edit", "edit", "delete", "rename", "mkdir", "dirmv"])
        if op == "create" or (not files and op in ("edit", "delete", "rename")):
            d = r.choice(self.local_dirs())
            data = self.fresh("L").encode()
            self.l_write(os.path.join(d, self.fresh("f") + ".txt"), data)
            self.required.add(data)
        elif op == "edit":
            rel = r.choice(files)
            old = (self.root / rel).read_bytes()
            data = self.fresh("LE").encode()
            self.l_write(rel, data)
            self.required.discard(old)
            self.required.add(data)
        elif op == "delete":
            rel = r.choice(files)
            self.required.discard((self.root / rel).read_bytes())
            os.remove(self.root / rel)
        elif op == "rename":
            rel = r.choice(files)
            d = r.choice(self.local_dirs())
            new = os.path.join(d, self.fresh("r") + ".txt")
            os.makedirs(self.root / d, exist_ok=True)
            os.rename(self.root / rel, self.root / new)
        elif op == "mkdir":
            (self.root / r.choice(self.local_dirs()) / self.fresh("d")).mkdir(parents=True, exist_ok=True)
        elif op == "dirmv":
            dirs = [d for d in self.local_dirs() if d]
            if dirs:
                d = r.choice(dirs)
                new = os.path.join(os.path.dirname(d), self.fresh("dm"))
                os.rename(self.root / d, self.root / new)

    # ----- remote ops -----
    def remote_dirs(self):
        def inside(f):
            while f["id"] != self.fd.root:
                if f["id"] == self.base:
                    return True
                f = self.fd.files[f["parents"][0]]
            return self.base == self.fd.root
        return [self.base] + [i for i, f in self.fd.live().items()
                              if f["mimeType"].endswith("folder") and i != self.base and inside(f)]

    def remote_filesl(self):
        inside = set()
        for d in self.remote_dirs():
            inside.add(d)
        return [i for i, f in self.fd.live().items() if "_data" in f and f["parents"][0] in inside]

    def remote_op(self):
        r, fd = self.rnd, self.fd
        files = self.remote_filesl()
        op = r.choice(["create", "create", "edit", "edit", "trash", "rename", "mkdir", "dirmv"])
        if op == "create" or (not files and op in ("edit", "trash", "rename")):
            data = self.fresh("R").encode()
            fd.remote_add(self.fresh("g") + ".txt", data, parent=r.choice(self.remote_dirs()))
            self.required.add(data)
        elif op == "edit":
            i = r.choice(files)
            self.required.discard(fd.files[i]["_data"])
            data = self.fresh("RE").encode()
            fd.remote_edit(i, data)
            self.required.add(data)
        elif op == "trash":
            i = r.choice(files)
            self.required.discard(fd.files[i]["_data"])
            fd.remote_trash(i)
        elif op == "rename":
            fd.remote_rename(r.choice(files), name=self.fresh("rr") + ".txt", parent=r.choice(self.remote_dirs()))
        elif op == "mkdir":
            fd.remote_dir(self.fresh("D"), parent=r.choice(self.remote_dirs()))
        elif op == "dirmv":
            dirs = [d for d in self.remote_dirs() if d != self.base]
            if dirs:
                fd.remote_rename(r.choice(dirs), name=self.fresh("DM"))

    def sync(self):
        try:
            self.eng.cycle()
        except SafetyStop:
            self.eng.force = True
            self.eng.cycle()
            self.eng.force = False

    def run(self, steps):
        for _ in range(steps):
            for _ in range(self.rnd.randint(1, 4)):
                (self.local_op if self.rnd.random() < 0.5 else self.remote_op)()
            if self.rnd.random() < 0.7:
                self.sync()
        for _ in range(6):   # settle
            self.sync()


@pytest.mark.parametrize("pair", [False, True])
@pytest.mark.parametrize("seed", range(100))
def test_random_operations_converge_and_lose_nothing(tmp_path, monkeypatch, seed, pair):
    monkeypatch.setattr("gdrive.engine.SETTLE_NS", 0)
    sim = Sim(tmp_path, seed, pair)
    sim.run(25)
    for i, data in sim.outside.items():   # files outside the synced folder: untouched, not trashed
        assert sim.fd.files[i]["_data"] == data and not sim.fd.files[i]["trashed"], f"seed {seed}: outside file touched"
    local, remote = local_tree(sim.root), remote_tree(sim.fd, sim.base)
    assert local == remote, f"seed {seed}: sides differ\nonly local: {set(local)-set(remote)}\nonly remote: {set(remote)-set(local)}"
    have = set(local.values())
    lost = sim.required - have
    assert not lost, f"seed {seed}: LOST DATA {lost}"
