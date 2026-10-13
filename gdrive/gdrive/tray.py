"""Optional tray icon (StatusNotifier via AppIndicator). Needs:
   sudo apt install gir1.2-ayatana-appindicator3-0.1"""
import importlib
import subprocess

ICONS = {"idle": "emblem-default", "syncing": "emblem-synchronizing",
         "paused": "media-playback-pause", "error": "dialog-warning", "starting": "emblem-synchronizing"}


def _indicator_module():
    import gi
    for ns in ("AyatanaAppIndicator3", "AppIndicator3"):
        try:
            gi.require_version(ns, "0.1")
            return importlib.import_module(f"gi.repository.{ns}")
        except (ValueError, ImportError):
            continue
    return None


def available():
    try:
        return _indicator_module() is not None
    except ImportError:
        return False


def attach(daemon, show_window, quit_app):
    """Create the tray icon. Returns a refresh() callable, or None if unavailable."""
    import gi
    gi.require_version("Gtk", "3.0")
    from gi.repository import Gtk
    AI = _indicator_module()
    if AI is None:
        return None
    ind = AI.Indicator.new("gdrive-linux", ICONS["starting"], AI.IndicatorCategory.APPLICATION_STATUS)
    ind.set_status(AI.IndicatorStatus.ACTIVE)
    menu = Gtk.Menu()
    status_item = Gtk.MenuItem(label="Starting...")
    status_item.set_sensitive(False)
    entries = [status_item, Gtk.SeparatorMenuItem()]

    def add(label, fn):
        it = Gtk.MenuItem(label=label)
        it.connect("activate", lambda *_: fn())
        entries.append(it)
        return it

    add("Open Google Drive window", show_window)
    add("Open drive.google.com", lambda: subprocess.Popen(["xdg-open", "https://drive.google.com"]))
    entries.append(Gtk.SeparatorMenuItem())
    add("Sync now", daemon.sync_now)
    pause = add("Pause syncing", daemon.toggle_pause)
    entries.append(Gtk.SeparatorMenuItem())
    add("Quit", quit_app)
    for e in entries:
        menu.append(e)
    menu.show_all()
    ind.set_menu(menu)

    def refresh():
        status_item.set_label(daemon.summary()[:70])
        pause.set_label("Resume syncing" if daemon.paused else "Pause syncing")
        ind.set_icon_full(ICONS.get(daemon.state, "emblem-default"), daemon.state)
    return refresh
