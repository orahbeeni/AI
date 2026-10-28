//! macOS: input capture with a Quartz event tap, injection with CGEventPost, clipboard changes
//! via NSPasteboard's change count. Both directions need the Accessibility permission for the
//! app that runs wifishare (Terminal, iTerm, ...).

use std::ffi::{CStr, c_char, c_void};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};

use super::{ClipboardWatch, Injector};
use crate::geom::{Rect, Screen};
use crate::keymap;
use crate::proto::Button;
use crate::server::core::{CaptureCtl, ServerCore, Verdict};

type CGEventRef = *mut c_void;
type CFMachPortRef = *mut c_void;
type CFRunLoopSourceRef = *mut c_void;
type CFRunLoopRef = *mut c_void;
type CFStringRef = *const c_void;
type CFTypeRef = *const c_void;
type CFDictionaryRef = *const c_void;
type CGEventSourceRef = *mut c_void;
type CGDirectDisplayID = u32;
type TapCallback = extern "C" fn(*mut c_void, u32, CGEventRef, *mut c_void) -> CGEventRef;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(tap: u32, place: u32, options: u32, mask: u64, callback: TapCallback, user: *mut c_void) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventGetLocation(e: CGEventRef) -> CGPoint;
    fn CGEventGetIntegerValueField(e: CGEventRef, field: u32) -> i64;
    fn CGEventGetDoubleValueField(e: CGEventRef, field: u32) -> f64;
    fn CGEventGetFlags(e: CGEventRef) -> u64;
    fn CGEventSetFlags(e: CGEventRef, flags: u64);
    fn CGEventSetIntegerValueField(e: CGEventRef, field: u32, value: i64);
    fn CGWarpMouseCursorPosition(p: CGPoint) -> i32;
    fn CGAssociateMouseAndMouseCursorPosition(connected: u32) -> i32;
    fn CGDisplayHideCursor(d: CGDirectDisplayID) -> i32;
    fn CGDisplayShowCursor(d: CGDirectDisplayID) -> i32;
    fn CGMainDisplayID() -> CGDirectDisplayID;
    fn CGGetActiveDisplayList(max: u32, displays: *mut CGDirectDisplayID, count: *mut u32) -> i32;
    fn CGDisplayBounds(d: CGDirectDisplayID) -> CGRect;
    fn CGEventSourceCreate(state: i32) -> CGEventSourceRef;
    fn CGEventCreateMouseEvent(src: CGEventSourceRef, type_: u32, p: CGPoint, button: u32) -> CGEventRef;
    fn CGEventCreateKeyboardEvent(src: CGEventSourceRef, key: u16, down: bool) -> CGEventRef;
    fn CGEventCreateScrollWheelEvent2(src: CGEventSourceRef, units: u32, count: u32, w1: i32, w2: i32, w3: i32) -> CGEventRef;
    fn CGEventPost(tap: u32, e: CGEventRef);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFMachPortCreateRunLoopSource(alloc: *const c_void, port: CFMachPortRef, order: isize) -> CFRunLoopSourceRef;
    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopAddSource(rl: CFRunLoopRef, src: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopRun();
    fn CFRelease(cf: CFTypeRef);
    fn CFStringCreateWithCString(alloc: *const c_void, s: *const c_char, encoding: u32) -> CFStringRef;
    fn CFDictionaryCreate(alloc: *const c_void, keys: *const CFTypeRef, values: *const CFTypeRef, n: isize, kcb: *const c_void, vcb: *const c_void) -> CFDictionaryRef;
    static kCFRunLoopCommonModes: CFStringRef;
    static kCFBooleanTrue: CFTypeRef;
    static kCFTypeDictionaryKeyCallBacks: u8;
    static kCFTypeDictionaryValueCallBacks: u8;
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> u8;
    static kAXTrustedCheckOptionPrompt: CFStringRef;
}

const SESSION_TAP: u32 = 1;
const HID_TAP: u32 = 0;
const HEAD_INSERT: u32 = 0;
const TAP_DEFAULT: u32 = 0;
const SOURCE_HID_STATE: i32 = 1;
const UTF8: u32 = 0x0800_0100;

const LEFT_DOWN: u32 = 1;
const LEFT_UP: u32 = 2;
const RIGHT_DOWN: u32 = 3;
const RIGHT_UP: u32 = 4;
const MOUSE_MOVED: u32 = 5;
const LEFT_DRAGGED: u32 = 6;
const RIGHT_DRAGGED: u32 = 7;
const KEY_DOWN: u32 = 10;
const KEY_UP: u32 = 11;
const FLAGS_CHANGED: u32 = 12;
const SCROLL: u32 = 22;
const OTHER_DOWN: u32 = 25;
const OTHER_UP: u32 = 26;
const OTHER_DRAGGED: u32 = 27;
const TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
const TAP_DISABLED_BY_USER: u32 = 0xFFFF_FFFF;

const FIELD_CLICK_STATE: u32 = 1;
const FIELD_BUTTON_NUMBER: u32 = 3;
const FIELD_DELTA_X: u32 = 4;
const FIELD_DELTA_Y: u32 = 5;
const FIELD_AUTOREPEAT: u32 = 8;
const FIELD_KEYCODE: u32 = 9;
const FIELD_SCROLL_IS_CONTINUOUS: u32 = 88;
const FIELD_SCROLL_FIXED_1: u32 = 93;
const FIELD_SCROLL_FIXED_2: u32 = 94;
const FIELD_SCROLL_POINT_1: u32 = 96;
const FIELD_SCROLL_POINT_2: u32 = 97;

const FLAG_ALPHA_SHIFT: u64 = 0x0001_0000;
const FLAG_SHIFT: u64 = 0x0002_0000;
const FLAG_CONTROL: u64 = 0x0004_0000;
const FLAG_ALTERNATE: u64 = 0x0008_0000;
const FLAG_COMMAND: u64 = 0x0010_0000;

/// Hi-res wheel units (1/120 notch) per point of trackpad scrolling.
const UNITS_PER_POINT: f64 = 3.0;

fn mask(types: &[u32]) -> u64 {
    types.iter().fold(0, |m, t| m | (1u64 << t))
}

/// Device-dependent flag bit telling whether a given modifier key is down.
fn modifier_bit(kvk: u16) -> Option<u64> {
    Some(match kvk {
        0x3B => 0x0001,
        0x38 => 0x0002,
        0x3C => 0x0004,
        0x37 => 0x0008,
        0x36 => 0x0010,
        0x3A => 0x0020,
        0x3D => 0x0040,
        0x3E => 0x2000,
        _ => return None,
    })
}

struct Capture {
    /// Swapped when the server restarts; the tap itself is installed once per process.
    core: Mutex<Arc<Mutex<ServerCore>>>,
    tap: AtomicPtr<c_void>,
}

static CAPTURE: OnceLock<Capture> = OnceLock::new();

extern "C" fn tap_callback(_proxy: *mut c_void, etype: u32, event: CGEventRef, _user: *mut c_void) -> CGEventRef {
    let Some(cap) = CAPTURE.get() else { return event };
    if etype == TAP_DISABLED_BY_TIMEOUT || etype == TAP_DISABLED_BY_USER {
        // macOS disables a tap whose callback was too slow; turn it straight back on.
        unsafe { CGEventTapEnable(cap.tap.load(Ordering::SeqCst), true) };
        return event;
    }
    let verdict = std::panic::catch_unwind(|| handle_event(cap, etype, event)).unwrap_or(Verdict::Pass);
    if verdict == Verdict::Swallow { null_mut() } else { event }
}

fn handle_event(cap: &Capture, etype: u32, event: CGEventRef) -> Verdict {
    let core = cap.core.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let mut core = core.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: `event` is the live event handed to the tap callback.
    unsafe {
        match etype {
            MOUSE_MOVED | LEFT_DRAGGED | RIGHT_DRAGGED | OTHER_DRAGGED => {
                let p = CGEventGetLocation(event);
                // The double accessors keep the sub-pixel part trackpads produce.
                let dx = CGEventGetDoubleValueField(event, FIELD_DELTA_X);
                let dy = CGEventGetDoubleValueField(event, FIELD_DELTA_Y);
                core.mouse(p.x, p.y, dx, dy)
            }
            LEFT_DOWN | LEFT_UP => core.button(Button::Left, etype == LEFT_DOWN),
            RIGHT_DOWN | RIGHT_UP => core.button(Button::Right, etype == RIGHT_DOWN),
            OTHER_DOWN | OTHER_UP => {
                let button = match CGEventGetIntegerValueField(event, FIELD_BUTTON_NUMBER) {
                    2 => Button::Middle,
                    3 => Button::Back,
                    4 => Button::Forward,
                    _ => return if core.is_remote() { Verdict::Swallow } else { Verdict::Pass },
                };
                core.button(button, etype == OTHER_DOWN)
            }
            SCROLL => {
                if !core.is_remote() {
                    return Verdict::Pass;
                }
                let (dy, dx) = if CGEventGetIntegerValueField(event, FIELD_SCROLL_IS_CONTINUOUS) != 0 {
                    (
                        CGEventGetDoubleValueField(event, FIELD_SCROLL_POINT_1) * UNITS_PER_POINT,
                        CGEventGetDoubleValueField(event, FIELD_SCROLL_POINT_2) * UNITS_PER_POINT,
                    )
                } else {
                    (CGEventGetDoubleValueField(event, FIELD_SCROLL_FIXED_1) * 120.0, CGEventGetDoubleValueField(event, FIELD_SCROLL_FIXED_2) * 120.0)
                };
                // macOS: positive horizontal delta scrolls left; the wire uses positive = right.
                core.wheel(-dx.round() as i32, dy.round() as i32)
            }
            KEY_DOWN | KEY_UP => {
                let kvk = CGEventGetIntegerValueField(event, FIELD_KEYCODE) as u16;
                let repeat = CGEventGetIntegerValueField(event, FIELD_AUTOREPEAT) != 0;
                match keymap::mac_to_evdev(kvk) {
                    Some(code) => core.key(code, etype == KEY_DOWN, repeat),
                    None if core.is_remote() => Verdict::Swallow,
                    None => Verdict::Pass,
                }
            }
            FLAGS_CHANGED => {
                let kvk = CGEventGetIntegerValueField(event, FIELD_KEYCODE) as u16;
                let flags = CGEventGetFlags(event);
                let Some(code) = keymap::mac_to_evdev(kvk) else {
                    // Fn/Globe and other keys without a counterpart.
                    return if core.is_remote() { Verdict::Swallow } else { Verdict::Pass };
                };
                if code == keymap::KEY_CAPSLOCK {
                    // macOS reports Caps Lock as a state toggle; elsewhere it is a key press.
                    let v = core.key(code, true, false);
                    core.key(code, false, false);
                    return v;
                }
                let down = match modifier_bit(kvk) {
                    Some(bit) => flags & bit != 0,
                    None => return Verdict::Pass,
                };
                core.key(code, down, false)
            }
            _ => Verdict::Pass,
        }
    }
}

struct MacCtl {
    hidden: AtomicBool,
}

impl CaptureCtl for MacCtl {
    fn enter_remote(&self) {
        // SAFETY: plain Quartz calls.
        unsafe {
            CGAssociateMouseAndMouseCursorPosition(0);
            if !self.hidden.swap(true, Ordering::SeqCst) {
                CGDisplayHideCursor(CGMainDisplayID());
            }
        }
    }

    fn leave_remote(&self, x: f64, y: f64) {
        // SAFETY: plain Quartz calls. Re-associating right after the warp also skips the
        // short input freeze macOS normally applies after a warp.
        unsafe {
            CGWarpMouseCursorPosition(CGPoint { x, y });
            CGAssociateMouseAndMouseCursorPosition(1);
            if self.hidden.swap(false, Ordering::SeqCst) {
                CGDisplayShowCursor(CGMainDisplayID());
            }
        }
    }
}

fn cfstr(s: &CStr) -> CFStringRef {
    // SAFETY: creates a CFString from a NUL-terminated UTF-8 string.
    unsafe { CFStringCreateWithCString(std::ptr::null(), s.as_ptr(), UTF8) }
}

/// Lets a background process hide the cursor (what other KVM tools do); without it the frozen
/// cursor stays visible while a client is driven.
fn allow_background_cursor_hiding() {
    type GetConn = unsafe extern "C" fn() -> i32;
    type SetProp = unsafe extern "C" fn(i32, i32, CFStringRef, CFTypeRef) -> i32;
    // SAFETY: looks up private SkyLight symbols and calls them only if both exist.
    unsafe {
        let get = libc::dlsym(libc::RTLD_DEFAULT, c"_CGSDefaultConnection".as_ptr());
        let set = libc::dlsym(libc::RTLD_DEFAULT, c"CGSSetConnectionProperty".as_ptr());
        if get.is_null() || set.is_null() {
            log::debug!("cannot enable background cursor hiding");
            return;
        }
        let get: GetConn = std::mem::transmute(get);
        let set: SetProp = std::mem::transmute(set);
        let conn = get();
        let key = cfstr(c"SetsCursorInBackground");
        set(conn, conn, key, kCFBooleanTrue);
        CFRelease(key);
    }
}

fn trusted(prompt: bool) -> bool {
    // SAFETY: AX trust check, optionally with the system prompt.
    unsafe {
        if !prompt {
            return AXIsProcessTrusted() != 0;
        }
        let keys = [kAXTrustedCheckOptionPrompt];
        let values = [kCFBooleanTrue];
        let dict = CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            &raw const kCFTypeDictionaryKeyCallBacks as *const c_void,
            &raw const kCFTypeDictionaryValueCallBacks as *const c_void,
        );
        let ok = AXIsProcessTrustedWithOptions(dict) != 0;
        CFRelease(dict);
        ok
    }
}

const ACCESSIBILITY_HINT: &str = "macOS has not granted Accessibility access. Open System Settings > Privacy & Security > \
     Accessibility, enable the app you run wifishare from (Terminal, iTerm, ...), then restart wifishare.";

pub fn start_capture(core: Arc<Mutex<ServerCore>>) -> Result<Arc<dyn CaptureCtl>> {
    if !trusted(true) {
        bail!("{ACCESSIBILITY_HINT}");
    }
    if let Some(cap) = CAPTURE.get() {
        *cap.core.lock().unwrap() = core;
        return Ok(Arc::new(MacCtl { hidden: AtomicBool::new(false) }));
    }
    allow_background_cursor_hiding();
    if CAPTURE.set(Capture { core: Mutex::new(core), tap: AtomicPtr::new(null_mut()) }).is_err() {
        bail!("input capture already started");
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new().name("event-tap".into()).spawn(move || {
        let types = [
            LEFT_DOWN, LEFT_UP, RIGHT_DOWN, RIGHT_UP, MOUSE_MOVED, LEFT_DRAGGED, RIGHT_DRAGGED, KEY_DOWN, KEY_UP, FLAGS_CHANGED, SCROLL, OTHER_DOWN, OTHER_UP,
            OTHER_DRAGGED,
        ];
        // SAFETY: standard event tap setup on a thread that then runs its own run loop.
        unsafe {
            let tap = CGEventTapCreate(SESSION_TAP, HEAD_INSERT, TAP_DEFAULT, mask(&types), tap_callback, null_mut());
            if tap.is_null() {
                let _ = tx.send(false);
                return;
            }
            if let Some(cap) = CAPTURE.get() {
                cap.tap.store(tap, Ordering::SeqCst);
            }
            let source = CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0);
            CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
            CGEventTapEnable(tap, true);
            let _ = tx.send(true);
            CFRunLoopRun();
        }
    })?;
    if !rx.recv_timeout(Duration::from_secs(5)).unwrap_or(false) {
        bail!("could not create the event tap. {ACCESSIBILITY_HINT}");
    }
    log::info!("capturing keyboard and mouse");
    Ok(Arc::new(MacCtl { hidden: AtomicBool::new(false) }))
}

pub fn screen() -> Result<Screen> {
    let mut ids = [0u32; 16];
    let mut n = 0u32;
    // SAFETY: the buffer holds 16 display ids.
    let err = unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut n) };
    if err != 0 || n == 0 {
        bail!("CGGetActiveDisplayList failed ({err})");
    }
    let monitors = ids[..n as usize]
        .iter()
        .map(|&id| {
            // SAFETY: id comes from the active display list.
            let b = unsafe { CGDisplayBounds(id) };
            Rect::new(b.origin.x.round() as i32, b.origin.y.round() as i32, b.size.width.round() as i32, b.size.height.round() as i32)
        })
        .collect();
    Ok(Screen { monitors })
}

pub struct MacInjector {
    src: CGEventSourceRef,
    pos: CGPoint,
    buttons: u8,
    flags: u64,
    held_mods: Vec<u16>,
    last_click: Option<(Instant, Button, CGPoint, i64)>,
    scroll_speed: f64,
}

// SAFETY: the event source is only used from whichever thread owns the injector.
unsafe impl Send for MacInjector {}

pub fn injector(scroll_speed: f64) -> Result<Box<dyn Injector>> {
    if !trusted(true) {
        bail!("{ACCESSIBILITY_HINT}");
    }
    // SAFETY: creates an event source tied to the HID system state.
    let src = unsafe { CGEventSourceCreate(SOURCE_HID_STATE) };
    Ok(Box::new(MacInjector { src, pos: CGPoint { x: 0.0, y: 0.0 }, buttons: 0, flags: 0, held_mods: Vec::new(), last_click: None, scroll_speed }))
}

fn mouse_button_number(b: Button) -> u32 {
    match b {
        Button::Left => 0,
        Button::Right => 1,
        Button::Middle => 2,
        Button::Back => 3,
        Button::Forward => 4,
    }
}

impl MacInjector {
    fn post(&self, ev: CGEventRef) {
        if ev.is_null() {
            return;
        }
        // SAFETY: `ev` was just created and is released after posting.
        unsafe {
            CGEventSetFlags(ev, self.flags);
            CGEventPost(HID_TAP, ev);
            CFRelease(ev);
        }
    }

    fn recompute_flags(&mut self) {
        let mut f = self.flags & FLAG_ALPHA_SHIFT;
        for &code in &self.held_mods {
            f |= match code {
                keymap::KEY_LEFTSHIFT | keymap::KEY_RIGHTSHIFT => FLAG_SHIFT,
                keymap::KEY_LEFTCTRL | keymap::KEY_RIGHTCTRL => FLAG_CONTROL,
                keymap::KEY_LEFTALT | keymap::KEY_RIGHTALT => FLAG_ALTERNATE,
                keymap::KEY_LEFTMETA | keymap::KEY_RIGHTMETA => FLAG_COMMAND,
                _ => 0,
            };
        }
        self.flags = f;
    }
}

impl Injector for MacInjector {
    fn set_screen(&mut self, _screen: &Screen) {}

    fn motion(&mut self, x: f64, y: f64) {
        self.pos = CGPoint { x, y };
        let (etype, button) = if self.buttons & Button::Left.bit() != 0 {
            (LEFT_DRAGGED, 0)
        } else if self.buttons & Button::Right.bit() != 0 {
            (RIGHT_DRAGGED, 1)
        } else if self.buttons != 0 {
            (OTHER_DRAGGED, 2)
        } else {
            (MOUSE_MOVED, 0)
        };
        // SAFETY: creates a mouse event that `post` releases.
        self.post(unsafe { CGEventCreateMouseEvent(self.src, etype, self.pos, button) });
    }

    fn button(&mut self, button: Button, down: bool) {
        if down {
            self.buttons |= button.bit();
        } else {
            self.buttons &= !button.bit();
        }
        let etype = match (button, down) {
            (Button::Left, true) => LEFT_DOWN,
            (Button::Left, false) => LEFT_UP,
            (Button::Right, true) => RIGHT_DOWN,
            (Button::Right, false) => RIGHT_UP,
            (_, true) => OTHER_DOWN,
            (_, false) => OTHER_UP,
        };
        // Double/triple clicks: macOS wants the click count on each event.
        let count = match self.last_click {
            Some((t, b, p, n)) if b == button && t.elapsed() < Duration::from_millis(500) && (p.x - self.pos.x).abs() < 5.0 && (p.y - self.pos.y).abs() < 5.0 => {
                if down { n + 1 } else { n }
            }
            _ => 1,
        };
        if down {
            self.last_click = Some((Instant::now(), button, self.pos, count));
        }
        // SAFETY: creates a mouse event that `post` releases.
        unsafe {
            let ev = CGEventCreateMouseEvent(self.src, etype, self.pos, mouse_button_number(button));
            if !ev.is_null() {
                CGEventSetIntegerValueField(ev, FIELD_CLICK_STATE, count);
                CGEventSetIntegerValueField(ev, FIELD_BUTTON_NUMBER, mouse_button_number(button) as i64);
            }
            self.post(ev);
        }
    }

    fn wheel(&mut self, dx: i32, dy: i32) {
        let px = |units: i32| (units as f64 * self.scroll_speed / UNITS_PER_POINT).round() as i32;
        // SAFETY: creates a scroll event (pixel units) that `post` releases.
        self.post(unsafe { CGEventCreateScrollWheelEvent2(self.src, 0, 2, px(dy), -px(dx), 0) });
    }

    fn key(&mut self, code: u16, down: bool) {
        let Some(kvk) = keymap::evdev_to_mac(code) else { return };
        if keymap::is_modifier(code) {
            self.held_mods.retain(|&c| c != code);
            if down {
                self.held_mods.push(code);
            }
            self.recompute_flags();
        } else if code == keymap::KEY_CAPSLOCK && down {
            self.flags ^= FLAG_ALPHA_SHIFT;
        }
        // SAFETY: creates a keyboard event that `post` releases.
        self.post(unsafe { CGEventCreateKeyboardEvent(self.src, kvk, down) });
    }

    fn os_repeats_keys(&self) -> bool {
        false
    }
}

pub struct PasteboardWatch;

pub fn clipboard_watch() -> Box<dyn ClipboardWatch> {
    Box::new(PasteboardWatch)
}

impl ClipboardWatch for PasteboardWatch {
    fn change_count(&mut self) -> Option<u64> {
        Some(objc2_app_kit::NSPasteboard::generalPasteboard().changeCount() as u64)
    }
}

pub fn permission_problems(_server: bool) -> Vec<String> {
    if trusted(false) { Vec::new() } else { vec![ACCESSIBILITY_HINT.into()] }
}
