//! Key codes. The wire format is the Linux evdev code of the physical key, so the receiving
//! machine's own keyboard layout decides which character a key produces.

pub const KEY_LEFTCTRL: u16 = 29;
#[cfg(test)]
pub const KEY_A: u16 = 30;
pub const KEY_LEFTSHIFT: u16 = 42;
#[cfg(test)]
pub const KEY_C: u16 = 46;
#[cfg(test)]
pub const KEY_B: u16 = 48;
pub const KEY_RIGHTSHIFT: u16 = 54;
pub const KEY_LEFTALT: u16 = 56;
#[allow(dead_code)]
pub const KEY_CAPSLOCK: u16 = 58;
pub const KEY_RIGHTCTRL: u16 = 97;
pub const KEY_RIGHTALT: u16 = 100;
pub const KEY_LEFTMETA: u16 = 125;
pub const KEY_RIGHTMETA: u16 = 126;

pub fn is_modifier(code: u16) -> bool {
    matches!(
        code,
        KEY_LEFTCTRL | KEY_RIGHTCTRL | KEY_LEFTSHIFT | KEY_RIGHTSHIFT | KEY_LEFTALT | KEY_RIGHTALT | KEY_LEFTMETA | KEY_RIGHTMETA
    )
}

/// Swaps Command/Super with Control so Cmd+C on a Mac keyboard is Ctrl+C on Linux/Windows.
pub fn swap_cmd_ctrl(code: u16) -> u16 {
    match code {
        KEY_LEFTMETA => KEY_LEFTCTRL,
        KEY_LEFTCTRL => KEY_LEFTMETA,
        KEY_RIGHTMETA => KEY_RIGHTCTRL,
        KEY_RIGHTCTRL => KEY_RIGHTMETA,
        other => other,
    }
}

/// macOS virtual key code (kVK_*) to evdev, by physical position.
#[allow(dead_code)]
const MAC: &[(u16, u16)] = &[
    (0x00, 30),  // A
    (0x01, 31),  // S
    (0x02, 32),  // D
    (0x03, 33),  // F
    (0x04, 35),  // H
    (0x05, 34),  // G
    (0x06, 44),  // Z
    (0x07, 45),  // X
    (0x08, 46),  // C
    (0x09, 47),  // V
    (0x0A, 86),  // ISO section -> 102nd key
    (0x0B, 48),  // B
    (0x0C, 16),  // Q
    (0x0D, 17),  // W
    (0x0E, 18),  // E
    (0x0F, 19),  // R
    (0x10, 21),  // Y
    (0x11, 20),  // T
    (0x12, 2),   // 1
    (0x13, 3),   // 2
    (0x14, 4),   // 3
    (0x15, 5),   // 4
    (0x16, 7),   // 6
    (0x17, 6),   // 5
    (0x18, 13),  // =
    (0x19, 10),  // 9
    (0x1A, 8),   // 7
    (0x1B, 12),  // -
    (0x1C, 9),   // 8
    (0x1D, 11),  // 0
    (0x1E, 27),  // ]
    (0x1F, 24),  // O
    (0x20, 22),  // U
    (0x21, 26),  // [
    (0x22, 23),  // I
    (0x23, 25),  // P
    (0x24, 28),  // Return
    (0x25, 38),  // L
    (0x26, 36),  // J
    (0x27, 40),  // '
    (0x28, 37),  // K
    (0x29, 39),  // ;
    (0x2A, 43),  // backslash
    (0x2B, 51),  // ,
    (0x2C, 53),  // /
    (0x2D, 49),  // N
    (0x2E, 50),  // M
    (0x2F, 52),  // .
    (0x30, 15),  // Tab
    (0x31, 57),  // Space
    (0x32, 41),  // `
    (0x33, 14),  // Delete (backspace)
    (0x35, 1),   // Escape
    (0x36, 126), // Right Command
    (0x37, 125), // Command
    (0x38, 42),  // Shift
    (0x39, 58),  // Caps Lock
    (0x3A, 56),  // Option
    (0x3B, 29),  // Control
    (0x3C, 54),  // Right Shift
    (0x3D, 100), // Right Option
    (0x3E, 97),  // Right Control
    (0x40, 187), // F17
    (0x41, 83),  // keypad .
    (0x43, 55),  // keypad *
    (0x45, 78),  // keypad +
    (0x47, 69),  // keypad Clear -> Num Lock
    (0x48, 115), // Volume Up
    (0x49, 114), // Volume Down
    (0x4A, 113), // Mute
    (0x4B, 98),  // keypad /
    (0x4C, 96),  // keypad Enter
    (0x4E, 74),  // keypad -
    (0x4F, 188), // F18
    (0x50, 189), // F19
    (0x51, 117), // keypad =
    (0x52, 82),  // keypad 0
    (0x53, 79),  // keypad 1
    (0x54, 80),  // keypad 2
    (0x55, 81),  // keypad 3
    (0x56, 75),  // keypad 4
    (0x57, 76),  // keypad 5
    (0x58, 77),  // keypad 6
    (0x59, 71),  // keypad 7
    (0x5A, 190), // F20
    (0x5B, 72),  // keypad 8
    (0x5C, 73),  // keypad 9
    (0x5D, 124), // JIS Yen
    (0x5E, 89),  // JIS underscore (Ro)
    (0x5F, 95),  // JIS keypad comma
    (0x60, 63),  // F5
    (0x61, 64),  // F6
    (0x62, 65),  // F7
    (0x63, 61),  // F3
    (0x64, 66),  // F8
    (0x65, 67),  // F9
    (0x66, 94),  // JIS Eisu -> Muhenkan
    (0x67, 87),  // F11
    (0x68, 92),  // JIS Kana -> Henkan
    (0x69, 183), // F13
    (0x6A, 186), // F16
    (0x6B, 184), // F14
    (0x6D, 68),  // F10
    (0x6E, 127), // context menu
    (0x6F, 88),  // F12
    (0x71, 185), // F15
    (0x72, 110), // Help (Insert position)
    (0x73, 102), // Home
    (0x74, 104), // Page Up
    (0x75, 111), // Forward Delete
    (0x76, 62),  // F4
    (0x77, 107), // End
    (0x78, 60),  // F2
    (0x79, 109), // Page Down
    (0x7A, 59),  // F1
    (0x7B, 105), // Left
    (0x7C, 106), // Right
    (0x7D, 108), // Down
    (0x7E, 103), // Up
];

#[allow(dead_code)]
pub fn mac_to_evdev(kvk: u16) -> Option<u16> {
    MAC.iter().find(|(m, _)| *m == kvk).map(|(_, e)| *e)
}

#[allow(dead_code)]
pub fn evdev_to_mac(code: u16) -> Option<u16> {
    MAC.iter().find(|(_, e)| *e == code).map(|(m, _)| *m)
}

/// Windows set-1 scan codes. For codes 1..=88 without the E0 prefix evdev equals the scan code;
/// this table covers the E0-prefixed (extended) keys.
#[allow(dead_code)]
const WIN_EXTENDED: &[(u16, u16)] = &[
    (0x1C, 96),  // keypad Enter
    (0x1D, 97),  // Right Ctrl
    (0x35, 98),  // keypad /
    (0x37, 99),  // Print Screen
    (0x38, 100), // Right Alt
    (0x45, 69),  // Num Lock (reported as extended by low-level hooks)
    (0x47, 102), // Home
    (0x48, 103), // Up
    (0x49, 104), // Page Up
    (0x4B, 105), // Left
    (0x4D, 106), // Right
    (0x4F, 107), // End
    (0x50, 108), // Down
    (0x51, 109), // Page Down
    (0x52, 110), // Insert
    (0x53, 111), // Delete
    (0x5B, 125), // Left Windows
    (0x5C, 126), // Right Windows
    (0x5D, 127), // Menu
    (0x20, 113), // Mute
    (0x2E, 114), // Volume Down
    (0x30, 115), // Volume Up
    (0x22, 164), // Play/Pause
    (0x24, 166), // Stop
    (0x10, 165), // Previous track
    (0x19, 163), // Next track
];

/// Converts a Windows scan code (plus its extended flag) to evdev.
#[allow(dead_code)]
pub fn win_to_evdev(scan: u16, extended: bool) -> Option<u16> {
    if extended {
        return WIN_EXTENDED.iter().find(|(s, _)| *s == scan).map(|(_, e)| *e);
    }
    match scan {
        0x45 => Some(119), // Pause (sent without E0 in hooks)
        0x54 => Some(99),  // Alt+Print Screen
        0x57 => Some(87),  // F11
        0x58 => Some(88),  // F12
        0x64..=0x6E => Some(scan + 83), // F13..F23
        1..=0x53 | 0x56 => Some(scan),
        _ => None,
    }
}

/// Converts evdev to a Windows scan code and its extended flag.
#[allow(dead_code)]
pub fn evdev_to_win(code: u16) -> Option<(u16, bool)> {
    if let Some((s, _)) = WIN_EXTENDED.iter().find(|(_, e)| *e == code) {
        return Some((*s, true));
    }
    match code {
        119 => Some((0x45, false)),
        87 => Some((0x57, false)),
        88 => Some((0x58, false)),
        183..=193 => Some((code - 83, false)),
        1..=0x53 | 0x56 => Some((code, false)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_table_round_trips() {
        for (m, e) in MAC {
            assert_eq!(evdev_to_mac(*e), Some(*m), "evdev {e}");
            assert_eq!(mac_to_evdev(*m), Some(*e));
        }
        assert_eq!(mac_to_evdev(0x37), Some(KEY_LEFTMETA));
    }

    #[test]
    fn windows_codes_round_trip() {
        for code in 1..=255u16 {
            if let Some((scan, ext)) = evdev_to_win(code) {
                assert_eq!(win_to_evdev(scan, ext), Some(code), "evdev {code}");
            }
        }
        assert_eq!(win_to_evdev(0x1D, false), Some(KEY_LEFTCTRL));
        assert_eq!(win_to_evdev(0x1D, true), Some(KEY_RIGHTCTRL));
    }
}
