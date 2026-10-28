//! Platform-independent screen switching for the server.
//!
//! The capture backend feeds every local input event through `ServerCore` and obeys the
//! returned `Verdict`: `Pass` lets the event reach local apps, `Swallow` drops it because it was
//! forwarded to a client. While a client is active the server tracks a virtual cursor on that
//! client's desktop and sends absolute positions, so the client never has to report back.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tokio::sync::mpsc::UnboundedSender;

use crate::geom::{Screen, Side};
use crate::keymap;
use crate::proto::{Button, Msg};

/// What the capture backend does to the local cursor when control moves between machines.
pub trait CaptureCtl: Send + Sync {
    /// A client took over: freeze and hide the local cursor, keep delivering relative motion.
    fn enter_remote(&self);
    /// Control came back: show the local cursor at (x, y).
    fn leave_remote(&self, x: f64, y: f64);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Swallow,
}

#[derive(Clone, Debug)]
pub struct ClientSettings {
    /// Which edge of the server's desktop the client sits beyond.
    pub side: Side,
    /// Multiplier applied to mouse motion while on the client.
    pub speed: f64,
    /// Swap Command/Super with Control (for a Mac keyboard driving Linux/Windows or vice versa).
    pub swap_cmd_ctrl: bool,
}

struct Slot {
    conn: u64,
    name: String,
    screen: Screen,
    settings: ClientSettings,
    tx: UnboundedSender<Msg>,
}

struct Remote {
    conn: u64,
    x: f64,
    y: f64,
    buttons: u8,
}

/// Where a pressed key's release has to go.
#[derive(Clone, Copy)]
struct KeyRoute {
    conn: u64,
    /// A modifier already held locally when the cursor crossed: release it on both sides.
    also_local: bool,
}

pub struct ServerCore {
    screen: Screen,
    slots: Vec<Slot>,
    remote: Option<Remote>,
    local_buttons: u8,
    held_local: HashSet<u16>,
    remote_keys: HashMap<u16, KeyRoute>,
    ctl: Option<Arc<dyn CaptureCtl>>,
    next_conn: u64,
    /// Told the name of the client that has the cursor (None: this machine) when it changes.
    on_active: Option<Box<dyn Fn(Option<&str>) + Send>>,
}

/// Pixels between the desktop edge and where the cursor reappears, so it does not bounce back.
const RETURN_INSET: f64 = 2.0;

impl ServerCore {
    pub fn new(screen: Screen) -> Self {
        ServerCore {
            screen,
            slots: Vec::new(),
            remote: None,
            local_buttons: 0,
            held_local: HashSet::new(),
            remote_keys: HashMap::new(),
            ctl: None,
            next_conn: 1,
            on_active: None,
        }
    }

    pub fn set_active_observer(&mut self, f: Box<dyn Fn(Option<&str>) + Send>) {
        self.on_active = Some(f);
    }

    fn active_changed(&self) {
        if let Some(f) = &self.on_active {
            let conn = self.remote.as_ref().map(|r| r.conn);
            f(self.slots.iter().find(|s| Some(s.conn) == conn).map(|s| s.name.as_str()));
        }
    }

    /// Applies new layout settings to a connected client without reconnecting it.
    pub fn set_settings(&mut self, name: &str, settings: ClientSettings) {
        if let Some(slot) = self.slots.iter_mut().find(|s| s.name == name) {
            slot.settings = settings;
        }
    }

    pub fn set_ctl(&mut self, ctl: Arc<dyn CaptureCtl>) {
        self.ctl = Some(ctl);
    }

    pub fn set_local_screen(&mut self, screen: Screen) {
        self.screen = screen;
    }

    pub fn local_screen(&self) -> &Screen {
        &self.screen
    }

    pub fn is_remote(&self) -> bool {
        self.remote.is_some()
    }

    /// Registers a connected client and returns its connection id. A client reconnecting under the
    /// same name replaces its old link immediately instead of being refused, and keeps control
    /// if it had it.
    pub fn attach(&mut self, name: &str, screen: Screen, settings: ClientSettings, tx: UnboundedSender<Msg>) -> u64 {
        let conn = self.next_conn;
        self.next_conn += 1;
        if let Some(i) = self.slots.iter().position(|s| s.name == name) {
            let old = self.slots.remove(i);
            let _ = old.tx.send(Msg::Bye("replaced by a newer connection".into()));
            self.remote_keys.retain(|_, r| r.conn != old.conn);
            if let Some(r) = self.remote.as_mut().filter(|r| r.conn == old.conn) {
                r.conn = conn;
                r.buttons = 0;
                (r.x, r.y) = screen.nearest(r.x, r.y);
                let _ = tx.send(Msg::Enter { x: r.x, y: r.y });
            }
            log::info!("{name}: reconnected, replacing its previous link");
        }
        if let Some(other) = self.slots.iter().find(|s| s.settings.side == settings.side) {
            log::warn!("{name} and {} are both on the {} edge; {} wins", other.name, settings.side, other.name);
        }
        self.slots.push(Slot { conn, name: name.to_string(), screen, settings, tx });
        conn
    }

    /// Forgets a client's link. If it had control, the cursor comes back to this machine.
    pub fn detach(&mut self, conn: u64) {
        let Some(i) = self.slots.iter().position(|s| s.conn == conn) else { return };
        let slot = self.slots.remove(i);
        self.remote_keys.retain(|_, r| r.conn != conn);
        if let Some(r) = self.remote.take_if(|r| r.conn == conn) {
            log::info!("{}: link lost while it had control; taking the cursor back", slot.name);
            let back = slot.settings.side.opposite();
            let t = slot.screen.fraction(back, r.x, r.y);
            self.return_local(slot.settings.side, t);
            self.active_changed();
        }
    }

    /// Gives the cursor back to this machine before exiting.
    pub fn shutdown(&mut self) {
        let conns: Vec<u64> = self.slots.iter().map(|s| s.conn).collect();
        for conn in conns {
            self.send_to(conn, Msg::Bye("server shutting down".into()));
            self.detach(conn);
        }
    }

    pub fn update_screen(&mut self, conn: u64, screen: Screen) {
        let Some(slot) = self.slots.iter_mut().find(|s| s.conn == conn) else { return };
        if let Some(r) = self.remote.as_mut().filter(|r| r.conn == conn) {
            (r.x, r.y) = screen.nearest(r.x, r.y);
            let _ = slot.tx.send(Msg::Motion { x: r.x, y: r.y });
        }
        slot.screen = screen;
    }

    /// Sends a message to every connected client except `except`.
    pub fn broadcast(&self, msg: &Msg, except: Option<u64>) {
        for s in self.slots.iter().filter(|s| Some(s.conn) != except) {
            let _ = s.tx.send(msg.clone());
        }
    }

    pub fn send_to(&self, conn: u64, msg: Msg) {
        if let Some(s) = self.slots.iter().find(|s| s.conn == conn) {
            let _ = s.tx.send(msg);
        }
    }

    /// Mouse motion: (x, y) is the local cursor position, (dx, dy) the raw movement.
    pub fn mouse(&mut self, x: f64, y: f64, dx: f64, dy: f64) -> Verdict {
        let Some(r) = self.remote.as_mut() else {
            if self.local_buttons != 0 {
                return Verdict::Pass;
            }
            let hit = self.slots.iter().position(|s| self.screen.at_edge(s.settings.side, x, y) && s.settings.side.pushing(dx, dy));
            return match hit {
                Some(i) => {
                    self.enter(i, x, y);
                    Verdict::Swallow
                }
                None => Verdict::Pass,
            };
        };

        let Some(slot) = self.slots.iter().find(|s| s.conn == r.conn) else {
            self.remote = None;
            return Verdict::Pass;
        };
        let (nx, ny) = (r.x + dx * slot.settings.speed, r.y + dy * slot.settings.speed);
        let back = slot.settings.side.opposite();
        if r.buttons == 0 && back.beyond(&slot.screen.bbox(), nx, ny) {
            let t = slot.screen.fraction(back, nx, ny);
            let side = slot.settings.side;
            let _ = slot.tx.send(Msg::Leave);
            log::info!("← back from {}", slot.name);
            self.remote = None;
            self.return_local(side, t);
            self.active_changed();
            return Verdict::Swallow;
        }
        let (nx, ny) = if slot.screen.monitor_at(nx, ny).is_some() {
            (nx, ny)
        } else {
            match slot.screen.monitor_at(r.x, r.y) {
                Some(m) => m.clamp(nx, ny),
                None => slot.screen.nearest(nx, ny),
            }
        };
        if (nx, ny) != (r.x, r.y) {
            (r.x, r.y) = (nx, ny);
            let _ = slot.tx.send(Msg::Motion { x: nx, y: ny });
        }
        Verdict::Swallow
    }

    pub fn button(&mut self, button: Button, down: bool) -> Verdict {
        let Some(r) = self.remote.as_mut() else {
            if down {
                self.local_buttons |= button.bit();
            } else {
                self.local_buttons &= !button.bit();
            }
            return Verdict::Pass;
        };
        if down {
            r.buttons |= button.bit();
        } else if r.buttons & button.bit() == 0 {
            // Pressed before crossing over: the release belongs to the local app.
            self.local_buttons &= !button.bit();
            return Verdict::Pass;
        } else {
            r.buttons &= !button.bit();
        }
        self.send_active(Msg::Button { button, down });
        Verdict::Swallow
    }

    pub fn wheel(&mut self, dx: i32, dy: i32) -> Verdict {
        if self.remote.is_none() {
            return Verdict::Pass;
        }
        if dx != 0 || dy != 0 {
            self.send_active(Msg::Wheel { dx, dy });
        }
        Verdict::Swallow
    }

    /// A key event. `code` is an evdev key code from this machine's keyboard.
    pub fn key(&mut self, code: u16, down: bool, repeat: bool) -> Verdict {
        if !down {
            self.held_local.remove(&code);
            let Some(route) = self.remote_keys.remove(&code) else { return Verdict::Pass };
            self.send_key(route.conn, code, false, false);
            return if route.also_local { Verdict::Pass } else { Verdict::Swallow };
        }
        let Some(conn) = self.remote.as_ref().map(|r| r.conn) else {
            self.held_local.insert(code);
            return Verdict::Pass;
        };
        let repeat = repeat || self.remote_keys.contains_key(&code);
        if !repeat {
            self.remote_keys.insert(code, KeyRoute { conn, also_local: false });
        }
        self.send_key(conn, code, true, repeat);
        Verdict::Swallow
    }

    fn send_key(&self, conn: u64, code: u16, down: bool, repeat: bool) {
        if let Some(s) = self.slots.iter().find(|s| s.conn == conn) {
            let code = if s.settings.swap_cmd_ctrl { keymap::swap_cmd_ctrl(code) } else { code };
            let _ = s.tx.send(Msg::Key { code, down, repeat });
        }
    }

    fn send_active(&self, msg: Msg) {
        if let Some(r) = &self.remote {
            self.send_to(r.conn, msg);
        }
    }

    fn enter(&mut self, i: usize, x: f64, y: f64) {
        let slot = &self.slots[i];
        let side = slot.settings.side;
        let t = self.screen.fraction(side, x, y);
        let (cx, cy) = slot.screen.edge_point(side.opposite(), t, 0.0);
        let conn = slot.conn;
        let _ = slot.tx.send(Msg::Enter { x: cx, y: cy });
        log::info!("→ {} ({side} edge)", slot.name);

        // Modifiers held while crossing (e.g. Shift during a drag-select) apply over there too.
        let carried: Vec<u16> = self.held_local.iter().copied().filter(|c| keymap::is_modifier(*c)).collect();
        for code in carried {
            self.remote_keys.insert(code, KeyRoute { conn, also_local: true });
            self.send_key(conn, code, true, false);
        }
        self.remote = Some(Remote { conn, x: cx, y: cy, buttons: 0 });
        if let Some(ctl) = &self.ctl {
            ctl.enter_remote();
        }
        self.active_changed();
    }

    fn return_local(&mut self, side: Side, t: f64) {
        let (x, y) = self.screen.edge_point(side, t, RETURN_INSET);
        if let Some(ctl) = &self.ctl {
            ctl.leave_remote(x, y);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

    use super::*;
    use crate::geom::Rect;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<String>>);

    impl CaptureCtl for Recorder {
        fn enter_remote(&self) {
            self.0.lock().unwrap().push("hide".into());
        }
        fn leave_remote(&self, x: f64, y: f64) {
            self.0.lock().unwrap().push(format!("show {x:.0},{y:.0}"));
        }
    }

    fn settings(side: Side) -> ClientSettings {
        ClientSettings { side, speed: 1.0, swap_cmd_ctrl: false }
    }

    fn mac_with_linux_on_left() -> (ServerCore, Arc<Recorder>, UnboundedReceiver<Msg>, u64) {
        let mut core = ServerCore::new(Screen::single(1440, 900));
        let rec = Arc::new(Recorder::default());
        core.set_ctl(rec.clone());
        let (tx, rx) = unbounded_channel();
        let linux = Screen { monitors: vec![Rect::new(0, 0, 1920, 1080), Rect::new(1920, 0, 1920, 1080)] };
        let conn = core.attach("thinkpad", linux, settings(Side::Left), tx);
        (core, rec, rx, conn)
    }

    fn drain(rx: &mut UnboundedReceiver<Msg>) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(m) = rx.try_recv() {
            out.push(match m {
                Msg::Enter { x, y } => format!("enter {x:.0},{y:.0}"),
                Msg::Motion { x, y } => format!("motion {x:.0},{y:.0}"),
                Msg::Leave => "leave".into(),
                Msg::Key { code, down, repeat } => format!("key {code} {down} {repeat}"),
                Msg::Button { button, down } => format!("button {button:?} {down}"),
                other => format!("{other:?}"),
            });
        }
        out
    }

    #[test]
    fn crosses_on_the_configured_edge_only() {
        let (mut core, rec, mut rx, _) = mac_with_linux_on_left();
        assert_eq!(core.mouse(1439.0, 450.0, 5.0, 0.0), Verdict::Pass, "right edge has no client");
        assert_eq!(core.mouse(0.0, 450.0, 3.0, 0.0), Verdict::Pass, "moving away from the edge");
        assert_eq!(core.mouse(0.0, 450.0, -3.0, 0.0), Verdict::Swallow);
        assert!(core.is_remote());
        // Enters on the far right of the client's desktop, at the same relative height.
        assert_eq!(drain(&mut rx), vec!["enter 3839,540"]);
        assert_eq!(rec.0.lock().unwrap().as_slice(), ["hide"]);
    }

    #[test]
    fn tracks_motion_and_returns_through_the_facing_edge() {
        let (mut core, rec, mut rx, _) = mac_with_linux_on_left();
        core.mouse(0.0, 450.0, -1.0, 0.0);
        drain(&mut rx);
        assert_eq!(core.mouse(0.0, 450.0, -100.0, 20.0), Verdict::Swallow);
        assert_eq!(core.mouse(0.0, 450.0, 0.0, -5000.0), Verdict::Swallow, "clamped at the top");
        assert_eq!(drain(&mut rx), vec!["motion 3739,560", "motion 3739,0"]);
        // Pushing right (towards the server) past the client's right edge comes home.
        assert_eq!(core.mouse(0.0, 0.0, 200.0, 0.0), Verdict::Swallow);
        assert!(!core.is_remote());
        assert_eq!(drain(&mut rx), vec!["leave"]);
        assert_eq!(rec.0.lock().unwrap().last().unwrap(), "show 2,0");
    }

    #[test]
    fn does_not_leave_while_dragging_on_the_client() {
        let (mut core, _, mut rx, _) = mac_with_linux_on_left();
        core.mouse(0.0, 450.0, -1.0, 0.0);
        core.button(Button::Left, true);
        core.mouse(0.0, 0.0, 5000.0, 0.0);
        assert!(core.is_remote());
        core.button(Button::Left, false);
        core.mouse(0.0, 0.0, 5.0, 0.0);
        assert!(!core.is_remote());
        assert_eq!(drain(&mut rx), vec!["enter 3839,540", "button Left true", "button Left false", "leave"]);
    }

    #[test]
    fn key_release_follows_its_press() {
        let (mut core, _, mut rx, _) = mac_with_linux_on_left();
        assert_eq!(core.key(keymap::KEY_A, true, false), Verdict::Pass);
        core.mouse(0.0, 450.0, -1.0, 0.0);
        // `a` was pressed locally, so its release stays local.
        assert_eq!(core.key(keymap::KEY_A, false, false), Verdict::Pass);
        assert_eq!(core.key(keymap::KEY_B, true, false), Verdict::Swallow);
        assert_eq!(core.key(keymap::KEY_B, true, false), Verdict::Swallow);
        core.mouse(0.0, 0.0, 5000.0, 0.0);
        // `b` went to the client, so its release does too even after coming back.
        assert_eq!(core.key(keymap::KEY_B, false, false), Verdict::Swallow);
        let log = drain(&mut rx);
        assert_eq!(log[1..], ["key 48 true false", "key 48 true true", "leave", "key 48 false false"]);
    }

    #[test]
    fn carries_held_modifiers_and_swaps_cmd_ctrl() {
        let mut core = ServerCore::new(Screen::single(1440, 900));
        let (tx, mut rx) = unbounded_channel();
        core.attach("pc", Screen::single(1920, 1080), ClientSettings { swap_cmd_ctrl: true, ..settings(Side::Right) }, tx);
        core.key(keymap::KEY_LEFTMETA, true, false);
        core.mouse(1439.0, 0.0, 1.0, 0.0);
        assert_eq!(core.key(keymap::KEY_C, true, false), Verdict::Swallow);
        // A carried modifier's release goes to both machines.
        assert_eq!(core.key(keymap::KEY_LEFTMETA, false, false), Verdict::Pass);
        assert_eq!(drain(&mut rx), vec!["enter 0,0", "key 29 true false", "key 46 true false", "key 29 false false"]);
    }

    #[test]
    fn link_loss_returns_the_cursor_and_reconnect_keeps_control() {
        let (mut core, rec, _rx, conn) = mac_with_linux_on_left();
        core.mouse(0.0, 450.0, -1.0, 0.0);
        let (tx2, mut rx2) = unbounded_channel();
        let conn2 = core.attach("thinkpad", Screen::single(1920, 1080), settings(Side::Left), tx2);
        assert_ne!(conn, conn2);
        assert!(core.is_remote(), "a reconnect under the same name keeps control");
        assert_eq!(drain(&mut rx2), vec!["enter 1919,540"]);
        core.detach(conn); // the stale connection id is a no-op
        assert!(core.is_remote());
        core.detach(conn2);
        assert!(!core.is_remote());
        assert_eq!(core.mouse(500.0, 500.0, 1.0, 0.0), Verdict::Pass);
        assert!(rec.0.lock().unwrap().last().unwrap().starts_with("show 2,"));
    }
}
