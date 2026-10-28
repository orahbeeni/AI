//! Linux: input injection through uinput (works on Wayland and X11 alike, no portal prompts),
//! monitor layout and clipboard changes through X11/XWayland.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xfixes::{ConnectionExt as _, SelectionEventMask};
use x11rb::protocol::xproto::{ConnectionExt as _, CreateWindowAux, WindowClass};

use super::{ClipboardWatch, Injector};
use crate::geom::{Rect, Screen};
use crate::proto::Button;
use crate::server::core::{CaptureCtl, ServerCore};

const UI_SET_EVBIT: u64 = 0x4004_5564;
const UI_SET_KEYBIT: u64 = 0x4004_5565;
const UI_SET_RELBIT: u64 = 0x4004_5566;
const UI_SET_ABSBIT: u64 = 0x4004_5567;
const UI_DEV_SETUP: u64 = 0x405c_5503;
const UI_ABS_SETUP: u64 = 0x401c_5504;
const UI_DEV_CREATE: u64 = 0x5501;
const UI_DEV_DESTROY: u64 = 0x5502;

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const EV_ABS: u16 = 3;
const REL_HWHEEL: u16 = 6;
const REL_WHEEL: u16 = 8;
const REL_WHEEL_HI_RES: u16 = 11;
const REL_HWHEEL_HI_RES: u16 = 12;
const ABS_X: u16 = 0;
const ABS_Y: u16 = 1;
const BTN_LEFT: u16 = 0x110;
const BTN_RIGHT: u16 = 0x111;
const BTN_MIDDLE: u16 = 0x112;
const BTN_SIDE: u16 = 0x113;
const BTN_EXTRA: u16 = 0x114;
const BUS_VIRTUAL: u16 = 0x06;
/// Highest key code given to the virtual keyboard (KEY_MICMUTE).
const MAX_KEY: u16 = 248;
const ABS_MAX: i32 = 65535;

const UINPUT_FIX: &str = "Allow it with:\n  \
    echo 'KERNEL==\"uinput\", SUBSYSTEM==\"misc\", TAG+=\"uaccess\", OPTIONS+=\"static_node=uinput\"' \
    | sudo tee /etc/udev/rules.d/60-wifishare-uinput.rules\n  \
    sudo udevadm control --reload && sudo udevadm trigger --sysname-match=uinput\n\
    then log out and back in.";

#[repr(C)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

#[repr(C)]
struct UinputSetup {
    id: InputId,
    name: [u8; 80],
    ff_effects_max: u32,
}

#[repr(C)]
struct AbsInfo {
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

#[repr(C)]
struct UinputAbsSetup {
    code: u16,
    absinfo: AbsInfo,
}

struct Device {
    file: File,
}

impl Device {
    fn create(name: &str, product: u16, configure: impl FnOnce(&File) -> std::io::Result<()>) -> Result<Device> {
        let file = OpenOptions::new().write(true).open("/dev/uinput").with_context(|| format!("cannot open /dev/uinput. {UINPUT_FIX}"))?;
        configure(&file).with_context(|| format!("configuring {name}"))?;
        let mut setup = UinputSetup {
            id: InputId { bustype: BUS_VIRTUAL, vendor: 0x5753, product, version: 1 },
            name: [0; 80],
            ff_effects_max: 0,
        };
        setup.name[..name.len()].copy_from_slice(name.as_bytes());
        ioctl_ptr(&file, UI_DEV_SETUP, &setup).with_context(|| format!("setting up {name}"))?;
        ioctl(&file, UI_DEV_CREATE, 0).with_context(|| format!("creating {name}"))?;
        Ok(Device { file })
    }

    /// Writes the events followed by a SYN_REPORT in one go.
    fn emit(&mut self, events: &[(u16, u16, i32)]) {
        let mut buf = Vec::with_capacity((events.len() + 1) * std::mem::size_of::<libc::input_event>());
        for &(type_, code, value) in events.iter().chain(std::iter::once(&(EV_SYN, 0, 0))) {
            let ev = libc::input_event { time: libc::timeval { tv_sec: 0, tv_usec: 0 }, type_, code, value };
            // SAFETY: input_event is plain old data.
            let bytes = unsafe { std::slice::from_raw_parts(&ev as *const _ as *const u8, std::mem::size_of::<libc::input_event>()) };
            buf.extend_from_slice(bytes);
        }
        if let Err(e) = self.file.write_all(&buf) {
            log::warn!("uinput write failed: {e}");
        }
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        let _ = ioctl(&self.file, UI_DEV_DESTROY, 0);
    }
}

fn ioctl(file: &File, request: u64, arg: libc::c_ulong) -> std::io::Result<()> {
    // SAFETY: uinput ioctls that take an integer argument.
    let r = unsafe { libc::ioctl(file.as_raw_fd(), request as _, arg) };
    if r < 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
}

fn ioctl_ptr<T>(file: &File, request: u64, arg: &T) -> std::io::Result<()> {
    // SAFETY: `arg` matches the struct the request expects.
    let r = unsafe { libc::ioctl(file.as_raw_fd(), request as _, arg as *const T) };
    if r < 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
}

pub struct UinputInjector {
    keyboard: Device,
    pointer: Device,
    bbox: Rect,
    scroll_speed: f64,
    /// Hi-res wheel units not yet emitted as a whole legacy notch.
    wheel_rest: (i32, i32),
}

pub fn injector(scroll_speed: f64) -> Result<Box<dyn Injector>> {
    let keyboard = Device::create("wifishare keyboard", 1, |f| {
        ioctl(f, UI_SET_EVBIT, EV_KEY as _)?;
        for code in 1..=MAX_KEY {
            ioctl(f, UI_SET_KEYBIT, code as _)?;
        }
        Ok(())
    })?;
    // Absolute axes plus mouse buttons: udev and libinput treat this like a VM's USB tablet, an
    // absolute pointer spanning the whole desktop.
    let pointer = Device::create("wifishare pointer", 2, |f| {
        ioctl(f, UI_SET_EVBIT, EV_KEY as _)?;
        for b in [BTN_LEFT, BTN_RIGHT, BTN_MIDDLE, BTN_SIDE, BTN_EXTRA] {
            ioctl(f, UI_SET_KEYBIT, b as _)?;
        }
        ioctl(f, UI_SET_EVBIT, EV_REL as _)?;
        for r in [REL_WHEEL, REL_HWHEEL, REL_WHEEL_HI_RES, REL_HWHEEL_HI_RES] {
            ioctl(f, UI_SET_RELBIT, r as _)?;
        }
        ioctl(f, UI_SET_EVBIT, EV_ABS as _)?;
        for axis in [ABS_X, ABS_Y] {
            ioctl(f, UI_SET_ABSBIT, axis as _)?;
            let setup = UinputAbsSetup { code: axis, absinfo: AbsInfo { value: 0, minimum: 0, maximum: ABS_MAX, fuzz: 0, flat: 0, resolution: 0 } };
            ioctl_ptr(f, UI_ABS_SETUP, &setup)?;
        }
        Ok(())
    })?;
    // The compositor needs a moment to pick up new devices before their events count.
    std::thread::sleep(Duration::from_millis(300));
    Ok(Box::new(UinputInjector { keyboard, pointer, bbox: super::fallback_screen().bbox(), scroll_speed, wheel_rest: (0, 0) }))
}

fn button_code(b: Button) -> u16 {
    match b {
        Button::Left => BTN_LEFT,
        Button::Right => BTN_RIGHT,
        Button::Middle => BTN_MIDDLE,
        Button::Back => BTN_SIDE,
        Button::Forward => BTN_EXTRA,
    }
}

impl Injector for UinputInjector {
    fn set_screen(&mut self, screen: &Screen) {
        self.bbox = screen.bbox();
    }

    fn motion(&mut self, x: f64, y: f64) {
        // libinput maps the axis range onto the desktop's bounding box: pixel = value * size / 65536.
        let scale = |v: f64, origin: i32, size: i32| (((v - origin as f64 + 0.5) * 65536.0 / size.max(1) as f64) as i32).clamp(0, ABS_MAX);
        let (ax, ay) = (scale(x, self.bbox.x, self.bbox.w), scale(y, self.bbox.y, self.bbox.h));
        self.pointer.emit(&[(EV_ABS, ABS_X, ax), (EV_ABS, ABS_Y, ay)]);
    }

    fn button(&mut self, button: Button, down: bool) {
        self.pointer.emit(&[(EV_KEY, button_code(button), down as i32)]);
    }

    fn wheel(&mut self, dx: i32, dy: i32) {
        let dx = (dx as f64 * self.scroll_speed).round() as i32;
        let dy = (dy as f64 * self.scroll_speed).round() as i32;
        let mut events = Vec::new();
        if dy != 0 {
            self.wheel_rest.1 += dy;
            events.push((EV_REL, REL_WHEEL_HI_RES, dy));
            let notches = self.wheel_rest.1 / 120;
            if notches != 0 {
                self.wheel_rest.1 -= notches * 120;
                events.push((EV_REL, REL_WHEEL, notches));
            }
        }
        if dx != 0 {
            self.wheel_rest.0 += dx;
            events.push((EV_REL, REL_HWHEEL_HI_RES, dx));
            let notches = self.wheel_rest.0 / 120;
            if notches != 0 {
                self.wheel_rest.0 -= notches * 120;
                events.push((EV_REL, REL_HWHEEL, notches));
            }
        }
        if !events.is_empty() {
            self.pointer.emit(&events);
        }
    }

    fn key(&mut self, code: u16, down: bool) {
        if (1..=MAX_KEY).contains(&code) {
            self.keyboard.emit(&[(EV_KEY, code, down as i32)]);
        }
    }

    fn os_repeats_keys(&self) -> bool {
        true
    }
}

/// Monitor layout from RandR (XWayland mirrors the compositor's logical layout).
pub fn screen() -> Result<Screen> {
    let (conn, n) = x11rb::connect(None).context("no X11/XWayland display (DISPLAY unset?)")?;
    let root = conn.setup().roots[n].root;
    let reply = conn.randr_get_monitors(root, true)?.reply()?;
    let monitors: Vec<Rect> = reply.monitors.iter().map(|m| Rect::new(m.x as i32, m.y as i32, m.width as i32, m.height as i32)).collect();
    if monitors.is_empty() {
        bail!("RandR reported no monitors");
    }
    Ok(Screen { monitors })
}

#[derive(Clone)]
pub struct XfixesWatch {
    count: Arc<AtomicU64>,
    alive: Arc<AtomicBool>,
}

static XFIXES: OnceLock<XfixesWatch> = OnceLock::new();

/// Counts CLIPBOARD owner changes. Wayland apps' copies reach X11 through XWayland's bridge.
/// One watcher thread serves every restart of the engine.
pub fn clipboard_watch() -> Box<dyn super::ClipboardWatch> {
    Box::new(XFIXES.get_or_init(start_xfixes_watch).clone())
}

fn start_xfixes_watch() -> XfixesWatch {
    let count = Arc::new(AtomicU64::new(0));
    let alive = Arc::new(AtomicBool::new(false));
    let (c, a) = (count.clone(), alive.clone());
    let setup = move || -> Result<()> {
        let (conn, n) = x11rb::connect(None)?;
        conn.xfixes_query_version(5, 0)?.reply()?;
        let root = conn.setup().roots[n].root;
        let win = conn.generate_id()?;
        conn.create_window(0, win, root, 0, 0, 1, 1, 0, WindowClass::INPUT_ONLY, 0, &CreateWindowAux::new())?;
        let clipboard = conn.intern_atom(false, b"CLIPBOARD")?.reply()?.atom;
        let mask = SelectionEventMask::SET_SELECTION_OWNER | SelectionEventMask::SELECTION_WINDOW_DESTROY | SelectionEventMask::SELECTION_CLIENT_CLOSE;
        conn.xfixes_select_selection_input(win, clipboard, mask)?;
        conn.flush()?;
        a.store(true, Ordering::SeqCst);
        while let Ok(ev) = conn.wait_for_event() {
            if let Event::XfixesSelectionNotify(_) = ev {
                c.fetch_add(1, Ordering::SeqCst);
            }
        }
        Ok(())
    };
    let a = alive.clone();
    std::thread::Builder::new()
        .name("clipboard-watch".into())
        .spawn(move || {
            if let Err(e) = setup() {
                log::warn!("clipboard: X11 change notifications unavailable ({e:#}); polling instead");
            }
            a.store(false, Ordering::SeqCst);
        })
        .ok();
    std::thread::sleep(Duration::from_millis(100));
    XfixesWatch { count, alive }
}

impl ClipboardWatch for XfixesWatch {
    fn change_count(&mut self) -> Option<u64> {
        self.alive.load(Ordering::SeqCst).then(|| self.count.load(Ordering::SeqCst))
    }
}

/// Capturing input on Linux needs X11 grabs or, on Wayland, the InputCapture portal; neither
/// is implemented yet, so a Linux server only shares the clipboard.
pub fn start_capture(_core: Arc<Mutex<ServerCore>>) -> Result<Arc<dyn CaptureCtl>> {
    bail!("sharing this machine's keyboard and mouse is not supported on Linux yet (macOS and Windows can be servers)")
}

/// Problems that would stop this machine from working in `role`, as user-facing hints.
pub fn permission_problems(server: bool) -> Vec<String> {
    let mut out = Vec::new();
    if !server && OpenOptions::new().write(true).open("/dev/uinput").is_err() {
        out.push(format!("cannot write /dev/uinput, so input cannot be injected. {UINPUT_FIX}"));
    }
    if std::env::var_os("DISPLAY").is_none() {
        out.push("DISPLAY is not set: clipboard sharing and monitor detection need X11 or XWayland".into());
    }
    out
}
