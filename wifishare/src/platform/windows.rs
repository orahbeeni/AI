//! Windows: input capture with low-level hooks, injection with SendInput (scan codes, so the
//! receiving layout applies), clipboard changes via the clipboard sequence number.

use std::collections::HashSet;
use std::ptr::{null, null_mut};
use std::sync::{Arc, Mutex, Once, OnceLock};
use std::time::Duration;

use anyhow::{Result, bail};
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO};
use windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MOUSEEVENTF_ABSOLUTE,
    MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT, SendInput,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, GetSystemMetrics, KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SetCursorPos, SetWindowsHookExW, TranslateMessage, WH_KEYBOARD_LL, WH_MOUSE_LL,
};

use super::{ClipboardWatch, Injector};
use crate::geom::{Rect, Screen};
use crate::keymap;
use crate::proto::Button;
use crate::server::core::{CaptureCtl, ServerCore, Verdict};

const WM_KEYDOWN: usize = 0x0100;
const WM_SYSKEYDOWN: usize = 0x0104;
const WM_MOUSEMOVE: usize = 0x0200;
const WM_LBUTTONDOWN: usize = 0x0201;
const WM_LBUTTONUP: usize = 0x0202;
const WM_RBUTTONDOWN: usize = 0x0204;
const WM_RBUTTONUP: usize = 0x0205;
const WM_MBUTTONDOWN: usize = 0x0207;
const WM_MBUTTONUP: usize = 0x0208;
const WM_MOUSEWHEEL: usize = 0x020A;
const WM_XBUTTONDOWN: usize = 0x020B;
const WM_XBUTTONUP: usize = 0x020C;
const WM_MOUSEHWHEEL: usize = 0x020E;
const LLMHF_INJECTED: u32 = 0x1;
const LLKHF_EXTENDED: u32 = 0x1;
const LLKHF_INJECTED: u32 = 0x10;
const XBUTTON1: u32 = 1;
const XBUTTON2: u32 = 2;

/// Per-monitor DPI awareness, so all coordinates are physical pixels.
fn dpi_aware() {
    static ONCE: Once = Once::new();
    // SAFETY: process-wide setting, applied once.
    ONCE.call_once(|| unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    });
}

struct HookState {
    /// Swapped when the server restarts; the hooks themselves are installed once per process.
    core: Mutex<Arc<Mutex<ServerCore>>>,
    /// Last cursor position seen while local, and where the cursor is parked while remote.
    last: Mutex<(i32, i32)>,
    park: Mutex<(i32, i32)>,
    pressed: Mutex<HashSet<u16>>,
}

static HOOKS: OnceLock<HookState> = OnceLock::new();

fn hiword(v: u32) -> i16 {
    (v >> 16) as u16 as i16
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == 0
        && let Some(st) = HOOKS.get()
    {
        // SAFETY: for WH_MOUSE_LL, lparam points to an MSLLHOOKSTRUCT.
        let info = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
        if info.flags & LLMHF_INJECTED == 0 && std::panic::catch_unwind(|| handle_mouse(st, wparam, info)).unwrap_or(Verdict::Pass) == Verdict::Swallow {
            return 1;
        }
    }
    // SAFETY: passes the event on to the next hook.
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}

fn handle_mouse(st: &HookState, msg: WPARAM, info: &MSLLHOOKSTRUCT) -> Verdict {
    let core = st.core.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let mut core = core.lock().unwrap_or_else(|e| e.into_inner());
    let (x, y) = (info.pt.x, info.pt.y);
    match msg {
        WM_MOUSEMOVE => {
            if core.is_remote() {
                // Moves are swallowed, so the cursor stays parked and each event's position is
                // the parked spot plus this movement.
                let (px, py) = *st.park.lock().unwrap();
                core.mouse(px as f64, py as f64, (x - px) as f64, (y - py) as f64)
            } else {
                let mut last = st.last.lock().unwrap();
                let (dx, dy) = (x - last.0, y - last.1);
                *last = (x, y);
                core.mouse(x as f64, y as f64, dx as f64, dy as f64)
            }
        }
        WM_LBUTTONDOWN | WM_LBUTTONUP => core.button(Button::Left, msg == WM_LBUTTONDOWN),
        WM_RBUTTONDOWN | WM_RBUTTONUP => core.button(Button::Right, msg == WM_RBUTTONDOWN),
        WM_MBUTTONDOWN | WM_MBUTTONUP => core.button(Button::Middle, msg == WM_MBUTTONDOWN),
        WM_XBUTTONDOWN | WM_XBUTTONUP => {
            let b = if hiword(info.mouseData) as u32 == XBUTTON1 { Button::Back } else { Button::Forward };
            core.button(b, msg == WM_XBUTTONDOWN)
        }
        WM_MOUSEWHEEL => core.wheel(0, hiword(info.mouseData) as i32),
        WM_MOUSEHWHEEL => core.wheel(hiword(info.mouseData) as i32, 0),
        _ => Verdict::Pass,
    }
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == 0
        && let Some(st) = HOOKS.get()
    {
        // SAFETY: for WH_KEYBOARD_LL, lparam points to a KBDLLHOOKSTRUCT.
        let info = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        if info.flags & LLKHF_INJECTED == 0 && std::panic::catch_unwind(|| handle_key(st, wparam, info)).unwrap_or(Verdict::Pass) == Verdict::Swallow {
            return 1;
        }
    }
    // SAFETY: passes the event on to the next hook.
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}

fn handle_key(st: &HookState, msg: WPARAM, info: &KBDLLHOOKSTRUCT) -> Verdict {
    let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
    let core = st.core.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let mut core = core.lock().unwrap_or_else(|e| e.into_inner());
    let Some(code) = keymap::win_to_evdev(info.scanCode as u16, info.flags & LLKHF_EXTENDED != 0) else {
        return if core.is_remote() { Verdict::Swallow } else { Verdict::Pass };
    };
    let mut pressed = st.pressed.lock().unwrap();
    let repeat = down && !pressed.insert(code);
    if !down {
        pressed.remove(&code);
    }
    drop(pressed);
    core.key(code, down, repeat)
}

struct WinCtl;

impl CaptureCtl for WinCtl {
    fn enter_remote(&self) {
        let Some(st) = HOOKS.get() else { return };
        // Park in the middle of the primary monitor so movement in every direction registers.
        // SAFETY: plain user32 calls.
        let (cx, cy) = unsafe { (GetSystemMetrics(0) / 2, GetSystemMetrics(1) / 2) };
        *st.park.lock().unwrap() = (cx, cy);
        // SAFETY: plain user32 call.
        unsafe { SetCursorPos(cx, cy) };
    }

    fn leave_remote(&self, x: f64, y: f64) {
        let (x, y) = (x.round() as i32, y.round() as i32);
        if let Some(st) = HOOKS.get() {
            *st.last.lock().unwrap() = (x, y);
        }
        // SAFETY: plain user32 call.
        unsafe { SetCursorPos(x, y) };
    }
}

pub fn start_capture(core: Arc<Mutex<ServerCore>>) -> Result<Arc<dyn CaptureCtl>> {
    dpi_aware();
    if let Some(st) = HOOKS.get() {
        *st.core.lock().unwrap() = core;
        return Ok(Arc::new(WinCtl));
    }
    let state = HookState { core: Mutex::new(core), last: Mutex::new((0, 0)), park: Mutex::new((0, 0)), pressed: Mutex::new(HashSet::new()) };
    if HOOKS.set(state).is_err() {
        bail!("input capture already started");
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new().name("input-hooks".into()).spawn(move || {
        // SAFETY: installs global low-level hooks and pumps messages on this thread, as
        // low-level hooks require.
        unsafe {
            let module = GetModuleHandleW(null());
            let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0);
            let keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), module, 0);
            let _ = tx.send(!mouse.is_null() && !keyboard.is_null());
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    })?;
    if !rx.recv_timeout(Duration::from_secs(5)).unwrap_or(false) {
        bail!("could not install keyboard/mouse hooks");
    }
    log::info!("capturing keyboard and mouse");
    Ok(Arc::new(WinCtl))
}

unsafe extern "system" fn monitor_proc(_m: HMONITOR, _dc: HDC, _r: *mut RECT, data: LPARAM) -> windows_sys::core::BOOL {
    // SAFETY: `data` is the Vec passed by `screen`, and `_m` a live monitor handle.
    unsafe {
        let out = &mut *(data as *mut Vec<Rect>);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(_m, &mut info) != 0 {
            let r = info.rcMonitor;
            out.push(Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top));
        }
    }
    1
}

pub fn screen() -> Result<Screen> {
    dpi_aware();
    let mut monitors: Vec<Rect> = Vec::new();
    // SAFETY: the callback only writes into `monitors` while this call runs.
    unsafe { EnumDisplayMonitors(null_mut(), null(), Some(monitor_proc), &mut monitors as *mut _ as LPARAM) };
    if monitors.is_empty() {
        bail!("no monitors found");
    }
    Ok(Screen { monitors })
}

pub struct WinInjector {
    scroll_speed: f64,
}

pub fn injector(scroll_speed: f64) -> Result<Box<dyn Injector>> {
    dpi_aware();
    Ok(Box::new(WinInjector { scroll_speed }))
}

fn send_mouse(dx: i32, dy: i32, data: u32, flags: u32) {
    let input = INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi: MOUSEINPUT { dx, dy, mouseData: data, dwFlags: flags, time: 0, dwExtraInfo: 0 } } };
    // SAFETY: one fully initialised INPUT.
    unsafe { SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) };
}

impl Injector for WinInjector {
    fn set_screen(&mut self, _screen: &Screen) {}

    fn motion(&mut self, x: f64, y: f64) {
        // SAFETY: plain user32 calls.
        let (vx, vy, vw, vh) =
            unsafe { (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN), GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN)) };
        let nx = ((x - vx as f64) * 65535.0 / (vw - 1).max(1) as f64).round() as i32;
        let ny = ((y - vy as f64) * 65535.0 / (vh - 1).max(1) as f64).round() as i32;
        send_mouse(nx, ny, 0, MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK);
    }

    fn button(&mut self, button: Button, down: bool) {
        let (flags, data) = match (button, down) {
            (Button::Left, true) => (MOUSEEVENTF_LEFTDOWN, 0),
            (Button::Left, false) => (MOUSEEVENTF_LEFTUP, 0),
            (Button::Right, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
            (Button::Right, false) => (MOUSEEVENTF_RIGHTUP, 0),
            (Button::Middle, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
            (Button::Middle, false) => (MOUSEEVENTF_MIDDLEUP, 0),
            (Button::Back, d) => (if d { MOUSEEVENTF_XDOWN } else { MOUSEEVENTF_XUP }, XBUTTON1),
            (Button::Forward, d) => (if d { MOUSEEVENTF_XDOWN } else { MOUSEEVENTF_XUP }, XBUTTON2),
        };
        send_mouse(0, 0, data, flags);
    }

    fn wheel(&mut self, dx: i32, dy: i32) {
        let scale = |v: i32| (v as f64 * self.scroll_speed).round() as i32;
        if dy != 0 {
            send_mouse(0, 0, scale(dy) as u32, MOUSEEVENTF_WHEEL);
        }
        if dx != 0 {
            send_mouse(0, 0, scale(dx) as u32, MOUSEEVENTF_HWHEEL);
        }
    }

    fn key(&mut self, code: u16, down: bool) {
        let Some((scan, extended)) = keymap::evdev_to_win(code) else { return };
        let mut flags = KEYEVENTF_SCANCODE;
        if extended {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        if !down {
            flags |= KEYEVENTF_KEYUP;
        }
        let input = INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: 0, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } } };
        // SAFETY: one fully initialised INPUT.
        unsafe { SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) };
    }

    fn os_repeats_keys(&self) -> bool {
        false
    }
}

pub struct SequenceWatch;

pub fn clipboard_watch() -> Box<dyn ClipboardWatch> {
    Box::new(SequenceWatch)
}

impl ClipboardWatch for SequenceWatch {
    fn change_count(&mut self) -> Option<u64> {
        // SAFETY: plain user32 call.
        Some(unsafe { GetClipboardSequenceNumber() } as u64)
    }
}

pub fn permission_problems(_server: bool) -> Vec<String> {
    Vec::new()
}

