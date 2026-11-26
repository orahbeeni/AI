//! egui keys -> USB HID keyboard usages (page 0x07).

use egui::Key;
use wfd_core::uibc::modifier;

/// Modifier bit for modifier keys (they go in the report's bitmap).
pub fn modifier_bit(key: Key) -> Option<u8> {
    Some(match key {
        Key::ControlLeft => modifier::LCTRL,
        Key::ShiftLeft => modifier::LSHIFT,
        Key::AltLeft => modifier::LALT,
        Key::SuperLeft => modifier::LGUI,
        Key::ControlRight => modifier::RCTRL,
        Key::ShiftRight => modifier::RSHIFT,
        Key::AltRight => modifier::RALT,
        Key::SuperRight => modifier::RGUI,
        _ => return None,
    })
}

/// Modifier bitmap from egui's summary (used when the platform doesn't send
/// separate modifier key events).
pub fn modifiers_from_egui(m: egui::Modifiers) -> u8 {
    let mut b = 0;
    if m.ctrl {
        b |= modifier::LCTRL;
    }
    if m.shift {
        b |= modifier::LSHIFT;
    }
    if m.alt {
        b |= modifier::LALT;
    }
    if m.mac_cmd {
        b |= modifier::LGUI;
    }
    b
}

pub fn hid_usage(key: Key) -> Option<u8> {
    use Key::*;
    let u = match key {
        A => 0x04,
        B => 0x05,
        C => 0x06,
        D => 0x07,
        E => 0x08,
        F => 0x09,
        G => 0x0A,
        H => 0x0B,
        I => 0x0C,
        J => 0x0D,
        K => 0x0E,
        L => 0x0F,
        M => 0x10,
        N => 0x11,
        O => 0x12,
        P => 0x13,
        Q => 0x14,
        R => 0x15,
        S => 0x16,
        T => 0x17,
        U => 0x18,
        V => 0x19,
        W => 0x1A,
        X => 0x1B,
        Y => 0x1C,
        Z => 0x1D,
        Num1 | Exclamationmark => 0x1E,
        Num2 => 0x1F,
        Num3 => 0x20,
        Num4 => 0x21,
        Num5 => 0x22,
        Num6 => 0x23,
        Num7 => 0x24,
        Num8 => 0x25,
        Num9 => 0x26,
        Num0 => 0x27,
        Enter => 0x28,
        Escape => 0x29,
        Backspace => 0x2A,
        Tab => 0x2B,
        Space => 0x2C,
        Minus => 0x2D,
        Equals | Plus => 0x2E,
        OpenBracket | OpenCurlyBracket => 0x2F,
        CloseBracket | CloseCurlyBracket => 0x30,
        Backslash | Pipe => 0x31,
        Semicolon | Colon => 0x33,
        Quote => 0x34,
        Backtick => 0x35,
        Comma => 0x36,
        Period => 0x37,
        Slash | Questionmark => 0x38,
        CapsLock => 0x39,
        F1 => 0x3A,
        F2 => 0x3B,
        F3 => 0x3C,
        F4 => 0x3D,
        F5 => 0x3E,
        F6 => 0x3F,
        F7 => 0x40,
        F8 => 0x41,
        F9 => 0x42,
        F10 => 0x43,
        F11 => 0x44,
        F12 => 0x45,
        PrintScreen => 0x46,
        ScrollLock => 0x47,
        Pause => 0x48,
        Insert => 0x49,
        Home => 0x4A,
        PageUp => 0x4B,
        Delete => 0x4C,
        End => 0x4D,
        PageDown => 0x4E,
        ArrowRight => 0x4F,
        ArrowLeft => 0x50,
        ArrowDown => 0x51,
        ArrowUp => 0x52,
        NumLock => 0x53,
        IntlBackslash => 0x64,
        Menu => 0x65,
        F13 => 0x68,
        F14 => 0x69,
        F15 => 0x6A,
        F16 => 0x6B,
        F17 => 0x6C,
        F18 => 0x6D,
        F19 => 0x6E,
        F20 => 0x6F,
        F21 => 0x70,
        F22 => 0x71,
        F23 => 0x72,
        F24 => 0x73,
        AudioVolumeMute => 0x7F,
        AudioVolumeUp => 0x80,
        AudioVolumeDown => 0x81,
        _ => return None,
    };
    Some(u)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapping() {
        assert_eq!(hid_usage(Key::A), Some(0x04));
        assert_eq!(hid_usage(Key::Z), Some(0x1D));
        assert_eq!(hid_usage(Key::Num0), Some(0x27));
        assert_eq!(hid_usage(Key::Enter), Some(0x28));
        assert_eq!(hid_usage(Key::ArrowUp), Some(0x52));
        assert_eq!(hid_usage(Key::Colon), hid_usage(Key::Semicolon));
        assert_eq!(modifier_bit(Key::SuperLeft), Some(modifier::LGUI));
        assert_eq!(hid_usage(Key::ShiftLeft), None);
    }
}
