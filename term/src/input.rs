//! Turning key presses and mouse actions into the bytes a terminal program expects.

use alacritty_terminal::term::TermMode;
use eframe::egui::{Key, Modifiers};

/// xterm's modifier parameter: 1 + shift + 2·alt + 4·ctrl.
fn mod_param(m: Modifiers) -> u8 {
    1 + m.shift as u8 + 2 * m.alt as u8 + 4 * m.ctrl as u8
}

/// Bytes for a key press that is not plain text (text arrives separately as a Text event).
/// `meta` says whether Alt acts as Meta (sending ESC before the key).
pub fn key_bytes(key: Key, m: Modifiers, mode: TermMode, meta: bool) -> Option<Vec<u8>> {
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    let modp = mod_param(m);
    let esc_alt = |b: &[u8]| -> Vec<u8> {
        let mut v = Vec::with_capacity(b.len() + 1);
        if m.alt && meta {
            v.push(0x1b);
        }
        v.extend_from_slice(b);
        v
    };
    let csi = |letter: char| -> Vec<u8> {
        if modp > 1 {
            format!("\x1b[1;{modp}{letter}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{letter}").into_bytes()
        } else {
            format!("\x1b[{letter}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> { if modp > 1 { format!("\x1b[{n};{modp}~").into_bytes() } else { format!("\x1b[{n}~").into_bytes() } };
    let ss3 = |letter: char| -> Vec<u8> {
        if modp > 1 { format!("\x1b[1;{modp}{letter}").into_bytes() } else { format!("\x1bO{letter}").into_bytes() }
    };

    let v = match key {
        Key::Enter => esc_alt(b"\r"),
        Key::Tab if m.shift => b"\x1b[Z".to_vec(),
        Key::Tab => esc_alt(b"\t"),
        Key::Backspace if m.ctrl => esc_alt(b"\x08"),
        Key::Backspace => esc_alt(b"\x7f"),
        Key::Escape => esc_alt(b"\x1b"),
        Key::ArrowUp => csi('A'),
        Key::ArrowDown => csi('B'),
        Key::ArrowRight => csi('C'),
        Key::ArrowLeft => csi('D'),
        Key::Home => csi('H'),
        Key::End => csi('F'),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::F1 => ss3('P'),
        Key::F2 => ss3('Q'),
        Key::F3 => ss3('R'),
        Key::F4 => ss3('S'),
        Key::F5 => tilde(15),
        Key::F6 => tilde(17),
        Key::F7 => tilde(18),
        Key::F8 => tilde(19),
        Key::F9 => tilde(20),
        Key::F10 => tilde(21),
        Key::F11 => tilde(23),
        Key::F12 => tilde(24),
        _ => {
            // Ctrl / Alt combinations with printable keys produce no Text event, so encode them here.
            let c = printable(key, m.shift)?;
            if m.ctrl {
                let b = ctrl_byte(c)?;
                return Some(esc_alt(&[b]));
            }
            if m.alt && meta {
                let mut v = vec![0x1b];
                let mut buf = [0u8; 4];
                v.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                return Some(v);
            }
            return None;
        }
    };
    Some(v)
}

/// The character a key types on a US layout (used only for Ctrl/Alt combinations).
fn printable(key: Key, shift: bool) -> Option<char> {
    let name = key.name();
    if name.len() == 1 {
        let c = name.chars().next()?;
        if c.is_ascii_alphabetic() {
            return Some(if shift { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() });
        }
        if c.is_ascii_digit() {
            return Some(c);
        }
    }
    Some(match key {
        Key::Space => ' ',
        Key::Minus => '-',
        Key::Equals => '=',
        Key::Plus => '+',
        Key::OpenBracket => '[',
        Key::CloseBracket => ']',
        Key::Backslash => '\\',
        Key::Slash => '/',
        Key::Semicolon => ';',
        Key::Quote => '\'',
        Key::Comma => ',',
        Key::Period => '.',
        Key::Backtick => '`',
        Key::Questionmark => '?',
        _ => return None,
    })
}

fn ctrl_byte(c: char) -> Option<u8> {
    Some(match c {
        'a'..='z' => c as u8 - b'a' + 1,
        'A'..='Z' => c as u8 - b'A' + 1,
        ' ' | '2' | '@' => 0,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '6' | '^' => 0x1e,
        '/' | '7' | '-' | '_' | '?' => 0x1f,
        '8' => 0x7f,
        _ => return None,
    })
}

/// Text to send for a paste, honouring bracketed-paste mode.
pub fn paste_bytes(text: &str, mode: TermMode) -> Vec<u8> {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    if mode.contains(TermMode::BRACKETED_PASTE) {
        // Strip any embedded end marker so pasted text cannot escape the bracket.
        let clean = text.replace("\x1b[201~", "");
        format!("\x1b[200~{clean}\x1b[201~").into_bytes()
    } else {
        text.into_bytes()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    Press,
    Release,
    Motion,
}

/// A mouse report for programs that asked for one (vim, htop, tmux…).
/// `button`: 0 left, 1 middle, 2 right, 3 none (motion), 64/65 wheel up/down.
pub fn mouse_report(button: u8, action: MouseAction, col: usize, row: usize, m: Modifiers, mode: TermMode) -> Option<Vec<u8>> {
    let mut code = button as u32;
    if action == MouseAction::Motion {
        code += 32;
    }
    if m.shift {
        code += 4;
    }
    if m.alt {
        code += 8;
    }
    if m.ctrl {
        code += 16;
    }
    let (x, y) = (col + 1, row + 1);
    if mode.contains(TermMode::SGR_MOUSE) {
        let suffix = if action == MouseAction::Release { 'm' } else { 'M' };
        return Some(format!("\x1b[<{code};{x};{y}{suffix}").into_bytes());
    }
    // Legacy encoding: release has no button information.
    if action == MouseAction::Release {
        code = 3 + (code & !3);
    }
    let enc = |v: usize| -> Option<Vec<u8>> {
        let v = v + 32;
        if mode.contains(TermMode::UTF8_MOUSE) {
            let c = char::from_u32(v as u32)?;
            let mut buf = [0u8; 4];
            Some(c.encode_utf8(&mut buf).as_bytes().to_vec())
        } else if v <= 255 {
            Some(vec![v as u8])
        } else {
            None
        }
    };
    let mut out = b"\x1b[M".to_vec();
    out.extend(enc(code as usize)?);
    out.extend(enc(x)?);
    out.extend(enc(y)?);
    Some(out)
}
