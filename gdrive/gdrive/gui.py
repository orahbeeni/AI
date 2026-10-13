"""GTK window + (optional) tray icon. One process owns the sync daemon."""
import json
import os
import shutil
import subprocess
import threading
import time

import gi

gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
from gi.repository import Gdk, Gio, GLib, Gtk  # noqa: E402

from . import auth, config, tray  # noqa: E402
from .api import COMPUTERS, FOLDER, Drive, parse_folder_ref  # noqa: E402

CONSOLE = "https://console.cloud.google.com/apis/credentials"
SETUP_TEXT = (
    "Google needs every app to have its own free credentials. One-time setup (~5 min):\n\n"
    "1.  Open the Google Cloud console and create a project.\n"
    "2.  Enable the <b>Google Drive API</b>.\n"
    "3.  OAuth consent screen: type External, add yourself as a test user, then click "
    "<b>Publish app</b> (otherwise sign-in expires every 7 days).\n"
    "4.  Credentials → Create → <b>OAuth client ID → Desktop app</b>, then download the JSON.\n"
    "5.  Pick the downloaded file below."
)
COL_CHECK, COL_NAME, COL_PATH, COL_LOADED, COL_INCONS, COL_SENS, COL_ID, COL_DEST = range(8)
COMPUTERS_ID = "__computers__"   # pseudo-row that groups Drive-for-desktop backups


def ago(t):
    if not t:
        return "never"
    s = int(time.time() - t)
    return "just now" if s < 10 else f"{s}s ago" if s < 90 else f"{s // 60} min ago" if s < 5400 else f"{s // 3600} h ago"


class Window(Gtk.ApplicationWindow):
    def __init__(self, app, daemon):
        super().__init__(application=app, title="Google Drive", default_width=700, default_height=640)
        self.app, self.daemon = app, daemon
        self.drive = None
        self.set_icon_name("folder-remote")
        self.stack = Gtk.Stack(transition_type=Gtk.StackTransitionType.CROSSFADE)
        self.add(self.stack)
        self.stack.add_named(self._setup_page(), "setup")
        self.stack.add_named(self._main_page(), "main")
        self.show_all()
        self.route()
        GLib.timeout_add_seconds(2, self.tick)

    # ----- pages -----
    def _box(self, spacing=10):
        b = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=spacing)
        b.set_border_width(18)
        return b

    def _setup_page(self):
        b = self._box(14)
        title = Gtk.Label(xalign=0)
        title.set_markup("<span size='x-large' weight='bold'>Set up Google Drive</span>")
        b.pack_start(title, False, False, 0)
        self.setup_msg = Gtk.Label(xalign=0, wrap=True, use_markup=True)
        b.pack_start(self.setup_msg, False, False, 0)
        row = Gtk.Box(spacing=8)
        self.btn_console = Gtk.Button(label="Open Google Cloud console")
        self.btn_console.connect("clicked", lambda *_: Gio.AppInfo.launch_default_for_uri(CONSOLE, None))
        self.btn_creds = Gtk.Button(label="Choose credentials file…")
        self.btn_creds.connect("clicked", self.on_choose_creds)
        row.pack_start(self.btn_console, False, False, 0)
        row.pack_start(self.btn_creds, False, False, 0)
        b.pack_start(row, False, False, 0)
        self.btn_signin = Gtk.Button(label="Sign in with Google")
        self.btn_signin.get_style_context().add_class("suggested-action")
        self.btn_signin.connect("clicked", self.on_signin)
        b.pack_start(self.btn_signin, False, False, 6)
        self.setup_err = Gtk.Label(xalign=0, wrap=True)
        b.pack_start(self.setup_err, False, False, 0)
        return b

    def _main_page(self):
        b = self._box(10)
        self.status_icon = Gtk.Image.new_from_icon_name("emblem-synchronizing", Gtk.IconSize.DIALOG)
        self.status_lbl = Gtk.Label(xalign=0, wrap=True)
        self.sub_lbl = Gtk.Label(xalign=0, wrap=True)
        self.sub_lbl.get_style_context().add_class("dim-label")
        head = Gtk.Box(spacing=12)
        text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        text.pack_start(self.status_lbl, False, False, 0)
        text.pack_start(self.sub_lbl, False, False, 0)
        head.pack_start(self.status_icon, False, False, 0)
        head.pack_start(text, True, True, 0)
        b.pack_start(head, False, False, 0)

        btns = Gtk.Box(spacing=6)
        for label, fn in (("Sync now", lambda *_: self.daemon.sync_now()),
                          ("Open folder", self.on_open_folder),
                          ("drive.google.com", lambda *_: Gio.AppInfo.launch_default_for_uri("https://drive.google.com", None))):
            bt = Gtk.Button(label=label)
            bt.connect("clicked", fn)
            btns.pack_start(bt, False, False, 0)
        self.btn_pause = Gtk.Button(label="Pause")
        self.btn_pause.connect("clicked", lambda *_: self.daemon.toggle_pause())
        btns.pack_start(self.btn_pause, False, False, 0)
        b.pack_start(btns, False, False, 0)

        hdr = Gtk.Label(xalign=0)
        hdr.set_markup("<b>Folders to sync</b>")
        b.pack_start(hdr, False, False, 6)
        hint = Gtk.Label(xalign=0, wrap=True,
                         label="Tick a Drive folder to sync it, then choose which folder on this computer it goes into. "
                               "Unticked folders stay in the cloud only.")
        hint.get_style_context().add_class("dim-label")
        b.pack_start(hint, False, False, 0)

        self.store = Gtk.TreeStore(bool, str, str, bool, bool, bool, str, str)
        self.tree = Gtk.TreeView(model=self.store, headers_visible=False)
        tog = Gtk.CellRendererToggle()
        tog.connect("toggled", self.on_toggled)
        # group row ("Computers") and "Loading…" placeholders get no tick box
        tog_col_fn = lambda col, cell, model, it, _d: cell.set_property(
            "visible", model.get_value(it, COL_ID) not in ("", COMPUTERS_ID))
        tcol = Gtk.TreeViewColumn("", tog, active=COL_CHECK, inconsistent=COL_INCONS, activatable=COL_SENS, sensitive=COL_SENS)
        tcol.set_cell_data_func(tog, tog_col_fn)
        self.tree.append_column(tcol)
        col = Gtk.TreeViewColumn("", Gtk.CellRendererText(), text=COL_NAME)
        icon = Gtk.CellRendererPixbuf(icon_name="folder")
        col.pack_start(icon, False)
        col.reorder(icon, 0) if hasattr(col, "reorder") else None
        self.tree.append_column(col)
        dest = Gtk.CellRendererText(foreground="#777777", ellipsize=3)
        self.tree.append_column(Gtk.TreeViewColumn("", dest, text=COL_DEST))
        self.tree.connect("test-expand-row", self.on_expand)
        self.tree.connect("row-activated", self.on_row_activated)
        sc = Gtk.ScrolledWindow(shadow_type=Gtk.ShadowType.IN)
        sc.add(self.tree)
        b.pack_start(sc, True, True, 0)

        link = Gtk.Button(label="Add a folder by link…", halign=Gtk.Align.START)
        link.set_tooltip_text("Paste a drive.google.com folder link, e.g. for a backed-up computer folder that isn't listed")
        link.connect("clicked", self.on_add_link)
        b.pack_start(link, False, False, 0)
        tip = Gtk.Label(xalign=0, label="Double-click a synced folder to open it on this computer.")
        tip.get_style_context().add_class("dim-label")
        b.pack_start(tip, False, False, 0)
        out = Gtk.Button(label="Sign out", halign=Gtk.Align.END)
        out.connect("clicked", self.on_signout)
        b.pack_start(out, False, False, 0)
        return b

    # ----- routing -----
    def route(self):
        have_creds, signed_in = config.CLIENT_SECRET.exists(), config.TOKEN.exists()
        if signed_in:
            self.stack.set_visible_child_name("main")
            if self.drive is None:
                self.drive = Drive()
                self.load_children(None, None)
        else:
            self.drive = None
            self.stack.set_visible_child_name("setup")
            self.setup_msg.set_markup(
                SETUP_TEXT if not have_creds else
                "Credentials found. Click <b>Sign in with Google</b>; your browser will open.")
            self.btn_signin.set_sensitive(have_creds)
            self.btn_console.set_visible(not have_creds)
            self.btn_creds.set_label("Choose credentials file…" if not have_creds else "Replace credentials file…")
        self.refresh()

    def refresh(self):
        d = self.daemon
        titles = {"idle": "Up to date", "syncing": "Syncing…", "paused": "Paused",
                  "error": "Needs attention", "starting": "Starting…"}
        self.status_lbl.set_markup(f"<span size='large' weight='bold'>{titles.get(d.state, d.state)}</span>")
        detail = d.summary() if d.state in ("syncing", "error") else ""
        self.sub_lbl.set_text(" · ".join(x for x in (detail[:90], f"Last sync {ago(d.last_sync)}") if x))
        self.status_icon.set_from_icon_name(tray.ICONS.get(d.state, "emblem-default"), Gtk.IconSize.DIALOG)
        self.btn_pause.set_label("Resume" if d.paused else "Pause")

    def tick(self):
        self.refresh()
        if (self.stack.get_visible_child_name() == "main") != config.TOKEN.exists():
            self.route()
        return True

    # ----- setup actions -----
    def on_choose_creds(self, *_):
        dlg = Gtk.FileChooserNative(title="Select client_secret JSON", transient_for=self,
                                    action=Gtk.FileChooserAction.OPEN)
        flt = Gtk.FileFilter()
        flt.set_name("JSON")
        flt.add_pattern("*.json")
        dlg.add_filter(flt)
        if dlg.run() == Gtk.ResponseType.ACCEPT:
            try:
                data = json.load(open(dlg.get_filename()))
                if "installed" not in data:
                    raise ValueError("That isn't a Desktop-app OAuth client (expected an 'installed' section).")
                config.CONFIG_DIR.mkdir(parents=True, exist_ok=True)
                shutil.copy(dlg.get_filename(), config.CLIENT_SECRET)
                os.chmod(config.CLIENT_SECRET, 0o600)
                self.setup_err.set_text("")
            except (OSError, ValueError) as e:
                self.setup_err.set_text(f"Couldn't use that file: {e}")
        self.route()

    def on_signin(self, btn):
        btn.set_sensitive(False)
        self.setup_err.set_text("Waiting for you to finish in the browser…")

        def work():
            try:
                auth.login()
                err = None
            except Exception as e:  # noqa: BLE001
                err = str(e)
            GLib.idle_add(self.signin_done, err)
        threading.Thread(target=work, daemon=True).start()

    def signin_done(self, err):
        self.btn_signin.set_sensitive(True)
        self.setup_err.set_text(f"Sign-in failed: {err}" if err else "")
        if not err:
            self.daemon.sync_now()
        self.route()

    def on_signout(self, *_):
        auth.logout()
        self.store.clear()
        self.route()

    # ----- folder tree -----
    def load_children(self, parent_iter, folder_id):
        """Fetch subfolders in a thread; fill the store when done. folder_id None = My Drive root."""
        tpath = parent_iter and self.store.get_path(parent_iter)
        path = self.store.get_value(parent_iter, COL_PATH) if parent_iter else ""

        def work():
            try:
                if folder_id == COMPUTERS_ID:
                    subs = self.drive.computers()
                else:
                    subs = self.drive.subfolders(folder_id or self.drive.root_id())
                err = None
            except Exception as e:  # noqa: BLE001
                subs, err = [], str(e)
            GLib.idle_add(self.fill, tpath, path, subs, err)
        threading.Thread(target=work, daemon=True).start()

    def fill(self, tpath, path, subs, err):
        parent = self.store.get_iter(tpath) if tpath is not None else None
        if parent is not None:  # drop the "Loading…" placeholder
            ch = self.store.iter_children(parent)
            while ch:
                self.store.remove(ch)
                ch = self.store.iter_children(parent)
            self.store.set_value(parent, COL_LOADED, True)
        elif self.store.iter_n_children(None):
            return
        if err:
            self.sub_lbl.set_text("Couldn't list folders: " + err[:80])
        under_computers = parent is not None and self.store.get_value(parent, COL_ID) == COMPUTERS_ID
        for s in subs:
            full = f"{COMPUTERS}/{s['name']}" if under_computers else (f"{path}/{s['name']}" if path else s["name"])
            it = self.store.append(parent, [False, s["name"], full, False, False, True, s["id"], ""])
            self.store.append(it, [False, "Loading…", "", True, False, False, "", ""])
        if parent is None and not err:
            top = self.store.append(None, [False, COMPUTERS, COMPUTERS, False, False, False, COMPUTERS_ID, "backups from your computers"])
            self.store.append(top, [False, "Loading…", "", True, False, False, "", ""])
        self.apply_selection()
        if parent is not None:  # removing the placeholder collapsed the row
            self.tree.expand_row(tpath, False)
        return False

    def on_expand(self, tree, it, tpath):
        if not self.store.get_value(it, COL_LOADED):
            self.load_children(it, self.store.get_value(it, COL_ID))
        return False

    def apply_selection(self):
        pairs = {p["id"]: p for p in self.daemon.settings["pairs"]}
        paths = [p["path"] for p in pairs.values()]

        def walk(it, inherited):
            while it:
                fid, path = self.store.get_value(it, COL_ID), self.store.get_value(it, COL_PATH)
                if fid == COMPUTERS_ID:
                    walk(self.store.iter_children(it), inherited)
                elif fid:
                    own = fid in pairs
                    below = any(q.startswith(path + "/") for q in paths)
                    self.store.set_value(it, COL_CHECK, own or inherited)
                    self.store.set_value(it, COL_INCONS, below and not (own or inherited))
                    self.store.set_value(it, COL_SENS, not inherited)  # inside a synced folder: comes along
                    self.store.set_value(it, COL_DEST, (f"→ {config.local_dir(pairs[fid])}  ·  {config.mode_label(pairs[fid])}") if own else "")
                    walk(self.store.iter_children(it), inherited or own)
                it = self.store.iter_next(it)
        walk(self.store.get_iter_first(), False)

    def message(self, title, text, kind=Gtk.MessageType.INFO, buttons=Gtk.ButtonsType.OK):
        dlg = Gtk.MessageDialog(transient_for=self, modal=True, message_type=kind, buttons=buttons, text=title)
        dlg.format_secondary_text(text)
        r = dlg.run()
        dlg.destroy()
        return r

    def on_toggled(self, _cell, tpath):
        it = self.store.get_iter(tpath)
        fid = self.store.get_value(it, COL_ID)
        path = self.store.get_value(it, COL_PATH)
        name = self.store.get_value(it, COL_NAME)
        if fid == COMPUTERS_ID:
            return
        st = config.load()
        existing = next((p for p in st["pairs"] if p["id"] == fid), None)
        if existing:
            self.stop_syncing(st, existing)
        else:
            self.start_syncing(st, fid, path, name)
        self.daemon.settings = config.load()
        self.apply_selection()
        self.daemon.sync_now()

    def stop_syncing(self, st, pair):
        if self.message(f"Stop syncing “{pair['name']}”?",
                        f"Your files in {config.local_dir(pair)} stay where they are, and nothing is deleted "
                        "on Drive or here. Changes just stop being synced.",
                        Gtk.MessageType.QUESTION, Gtk.ButtonsType.OK_CANCEL) != Gtk.ResponseType.OK:
            return
        st["pairs"] = [p for p in st["pairs"] if p["id"] != pair["id"]]
        config.save(st)
        for suffix in ("", "-wal", "-shm", ".lock"):
            config.db_path(pair).with_name(config.db_path(pair).name + suffix).unlink(missing_ok=True)

    def ask_mode(self, name, is_computer):
        dlg = Gtk.MessageDialog(transient_for=self, modal=True, message_type=Gtk.MessageType.QUESTION,
                                text=f"How should “{name}” sync?")
        dlg.format_secondary_text(
            "Two-way: changes you make here are uploaded to Drive, and Drive changes come down.\n\n"
            "Download only: you get a copy and Drive changes come down, but nothing you do here "
            "(edit, delete, add) ever changes Drive."
            + ("\n\nFor a computer's backup, download only is the safe choice." if is_computer else ""))
        dlg.add_button("Cancel", Gtk.ResponseType.CANCEL)
        two, dl = dlg.add_button("Two-way sync", 1), dlg.add_button("Download only", 2)
        (dl if is_computer else two).get_style_context().add_class("suggested-action")
        r = dlg.run()
        dlg.destroy()
        return {1: "mirror", 2: "download"}.get(r)

    def start_syncing(self, st, fid, path, name):
        is_computer = path.startswith(COMPUTERS + "/")
        dlg = Gtk.FileChooserNative(
            title=f"Choose where “{name}” will be saved", transient_for=self,
            action=Gtk.FileChooserAction.SELECT_FOLDER, accept_label="Next")
        dlg.set_create_folders(True)
        start = st["default_dest"]
        if os.path.isdir(start):
            dlg.set_current_folder(start)
        if dlg.run() != Gtk.ResponseType.ACCEPT:
            return
        parent = dlg.get_filename()
        mode = self.ask_mode(name, is_computer)
        if mode is None:
            return
        pair = config.make_pair(fid, path, parent, mode)
        err = config.check_new_pair(st["pairs"], pair)
        if err:
            return self.message("Can't sync that folder there", err, Gtk.MessageType.ERROR)
        target = config.local_dir(pair)
        if os.path.isdir(target) and os.listdir(target) and self.message(
                "That folder already has files",
                f"{target} isn't empty. Its files will be merged with the Drive folder: identical files are "
                "matched" + ("." if mode == "download" else ", and any other files in it are uploaded to Drive.")
                + "\n\nContinue?",
                Gtk.MessageType.WARNING, Gtk.ButtonsType.OK_CANCEL) != Gtk.ResponseType.OK:
            return
        st["pairs"].append(pair)
        st["default_dest"] = parent
        config.save(st)

    def on_add_link(self, *_):
        dlg = Gtk.MessageDialog(transient_for=self, modal=True, message_type=Gtk.MessageType.QUESTION,
                                buttons=Gtk.ButtonsType.OK_CANCEL, text="Add a folder by link")
        dlg.format_secondary_text("Open the folder in drive.google.com and paste its address here.")
        entry = Gtk.Entry(activates_default=True, width_chars=50)
        dlg.get_content_area().pack_start(entry, False, False, 6)
        dlg.show_all()
        ok = dlg.run() == Gtk.ResponseType.OK
        text = entry.get_text()
        dlg.destroy()
        if not ok:
            return
        fid = parse_folder_ref(text)
        try:
            meta = self.drive.get(fid) if fid else None
        except Exception as e:  # noqa: BLE001
            return self.message("Couldn't open that folder", str(e)[:200], Gtk.MessageType.ERROR)
        if not meta or meta["mimeType"] != FOLDER:
            return self.message("That isn't a folder link", "Paste the address of a Drive folder.", Gtk.MessageType.ERROR)
        path = f"{COMPUTERS}/{meta['name']}" if not meta.get("parents") else f"(link)/{meta['name']}"
        st = config.load()
        if any(p["id"] == fid for p in st["pairs"]):
            return self.message("Already syncing", f"“{meta['name']}” is already being synced.")
        self.start_syncing(st, fid, path, meta["name"])
        self.daemon.settings = config.load()
        self.apply_selection()
        self.daemon.sync_now()

    def on_row_activated(self, tree, tpath, col):
        it = self.store.get_iter(tpath)
        pair = next((p for p in self.daemon.settings["pairs"] if p["id"] == self.store.get_value(it, COL_ID)), None)
        if pair and os.path.isdir(config.local_dir(pair)):
            subprocess.Popen(["xdg-open", config.local_dir(pair)])

    def on_open_folder(self, btn):
        pairs = self.daemon.settings["pairs"]
        if len(pairs) == 1:
            subprocess.Popen(["xdg-open", config.local_dir(pairs[0])])
        elif pairs:
            menu = Gtk.Menu()
            for p in pairs:
                mi = Gtk.MenuItem(label=f"{p['name']}  →  {config.local_dir(p)}")
                mi.connect("activate", lambda _m, d=config.local_dir(p): subprocess.Popen(["xdg-open", d]))
                menu.append(mi)
            menu.show_all()
            menu.popup_at_widget(btn, Gdk.Gravity.SOUTH_WEST, Gdk.Gravity.NORTH_WEST, None)
        else:
            self.message("Nothing synced yet", "Tick a Drive folder below to start syncing it.")


class App(Gtk.Application):
    def __init__(self, daemon, background):
        super().__init__(application_id="io.github.gdrive_linux", flags=Gio.ApplicationFlags.FLAGS_NONE)
        self.daemon, self.background, self.win, self.refresh_tray = daemon, background, None, None
        self.started = False

    def do_activate(self):
        if not self.started:
            self.started = True
            self.win = Window(self, self.daemon)
            self.win.connect("delete-event", self.on_close)
            self.refresh_tray = tray.attach(self.daemon, self.show, self.quit_all) if tray.available() else None
            prev = self.win.refresh
            self.daemon.on_status = lambda: GLib.idle_add(self.on_status)
            threading.Thread(target=self.daemon.run, daemon=True).start()
            self.hold()
            if self.background and self.refresh_tray:
                self.win.hide()
                return
        self.show()

    def on_status(self):
        self.win.refresh()
        if self.refresh_tray:
            self.refresh_tray()
        return False

    def show(self):
        self.win.show_all()
        self.win.route()
        self.win.present()

    def on_close(self, *_):
        if self.refresh_tray:  # keep syncing from the tray
            self.win.hide()
            return True
        self.quit_all()
        return False

    def quit_all(self):
        self.daemon.stop.set()
        self.daemon.wake.set()
        self.release()
        self.quit()


def run(daemon, background=False):
    app = App(daemon, background)
    return app.run(None)
