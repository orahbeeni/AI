import argparse
import json
import logging
import subprocess
import sys
import time

from . import auth, config


def _confirm(question, yes):
    if yes:
        return True
    if not sys.stdin.isatty():
        sys.exit(question + " (re-run with --yes to confirm)")
    return input(question + " [y/N] ").strip().lower() in ("y", "yes")


def folders_cmd(a):
    import os
    st = config.load()
    pairs = st["pairs"]
    path = (getattr(a, "path", None) or "").strip("/")
    if a.action in (None, "list"):
        if not pairs:
            print("Nothing is synced yet. Try: gdrive folders browse")
        for p in pairs:
            print(f"{p['path']}  ->  {config.local_dir(p)}   [{config.mode_label(p)}]")
    elif a.action == "browse":
        from .api import COMPUTERS, Drive
        d = Drive()
        done = {p["path"] for p in pairs}
        if path == COMPUTERS:
            for c in d.computers():
                full = f"{COMPUTERS}/{c['name']}"
                print(("* " if full in done else "  ") + full)
            return
        fid = d.find_folder(path)
        if fid is None:
            sys.exit(f"No such folder in Drive: {path}")
        for sub in d.subfolders(fid):
            full = f"{path}/{sub['name']}" if path else sub["name"]
            print(("* " if full in done else "  ") + full)
        if not path:
            print(f"  {COMPUTERS}/   (backups from your computers)")
    elif a.action == "add":
        from .api import COMPUTERS, FOLDER, Drive, parse_folder_ref
        drive = Drive()
        if a.link:
            fid = parse_folder_ref(a.link)
            if not fid:
                sys.exit("Couldn't find a folder id in that link.")
            meta = drive.get(fid)
            if meta["mimeType"] != FOLDER:
                sys.exit("That link isn't a folder.")
            path = f"{COMPUTERS}/{meta['name']}" if not meta.get("parents") else f"(link)/{meta['name']}"
        else:
            fid = drive.find_folder(path) if path else None
            if fid is None:
                sys.exit(f"No such folder in Drive: {path or '(empty)'}  (see: gdrive folders browse)")
        dest = a.to
        if not dest:
            if not sys.stdin.isatty():
                sys.exit("Say where to put it with --to /parent/folder")
            dest = input(f"Sync '{path}' into which local parent folder? [{st['default_dest']}]: ").strip() or st["default_dest"]
        mode = "download" if a.download_only else "mirror"
        if path.startswith(COMPUTERS + "/") and not a.two_way:
            mode = "download"   # a computer's backup: keep a copy, never alter the backup
        pair = config.make_pair(fid, path, os.path.expanduser(dest), mode)
        err = config.check_new_pair(pairs, pair)
        if err:
            sys.exit(err)
        target = config.local_dir(pair)
        if os.path.isdir(target) and os.listdir(target) and not _confirm(
                f"{target} already has files. They will be merged with the Drive folder "
                f"(identical files matched" + ("" if mode == "download" else ", new local files uploaded") + "). Continue?", a.yes):
            sys.exit("Cancelled.")
        st["pairs"] = pairs + [pair]
        config.save(st)
        print(f"'{path}' will sync to {target}  [{config.mode_label(pair)}]")
    elif a.action == "remove":
        match = [p for p in pairs if p["path"] == path or config.local_dir(p) == os.path.abspath(os.path.expanduser(path))]
        if not match:
            sys.exit(f"Not synced: {path}")
        p = match[0]
        st["pairs"] = [x for x in pairs if x["id"] != p["id"]]
        config.save(st)
        for suffix in ("", "-wal", "-shm", ".lock"):
            config.db_path(p).with_name(config.db_path(p).name + suffix).unlink(missing_ok=True)
        print(f"Stopped syncing '{p['path']}'. Your local files in {config.local_dir(p)} were left as they are.")


def main(argv=None):
    ap = argparse.ArgumentParser(prog="gdrive", description="Google Drive for Linux")
    sub = ap.add_subparsers(dest="cmd")
    sub.add_parser("login", help="sign in to Google")
    sub.add_parser("logout", help="forget the saved sign-in")
    g = sub.add_parser("gui", help="open the Google Drive window (also runs the sync)")
    g.add_argument("--background", action="store_true", help="start hidden in the tray (used at login)")
    r = sub.add_parser("run", help="headless: sync without any window")
    s = sub.add_parser("sync", help="do one sync pass and exit")
    s.add_argument("--force", action="store_true", help="allow mass deletions")
    sub.add_parser("status", help="show what the running app is doing")
    o = sub.add_parser("open", help="open a .gdoc/.gsheet/... file in the browser")
    o.add_argument("file")
    f = sub.add_parser("folders", help="choose which Drive folders to sync")
    fs = f.add_subparsers(dest="action")
    fs.add_parser("list", help="show synced folders and where they live")
    ad = fs.add_parser("add", help="sync a Drive folder, e.g. 'Work/Projects', into a local parent folder")
    ad.add_argument("path", nargs="?", default="")
    ad.add_argument("--link", help="a drive.google.com folder link or id (e.g. for a folder under Computers)")
    ad.add_argument("--download-only", action="store_true", help="keep a copy only: never change anything on Drive")
    ad.add_argument("--two-way", action="store_true", help="for Computers folders: allow changes to flow back to Drive")
    ad.add_argument("--to", metavar="PARENT", help="local parent folder; the Drive folder is created inside it")
    ad.add_argument("--yes", action="store_true", help="don't ask before merging into an existing folder")
    fs.add_parser("remove", help="stop syncing a folder (local files are kept)").add_argument("path")
    fs.add_parser("browse", help="list folders in Drive").add_argument("path", nargs="?", default="")
    c = sub.add_parser("config", help="show/set settings")
    c.add_argument("--default-dest", help="where the folder picker starts")
    a = ap.parse_args(argv if argv is not None else (sys.argv[1:] or ["gui"]))
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")

    try:
        if a.cmd == "login":
            auth.login()
            print("Signed in.")
        elif a.cmd == "logout":
            auth.logout()
        elif a.cmd == "config":
            s_ = config.load()
            if a.default_dest:
                s_["default_dest"] = a.default_dest
                config.save(s_)
            print(json.dumps(s_, indent=2))
        elif a.cmd == "folders":
            folders_cmd(a)
        elif a.cmd == "open":
            url = json.load(open(a.file)).get("url")
            if not url:
                sys.exit("No URL in that file.")
            subprocess.run(["xdg-open", url])
        elif a.cmd == "status":
            st = json.loads(config.STATUS.read_text())
            age = int(time.time() - st["time"])
            print(f"{st['state']}: {st['detail']}  (updated {age}s ago)")
        elif a.cmd == "sync":
            from .api import Drive
            from .engine import Busy, Engine, SafetyStop
            from .state import DB
            config.PAIRS_DIR.mkdir(parents=True, exist_ok=True)
            st = config.load()
            if not st["pairs"]:
                sys.exit("Nothing to sync. Add a folder: gdrive folders add \"Work\" --to ~/Documents")
            drive, failed = Drive(), 0
            for p in st["pairs"]:
                print(f"== {p['path']} -> {config.local_dir(p)}")
                eng = Engine(drive, config.local_dir(p), DB(config.db_path(p)), p["id"], force=a.force,
                             download_only=p.get("mode") == "download",
                             status=lambda s, d="": print(f"[{s}] {d}") if d else None)
                try:
                    errs = eng.cycle(wait=False)
                    print("Done." if not errs else f"Done with {len(errs)} error(s).")
                    failed += bool(errs)
                except Busy:
                    sys.exit("Another Google Drive sync (the app) is already running.")
                except SafetyStop as e:
                    print("STOPPED:", e)
                    failed += 1
            sys.exit(1 if failed else 0)
        elif a.cmd == "gui":
            from . import gui
            from .daemon import Daemon
            sys.exit(gui.run(Daemon(), background=a.background))
        elif a.cmd == "run":
            from .daemon import Daemon
            try:
                Daemon().run()
            except KeyboardInterrupt:
                pass
    except auth.AuthError as e:
        sys.exit(str(e))


if __name__ == "__main__":
    main()
