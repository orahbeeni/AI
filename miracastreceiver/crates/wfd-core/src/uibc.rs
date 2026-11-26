//! User Input Back Channel (UIBC): sink -> source mouse/keyboard over TCP.
//!
//! Negotiation happens in RTSP (`wfd_uibc_capability`, `wfd_uibc_setting`).
//! The source picks the categories and puts its TCP port in M4; the sink
//! connects to `source_ip:port` and writes packets:
//!
//! ```text
//! UIBC header: [0] version(3) T(1) reserved(4) | [1] reserved(4) category(4)
//!              [2..4] total packet length (BE, includes header and padding)
//!              [4..6] timestamp, only when T=1 (we never set it)
//! Generic body: [id:1][len:2 BE][describe]
//! HIDC body:    [input path:1][HID type:1][usage:1][len:2 BE][report or descriptor]
//! ```
//! Packets are padded to an even length.

use std::fmt;

pub const CATEGORY_GENERIC: u8 = 0;
pub const CATEGORY_HIDC: u8 = 1;

/// Generic input type IDs.
pub mod generic_id {
    pub const LEFT_DOWN: u8 = 0;
    pub const LEFT_UP: u8 = 1;
    pub const MOVE: u8 = 2;
    pub const KEY_DOWN: u8 = 3;
    pub const KEY_UP: u8 = 4;
    pub const ZOOM: u8 = 5;
    pub const VSCROLL: u8 = 6;
    pub const HSCROLL: u8 = 7;
    pub const ROTATE: u8 = 8;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputType {
    Keyboard = 0,
    Mouse = 1,
    SingleTouch = 2,
    MultiTouch = 3,
    Joystick = 4,
    Camera = 5,
    Gesture = 6,
    RemoteControl = 7,
}

impl InputType {
    const ALL: [InputType; 8] = [
        Self::Keyboard,
        Self::Mouse,
        Self::SingleTouch,
        Self::MultiTouch,
        Self::Joystick,
        Self::Camera,
        Self::Gesture,
        Self::RemoteControl,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Keyboard => "Keyboard",
            Self::Mouse => "Mouse",
            Self::SingleTouch => "SingleTouch",
            Self::MultiTouch => "MultiTouch",
            Self::Joystick => "Joystick",
            Self::Camera => "Camera",
            Self::Gesture => "Gesture",
            Self::RemoteControl => "RemoteControl",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.name().eq_ignore_ascii_case(s.trim()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HidPath {
    Infrared = 0,
    Usb = 1,
    Bluetooth = 2,
    Zigbee = 3,
    WiFi = 4,
    NoSp = 5,
}

impl HidPath {
    const ALL: [HidPath; 6] = [Self::Infrared, Self::Usb, Self::Bluetooth, Self::Zigbee, Self::WiFi, Self::NoSp];

    pub fn name(self) -> &'static str {
        match self {
            Self::Infrared => "Infrared",
            Self::Usb => "USB",
            Self::Bluetooth => "BT",
            Self::Zigbee => "Zigbee",
            Self::WiFi => "Wi-Fi",
            Self::NoSp => "No-SP",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.name().eq_ignore_ascii_case(s.trim()))
    }
}

/// `wfd_uibc_capability` value.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UibcCapability {
    pub generic: Vec<InputType>,
    pub hidc: Vec<(InputType, HidPath)>,
    pub port: Option<u16>,
}

impl UibcCapability {
    /// What this sink offers: generic + HIDC keyboard and mouse.
    pub fn sink_default() -> Self {
        Self {
            generic: vec![InputType::Keyboard, InputType::Mouse, InputType::SingleTouch],
            hidc: vec![(InputType::Keyboard, HidPath::Usb), (InputType::Mouse, HidPath::Usb)],
            port: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.generic.is_empty() && self.hidc.is_empty()
    }

    pub fn has_generic(&self, t: InputType) -> bool {
        self.generic.contains(&t)
    }

    pub fn hidc_path(&self, t: InputType) -> Option<HidPath> {
        self.hidc.iter().find(|(ty, _)| *ty == t).map(|(_, p)| *p)
    }

    /// Parses the value; `"none"` yields `None`.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("none") || s.is_empty() {
            return None;
        }
        let mut cap = Self::default();
        let mut categories: Vec<String> = Vec::new();
        for part in s.split(';') {
            let Some((k, v)) = part.split_once('=') else { continue };
            let v = v.trim();
            let list = || v.split(',').map(str::trim).filter(|x| !x.is_empty() && !x.eq_ignore_ascii_case("none"));
            match k.trim().to_ascii_lowercase().as_str() {
                "input_category_list" => categories = list().map(|x| x.to_ascii_uppercase()).collect(),
                "generic_cap_list" => cap.generic = list().filter_map(InputType::parse).collect(),
                "hidc_cap_list" => {
                    cap.hidc = list()
                        .filter_map(|x| {
                            let (t, p) = x.split_once('/')?;
                            Some((InputType::parse(t)?, HidPath::parse(p)?))
                        })
                        .collect()
                }
                "port" => cap.port = v.parse().ok(),
                _ => {}
            }
        }
        if !categories.iter().any(|c| c == "GENERIC") {
            cap.generic.clear();
        }
        if !categories.iter().any(|c| c == "HIDC") {
            cap.hidc.clear();
        }
        Some(cap)
    }

    /// Our capability restricted to what the source offered (for display).
    pub fn intersect(&self, other: &Self) -> Self {
        Self {
            generic: self.generic.iter().copied().filter(|t| other.generic.contains(t)).collect(),
            hidc: self.hidc.iter().copied().filter(|(t, _)| other.hidc_path(*t).is_some()).collect(),
            port: other.port.or(self.port),
        }
    }
}

impl fmt::Display for UibcCapability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut cats = Vec::new();
        if !self.generic.is_empty() {
            cats.push("GENERIC");
        }
        if !self.hidc.is_empty() {
            cats.push("HIDC");
        }
        if cats.is_empty() {
            return f.write_str("none");
        }
        let generic =
            if self.generic.is_empty() { "none".to_string() } else { self.generic.iter().map(|t| t.name()).collect::<Vec<_>>().join(", ") };
        let hidc = if self.hidc.is_empty() {
            "none".to_string()
        } else {
            self.hidc.iter().map(|(t, p)| format!("{}/{}", t.name(), p.name())).collect::<Vec<_>>().join(", ")
        };
        let port = self.port.map_or("none".to_string(), |p| p.to_string());
        write!(f, "input_category_list={};generic_cap_list={generic};hidc_cap_list={hidc};port={port}", cats.join(", "))
    }
}

// ---------------------------------------------------------------- packets

fn uibc_packet(category: u8, body: &[u8]) -> Vec<u8> {
    let mut total = 4 + body.len();
    if total % 2 == 1 {
        total += 1;
    }
    let mut p = Vec::with_capacity(total);
    p.push(0x00); // version 0, no timestamp
    p.push(category & 0x0F);
    p.extend_from_slice(&(total as u16).to_be_bytes());
    p.extend_from_slice(body);
    p.resize(total, 0);
    p
}

pub fn generic_packet(id: u8, describe: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(3 + describe.len());
    body.push(id);
    body.extend_from_slice(&(describe.len() as u16).to_be_bytes());
    body.extend_from_slice(describe);
    uibc_packet(CATEGORY_GENERIC, &body)
}

pub fn generic_pointer(id: u8, x: u16, y: u16) -> Vec<u8> {
    let [x0, x1] = x.to_be_bytes();
    let [y0, y1] = y.to_be_bytes();
    generic_packet(id, &[1, 0, x0, x1, y0, y1])
}

pub fn generic_key(id: u8, code: u16) -> Vec<u8> {
    let [c0, c1] = code.to_be_bytes();
    generic_packet(id, &[0, c0, c1, 0, 0])
}

/// Generic scroll in mouse notches. `amount > 0` scrolls up (vertical) or
/// left (horizontal).
pub fn generic_scroll(vertical: bool, amount: i32) -> Option<Vec<u8>> {
    if amount == 0 {
        return None;
    }
    let mut v: u16 = 0b01 << 14; // unit: mouse notch
    if amount > 0 {
        v |= 1 << 13;
    }
    v |= (amount.unsigned_abs().min(0x1FFF)) as u16;
    Some(generic_packet(if vertical { generic_id::VSCROLL } else { generic_id::HSCROLL }, &v.to_be_bytes()))
}

pub const HID_USAGE_REPORT: u8 = 0;
pub const HID_USAGE_DESCRIPTOR: u8 = 1;

pub fn hidc_packet(path: HidPath, hid_type: InputType, usage: u8, data: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(5 + data.len());
    body.push(path as u8);
    body.push(hid_type as u8);
    body.push(usage);
    body.extend_from_slice(&(data.len() as u16).to_be_bytes());
    body.extend_from_slice(data);
    uibc_packet(CATEGORY_HIDC, &body)
}

/// Boot-protocol keyboard: 8-byte reports `[mods, 0, k1..k6]`.
pub const KEYBOARD_DESCRIPTOR: &[u8] = &[
    0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x05, 0x07, 0x19, 0xe0, 0x29, 0xe7, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x08, 0x81, 0x02,
    0x95, 0x01, 0x75, 0x08, 0x81, 0x03, 0x95, 0x05, 0x75, 0x01, 0x05, 0x08, 0x19, 0x01, 0x29, 0x05, 0x91, 0x02, 0x95, 0x01, 0x75, 0x03,
    0x91, 0x03, 0x95, 0x06, 0x75, 0x08, 0x15, 0x00, 0x25, 0x65, 0x05, 0x07, 0x19, 0x00, 0x29, 0x65, 0x81, 0x00, 0xc0,
];

/// Relative mouse: 4-byte reports `[buttons(3 bits), dx, dy, wheel]`.
pub const MOUSE_DESCRIPTOR: &[u8] = &[
    0x05, 0x01, 0x09, 0x02, 0xa1, 0x01, 0x09, 0x01, 0xa1, 0x00, 0x05, 0x09, 0x19, 0x01, 0x29, 0x03, 0x15, 0x00, 0x25, 0x01, 0x95, 0x03,
    0x75, 0x01, 0x81, 0x02, 0x95, 0x01, 0x75, 0x05, 0x81, 0x03, 0x05, 0x01, 0x09, 0x30, 0x09, 0x31, 0x09, 0x38, 0x15, 0x81, 0x25, 0x7f,
    0x75, 0x08, 0x95, 0x03, 0x81, 0x06, 0xc0, 0xc0,
];

/// HID keyboard modifier bits.
pub mod modifier {
    pub const LCTRL: u8 = 0x01;
    pub const LSHIFT: u8 = 0x02;
    pub const LALT: u8 = 0x04;
    pub const LGUI: u8 = 0x08;
    pub const RCTRL: u8 = 0x10;
    pub const RSHIFT: u8 = 0x20;
    pub const RALT: u8 = 0x40;
    pub const RGUI: u8 = 0x80;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

impl MouseButton {
    fn bit(self) -> u8 {
        match self {
            MouseButton::Left => 0x01,
            MouseButton::Right => 0x02,
            MouseButton::Middle => 0x04,
        }
    }
}

/// Input in video coordinates (the negotiated stream resolution).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    /// Absolute position plus the relative motion since the last event.
    PointerMove {
        x: u16,
        y: u16,
        dx: i32,
        dy: i32,
    },
    Button {
        button: MouseButton,
        pressed: bool,
        x: u16,
        y: u16,
    },
    /// Wheel notches; positive = up / left.
    Wheel {
        dx: i32,
        dy: i32,
    },
    /// HID usage ID (keyboard page) with the current modifier bitmap.
    Key {
        usage: u8,
        pressed: bool,
        modifiers: u8,
    },
    /// Modifier state changed without a key event.
    Modifiers(u8),
    /// Release everything (focus lost).
    ReleaseAll,
}

/// Turns [`InputEvent`]s into UIBC packets for the categories the source
/// enabled. Prefers HIDC (full keyboard, all mouse buttons, wheel) and falls
/// back to Generic.
#[derive(Debug, Clone)]
pub struct UibcEncoder {
    hid_keyboard: Option<HidPath>,
    hid_mouse: Option<HidPath>,
    generic_keyboard: bool,
    generic_pointer: bool,
    buttons: u8,
    keys: Vec<u8>,
    modifiers: u8,
}

impl UibcEncoder {
    pub fn new(negotiated: &UibcCapability) -> Self {
        Self {
            hid_keyboard: negotiated.hidc_path(InputType::Keyboard),
            hid_mouse: negotiated.hidc_path(InputType::Mouse),
            generic_keyboard: negotiated.has_generic(InputType::Keyboard),
            generic_pointer: negotiated.has_generic(InputType::Mouse) || negotiated.has_generic(InputType::SingleTouch),
            buttons: 0,
            keys: Vec::new(),
            modifiers: 0,
        }
    }

    pub fn describe(&self) -> String {
        let kb = if self.hid_keyboard.is_some() {
            "HID keyboard"
        } else if self.generic_keyboard {
            "generic keyboard"
        } else {
            "no keyboard"
        };
        let ms = if self.hid_mouse.is_some() {
            "HID mouse"
        } else if self.generic_pointer {
            "generic pointer"
        } else {
            "no mouse"
        };
        format!("{kb}, {ms}")
    }

    pub fn uses_relative_mouse(&self) -> bool {
        self.hid_mouse.is_some()
    }

    /// HID report descriptors, sent once after connecting.
    pub fn initial_packets(&self) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        if let Some(p) = self.hid_mouse {
            out.push(hidc_packet(p, InputType::Mouse, HID_USAGE_DESCRIPTOR, MOUSE_DESCRIPTOR));
        }
        if let Some(p) = self.hid_keyboard {
            out.push(hidc_packet(p, InputType::Keyboard, HID_USAGE_DESCRIPTOR, KEYBOARD_DESCRIPTOR));
        }
        out
    }

    fn mouse_reports(&self, path: HidPath, mut dx: i32, mut dy: i32, wheel: i32) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let mut wheel = wheel.clamp(-127, 127);
        loop {
            let sx = dx.clamp(-127, 127);
            let sy = dy.clamp(-127, 127);
            let report = [self.buttons, sx as i8 as u8, sy as i8 as u8, wheel as i8 as u8];
            out.push(hidc_packet(path, InputType::Mouse, HID_USAGE_REPORT, &report));
            dx -= sx;
            dy -= sy;
            wheel = 0;
            if dx == 0 && dy == 0 {
                break;
            }
        }
        out
    }

    fn keyboard_report(&self, path: HidPath) -> Vec<u8> {
        let mut r = [0u8; 8];
        r[0] = self.modifiers;
        for (i, k) in self.keys.iter().take(6).enumerate() {
            r[2 + i] = *k;
        }
        hidc_packet(path, InputType::Keyboard, HID_USAGE_REPORT, &r)
    }

    pub fn encode(&mut self, ev: &InputEvent) -> Vec<Vec<u8>> {
        match *ev {
            InputEvent::PointerMove { x, y, dx, dy } => {
                if let Some(p) = self.hid_mouse {
                    if dx == 0 && dy == 0 {
                        return Vec::new();
                    }
                    self.mouse_reports(p, dx, dy, 0)
                } else if self.generic_pointer {
                    vec![generic_pointer(generic_id::MOVE, x, y)]
                } else {
                    Vec::new()
                }
            }
            InputEvent::Button { button, pressed, x, y } => {
                if pressed {
                    self.buttons |= button.bit();
                } else {
                    self.buttons &= !button.bit();
                }
                if let Some(p) = self.hid_mouse {
                    self.mouse_reports(p, 0, 0, 0)
                } else if self.generic_pointer && button == MouseButton::Left {
                    vec![generic_pointer(if pressed { generic_id::LEFT_DOWN } else { generic_id::LEFT_UP }, x, y)]
                } else {
                    Vec::new()
                }
            }
            InputEvent::Wheel { dx, dy } => {
                if let Some(p) = self.hid_mouse {
                    if dy == 0 {
                        return Vec::new();
                    }
                    self.mouse_reports(p, 0, 0, dy)
                } else if self.generic_pointer {
                    generic_scroll(true, dy).into_iter().chain(generic_scroll(false, dx)).collect()
                } else {
                    Vec::new()
                }
            }
            InputEvent::Key { usage, pressed, modifiers } => {
                self.modifiers = modifiers;
                if pressed {
                    if !self.keys.contains(&usage) {
                        self.keys.push(usage);
                    }
                } else {
                    self.keys.retain(|k| *k != usage);
                }
                if let Some(p) = self.hid_keyboard {
                    vec![self.keyboard_report(p)]
                } else if self.generic_keyboard {
                    let shift = modifiers & (modifier::LSHIFT | modifier::RSHIFT) != 0;
                    match usage_to_ascii(usage, shift) {
                        Some(c) => vec![generic_key(if pressed { generic_id::KEY_DOWN } else { generic_id::KEY_UP }, c)],
                        None => Vec::new(),
                    }
                } else {
                    Vec::new()
                }
            }
            InputEvent::Modifiers(m) => {
                if m == self.modifiers {
                    return Vec::new();
                }
                self.modifiers = m;
                match self.hid_keyboard {
                    Some(p) => vec![self.keyboard_report(p)],
                    None => Vec::new(),
                }
            }
            InputEvent::ReleaseAll => {
                let mut out = Vec::new();
                let had_keys = !self.keys.is_empty() || self.modifiers != 0;
                let had_buttons = self.buttons != 0;
                self.keys.clear();
                self.modifiers = 0;
                self.buttons = 0;
                if had_keys && let Some(p) = self.hid_keyboard {
                    out.push(self.keyboard_report(p));
                }
                if had_buttons && let Some(p) = self.hid_mouse {
                    out.extend(self.mouse_reports(p, 0, 0, 0));
                }
                out
            }
        }
    }
}

/// One received UIBC packet (source side; used by the simulator and tests).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedInput {
    Generic { id: u8, describe: Vec<u8> },
    Hidc { path: u8, hid_type: u8, usage: u8, data: Vec<u8> },
}

/// Splits one UIBC packet off the front of `buf`: `Some((packet, consumed))`,
/// or `None` if more bytes are needed. Malformed packets are skipped whole.
pub fn parse_packet(buf: &[u8]) -> Option<(Option<ParsedInput>, usize)> {
    if buf.len() < 4 {
        return None;
    }
    let total = u16::from_be_bytes([buf[2], buf[3]]) as usize;
    if total < 4 {
        return Some((None, buf.len())); // garbage: drop what we have
    }
    if buf.len() < total {
        return None;
    }
    let timestamp = buf[0] & 0x10 != 0;
    let category = buf[1] & 0x0F;
    let body = &buf[if timestamp { 6 } else { 4 }..total];
    let parsed = match category {
        CATEGORY_GENERIC if body.len() >= 3 => {
            let len = u16::from_be_bytes([body[1], body[2]]) as usize;
            body.get(3..3 + len).map(|d| ParsedInput::Generic { id: body[0], describe: d.to_vec() })
        }
        CATEGORY_HIDC if body.len() >= 5 => {
            let len = u16::from_be_bytes([body[3], body[4]]) as usize;
            body.get(5..5 + len).map(|d| ParsedInput::Hidc { path: body[0], hid_type: body[1], usage: body[2], data: d.to_vec() })
        }
        _ => None,
    };
    Some((parsed, total))
}

impl ParsedInput {
    /// Human-readable summary, e.g. "mouse dx=4 dy=-2 buttons=1".
    pub fn describe(&self) -> String {
        match self {
            ParsedInput::Generic { id, describe: d } => {
                let xy = || (d.len() >= 6).then(|| (u16::from_be_bytes([d[2], d[3]]), u16::from_be_bytes([d[4], d[5]])));
                match *id {
                    generic_id::LEFT_DOWN => xy().map_or("left down".into(), |(x, y)| format!("left down at {x},{y}")),
                    generic_id::LEFT_UP => xy().map_or("left up".into(), |(x, y)| format!("left up at {x},{y}")),
                    generic_id::MOVE => xy().map_or("move".into(), |(x, y)| format!("pointer at {x},{y}")),
                    generic_id::KEY_DOWN | generic_id::KEY_UP if d.len() >= 3 => {
                        let c = u16::from_be_bytes([d[1], d[2]]);
                        let ch = char::from_u32(c as u32).filter(|c| !c.is_control()).map_or(format!("0x{c:02x}"), |c| format!("'{c}'"));
                        format!("key {} {ch}", if *id == generic_id::KEY_DOWN { "down" } else { "up" })
                    }
                    generic_id::VSCROLL | generic_id::HSCROLL if d.len() >= 2 => {
                        let v = u16::from_be_bytes([d[0], d[1]]);
                        let n = v & 0x1FFF;
                        let dir = match (*id == generic_id::VSCROLL, v & (1 << 13) != 0) {
                            (true, true) => "up",
                            (true, false) => "down",
                            (false, true) => "left",
                            (false, false) => "right",
                        };
                        format!("scroll {dir} {n}")
                    }
                    other => format!("generic input {other}"),
                }
            }
            ParsedInput::Hidc { hid_type, usage, data, .. } => {
                let kind = if *hid_type == InputType::Keyboard as u8 {
                    "keyboard"
                } else if *hid_type == InputType::Mouse as u8 {
                    "mouse"
                } else {
                    "hid"
                };
                if *usage == HID_USAGE_DESCRIPTOR {
                    return format!("{kind} descriptor ({} bytes)", data.len());
                }
                if *hid_type == InputType::Mouse as u8 && data.len() >= 4 {
                    format!("mouse buttons={} dx={} dy={} wheel={}", data[0], data[1] as i8, data[2] as i8, data[3] as i8)
                } else if *hid_type == InputType::Keyboard as u8 && data.len() >= 8 {
                    let keys: Vec<String> = data[2..8].iter().filter(|k| **k != 0).map(|k| format!("0x{k:02x}")).collect();
                    format!("keyboard mods=0x{:02x} keys=[{}]", data[0], keys.join(","))
                } else {
                    format!("{kind} report {data:02x?}")
                }
            }
        }
    }
}

/// HID keyboard usage -> ASCII for generic key events.
pub fn usage_to_ascii(usage: u8, shift: bool) -> Option<u16> {
    let c = match usage {
        0x04..=0x1d => {
            let base = b'a' + (usage - 0x04);
            if shift { base.to_ascii_uppercase() } else { base }
        }
        0x1e..=0x27 => {
            const DIGITS: &[u8; 10] = b"1234567890";
            const SHIFTED: &[u8; 10] = b"!@#$%^&*()";
            let i = (usage - 0x1e) as usize;
            if shift { SHIFTED[i] } else { DIGITS[i] }
        }
        0x28 => 0x0d, // Enter
        0x29 => 0x1b, // Escape
        0x2a => 0x08, // Backspace
        0x2b => 0x09, // Tab
        0x2c => b' ',
        0x2d => {
            if shift {
                b'_'
            } else {
                b'-'
            }
        }
        0x2e => {
            if shift {
                b'+'
            } else {
                b'='
            }
        }
        0x2f => {
            if shift {
                b'{'
            } else {
                b'['
            }
        }
        0x30 => {
            if shift {
                b'}'
            } else {
                b']'
            }
        }
        0x31 => {
            if shift {
                b'|'
            } else {
                b'\\'
            }
        }
        0x33 => {
            if shift {
                b':'
            } else {
                b';'
            }
        }
        0x34 => {
            if shift {
                b'"'
            } else {
                b'\''
            }
        }
        0x35 => {
            if shift {
                b'~'
            } else {
                b'`'
            }
        }
        0x36 => {
            if shift {
                b'<'
            } else {
                b','
            }
        }
        0x37 => {
            if shift {
                b'>'
            } else {
                b'.'
            }
        }
        0x38 => {
            if shift {
                b'?'
            } else {
                b'/'
            }
        }
        0x4c => 0x7f, // Delete
        _ => return None,
    };
    Some(c as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_roundtrip() {
        let ours = UibcCapability::sink_default();
        let s = ours.to_string();
        assert_eq!(
            s,
            "input_category_list=GENERIC, HIDC;generic_cap_list=Keyboard, Mouse, SingleTouch;hidc_cap_list=Keyboard/USB, Mouse/USB;port=none"
        );
        assert_eq!(UibcCapability::parse(&s), Some(ours));
        assert_eq!(UibcCapability::parse("none"), None);
    }

    #[test]
    fn parses_source_m4_selection() {
        let c = UibcCapability::parse("input_category_list=HIDC;generic_cap_list=none;hidc_cap_list=Mouse/USB, Keyboard/USB;port=7239")
            .unwrap();
        assert!(c.generic.is_empty());
        assert_eq!(c.hidc_path(InputType::Mouse), Some(HidPath::Usb));
        assert_eq!(c.port, Some(7239));
        // categories not listed in input_category_list are dropped
        let c = UibcCapability::parse("input_category_list=GENERIC;generic_cap_list=Mouse;hidc_cap_list=Mouse/USB;port=1000").unwrap();
        assert!(c.hidc.is_empty());
        assert!(c.has_generic(InputType::Mouse));
    }

    #[test]
    fn generic_packets_match_miraclecast_layout() {
        // touch/mouse move, 1 pointer: total = 8 + 5 = 13 -> padded to 14
        let p = generic_pointer(generic_id::MOVE, 0x0102, 0x0304);
        assert_eq!(p, vec![0x00, 0x00, 0x00, 0x0e, 0x02, 0x00, 0x06, 0x01, 0x00, 0x01, 0x02, 0x03, 0x04, 0x00]);
        // key down 'a': total 12
        let k = generic_key(generic_id::KEY_DOWN, b'a' as u16);
        assert_eq!(k, vec![0x00, 0x00, 0x00, 0x0c, 0x03, 0x00, 0x05, 0x00, 0x00, 0x61, 0x00, 0x00]);
        // wheel up 2 notches -> 0x6002, down 2 -> 0x4002 (lazycast control.c)
        assert_eq!(generic_scroll(true, 2).unwrap()[7..9], [0x60, 0x02]);
        assert_eq!(generic_scroll(true, -2).unwrap()[7..9], [0x40, 0x02]);
        assert!(generic_scroll(true, 0).is_none());
    }

    #[test]
    fn hidc_packets_match_lazycast_layout() {
        // lazycast controlhidc mouse report: total 14 = 4 hdr + 5 hidc hdr + 4 data + 1 pad
        let p = hidc_packet(HidPath::Usb, InputType::Mouse, HID_USAGE_REPORT, &[0x01, 0x05, 0xfb, 0x00]);
        assert_eq!(p, vec![0x00, 0x01, 0x00, 0x0e, 0x01, 0x01, 0x00, 0x00, 0x04, 0x01, 0x05, 0xfb, 0x00, 0x00]);
        let d = hidc_packet(HidPath::Usb, InputType::Mouse, HID_USAGE_DESCRIPTOR, MOUSE_DESCRIPTOR);
        assert_eq!(MOUSE_DESCRIPTOR.len(), 52);
        assert_eq!(&d[..9], &[0x00, 0x01, 0x00, 62, 0x01, 0x01, 0x01, 0x00, 52]);
        assert_eq!(KEYBOARD_DESCRIPTOR.len(), 63);
    }

    #[test]
    fn encoder_hidc_mouse_and_keyboard() {
        let neg = UibcCapability::parse("input_category_list=HIDC;hidc_cap_list=Keyboard/USB, Mouse/USB;port=7239").unwrap();
        let mut e = UibcEncoder::new(&neg);
        assert!(e.uses_relative_mouse());
        assert_eq!(e.initial_packets().len(), 2);
        // a 300px move is split into three relative reports
        let pk = e.encode(&InputEvent::PointerMove { x: 0, y: 0, dx: 300, dy: -10 });
        assert_eq!(pk.len(), 3);
        assert_eq!(pk[0][9..13], [0x00, 127, (-10i8) as u8, 0]);
        assert_eq!(pk[2][9..13], [0x00, 46, 0, 0]);
        let pk = e.encode(&InputEvent::Button { button: MouseButton::Right, pressed: true, x: 0, y: 0 });
        assert_eq!(pk[0][9], 0x02);
        let pk = e.encode(&InputEvent::Wheel { dx: 0, dy: -1 });
        assert_eq!(pk[0][9..13], [0x02, 0, 0, 0xff]);
        // ctrl+c
        let pk = e.encode(&InputEvent::Key { usage: 0x06, pressed: true, modifiers: modifier::LCTRL });
        assert_eq!(pk[0][9..17], [0x01, 0, 0x06, 0, 0, 0, 0, 0]);
        let pk = e.encode(&InputEvent::ReleaseAll);
        assert_eq!(pk.len(), 2);
        assert_eq!(pk[0][9..17], [0; 8]);
    }

    #[test]
    fn parse_what_the_encoder_writes() {
        let neg =
            UibcCapability::parse("input_category_list=GENERIC, HIDC;generic_cap_list=Mouse;hidc_cap_list=Keyboard/USB, Mouse/USB;port=1")
                .unwrap();
        let mut e = UibcEncoder::new(&neg);
        let mut stream: Vec<u8> = e.initial_packets().concat();
        stream.extend(e.encode(&InputEvent::PointerMove { x: 5, y: 6, dx: 3, dy: -2 }).concat());
        stream.extend(e.encode(&InputEvent::Key { usage: 0x04, pressed: true, modifiers: modifier::LSHIFT }).concat());
        stream.extend(generic_scroll(true, 1).unwrap());
        stream.extend(generic_pointer(generic_id::LEFT_DOWN, 640, 360));
        let mut out = Vec::new();
        let mut buf = &stream[..];
        while let Some((p, n)) = parse_packet(buf) {
            out.push(p.unwrap().describe());
            buf = &buf[n..];
        }
        assert_eq!(
            out,
            vec![
                "mouse descriptor (52 bytes)",
                "keyboard descriptor (63 bytes)",
                "mouse buttons=0 dx=3 dy=-2 wheel=0",
                "keyboard mods=0x02 keys=[0x04]",
                "scroll up 1",
                "left down at 640,360",
            ]
        );
        assert!(parse_packet(&stream[..3]).is_none());
    }

    #[test]
    fn encoder_generic_fallback() {
        let neg = UibcCapability::parse("input_category_list=GENERIC;generic_cap_list=Mouse, Keyboard;port=7239").unwrap();
        let mut e = UibcEncoder::new(&neg);
        assert!(!e.uses_relative_mouse());
        assert!(e.initial_packets().is_empty());
        let pk = e.encode(&InputEvent::PointerMove { x: 100, y: 200, dx: 1, dy: 1 });
        assert_eq!(pk[0][4], generic_id::MOVE);
        let pk = e.encode(&InputEvent::Button { button: MouseButton::Left, pressed: true, x: 100, y: 200 });
        assert_eq!(pk[0][4], generic_id::LEFT_DOWN);
        assert!(e.encode(&InputEvent::Button { button: MouseButton::Right, pressed: true, x: 1, y: 1 }).is_empty());
        let pk = e.encode(&InputEvent::Key { usage: 0x04, pressed: true, modifiers: modifier::LSHIFT });
        assert_eq!(pk[0][8..10], [0x00, b'A']);
    }
}
