//! HID keyboard report packer + USB HID usage IDs (TapBoard-compatible).

use serde::{Deserialize, Serialize};

pub const KEYBOARD_REPORT_ID: u8 = 3;
/// modifiers(1) + reserved(1) + 6 keycodes
pub const KEYBOARD_REPORT_LEN: usize = 8;

/// USB HID Usage Page 0x07 keycodes (same set as TapBoard `HidKeyCodes`).
#[allow(dead_code)]
pub mod key {
    pub const A: u8 = 0x04;
    pub const B: u8 = 0x05;
    pub const C: u8 = 0x06;
    pub const D: u8 = 0x07;
    pub const E: u8 = 0x08;
    pub const F: u8 = 0x09;
    pub const G: u8 = 0x0A;
    pub const H: u8 = 0x0B;
    pub const I: u8 = 0x0C;
    pub const J: u8 = 0x0D;
    pub const K: u8 = 0x0E;
    pub const L: u8 = 0x0F;
    pub const M: u8 = 0x10;
    pub const N: u8 = 0x11;
    pub const O: u8 = 0x12;
    pub const P: u8 = 0x13;
    pub const Q: u8 = 0x14;
    pub const R: u8 = 0x15;
    pub const S: u8 = 0x16;
    pub const T: u8 = 0x17;
    pub const U: u8 = 0x18;
    pub const V: u8 = 0x19;
    pub const W: u8 = 0x1A;
    pub const X: u8 = 0x1B;
    pub const Y: u8 = 0x1C;
    pub const Z: u8 = 0x1D;
    pub const NUM_1: u8 = 0x1E;
    pub const NUM_2: u8 = 0x1F;
    pub const NUM_3: u8 = 0x20;
    pub const NUM_4: u8 = 0x21;
    pub const NUM_5: u8 = 0x22;
    pub const NUM_6: u8 = 0x23;
    pub const NUM_7: u8 = 0x24;
    pub const NUM_8: u8 = 0x25;
    pub const NUM_9: u8 = 0x26;
    pub const NUM_0: u8 = 0x27;
    pub const ENTER: u8 = 0x28;
    pub const ESCAPE: u8 = 0x29;
    pub const BACKSPACE: u8 = 0x2A;
    pub const TAB: u8 = 0x2B;
    pub const SPACE: u8 = 0x2C;
    pub const MINUS: u8 = 0x2D;
    pub const EQUAL: u8 = 0x2E;
    pub const LEFT_BRACKET: u8 = 0x2F;
    pub const RIGHT_BRACKET: u8 = 0x30;
    pub const BACKSLASH: u8 = 0x31;
    pub const SEMICOLON: u8 = 0x33;
    pub const APOSTROPHE: u8 = 0x34;
    pub const GRAVE: u8 = 0x35;
    pub const COMMA: u8 = 0x36;
    pub const PERIOD: u8 = 0x37;
    pub const SLASH: u8 = 0x38;
    pub const CAPS_LOCK: u8 = 0x39;
    pub const F1: u8 = 0x3A;
    pub const F2: u8 = 0x3B;
    pub const F3: u8 = 0x3C;
    pub const F4: u8 = 0x3D;
    pub const F5: u8 = 0x3E;
    pub const F6: u8 = 0x3F;
    pub const F7: u8 = 0x40;
    pub const F8: u8 = 0x41;
    pub const F9: u8 = 0x42;
    pub const F10: u8 = 0x43;
    pub const F11: u8 = 0x44;
    pub const F12: u8 = 0x45;
    pub const PRINT_SCREEN: u8 = 0x46;
    pub const INSERT: u8 = 0x49;
    pub const HOME: u8 = 0x4A;
    pub const PAGE_UP: u8 = 0x4B;
    pub const DELETE: u8 = 0x4C;
    pub const END: u8 = 0x4D;
    pub const PAGE_DOWN: u8 = 0x4E;
    pub const RIGHT: u8 = 0x4F;
    pub const LEFT: u8 = 0x50;
    pub const DOWN: u8 = 0x51;
    pub const UP: u8 = 0x52;
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct KeyModifiers: u8 {
        const LEFT_CTRL  = 1 << 0;
        const LEFT_SHIFT = 1 << 1;
        const LEFT_ALT   = 1 << 2;
        const LEFT_GUI   = 1 << 3;
        const RIGHT_CTRL  = 1 << 4;
        const RIGHT_SHIFT = 1 << 5;
        const RIGHT_ALT   = 1 << 6;
        const RIGHT_GUI   = 1 << 7;
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyboardReport {
    pub modifiers: KeyModifiers,
    pub keys: [u8; 6],
}

impl KeyboardReport {
    pub fn pack(&self) -> [u8; KEYBOARD_REPORT_LEN] {
        [
            self.modifiers.bits(),
            0,
            self.keys[0],
            self.keys[1],
            self.keys[2],
            self.keys[3],
            self.keys[4],
            self.keys[5],
        ]
    }

    pub fn push_key(&mut self, code: u8) {
        if code == 0 {
            return;
        }
        if self.keys.contains(&code) {
            return;
        }
        if let Some(slot) = self.keys.iter_mut().find(|k| **k == 0) {
            *slot = code;
        }
    }

    pub fn is_idle(&self) -> bool {
        self.modifiers.is_empty() && self.keys.iter().all(|&k| k == 0)
    }

    pub fn packet(modifiers: KeyModifiers, keys: [u8; 6]) -> crate::HidPacket {
        let r = Self { modifiers, keys };
        crate::HidPacket {
            report_id: KEYBOARD_REPORT_ID,
            data: r.pack().to_vec(),
        }
    }

    /// Momentary press packet pair helpers: (down, up-clear-mods).
    pub fn tap_packets(modifiers: KeyModifiers, code: u8) -> [crate::HidPacket; 2] {
        let mut keys = [0u8; 6];
        keys[0] = code;
        [
            Self::packet(modifiers, keys),
            Self::packet(KeyModifiers::empty(), [0; 6]),
        ]
    }
}

/// Map a Unicode character to (HID keycode, extra modifiers), TapBoard-style.
pub fn from_char(ch: char) -> Option<(u8, KeyModifiers)> {
    use key::*;
    match ch {
        ' ' => return Some((SPACE, KeyModifiers::empty())),
        '\n' | '\r' => return Some((ENTER, KeyModifiers::empty())),
        '\t' => return Some((TAB, KeyModifiers::empty())),
        _ => {}
    }
    let lower = ch.to_ascii_lowercase();
    if lower.is_ascii_lowercase() {
        let code = A + (lower as u8 - b'a');
        let mods = if ch.is_ascii_uppercase() {
            KeyModifiers::LEFT_SHIFT
        } else {
            KeyModifiers::empty()
        };
        return Some((code, mods));
    }
    if let Some(d) = ch.to_digit(10) {
        let code = if d == 0 { NUM_0 } else { NUM_1 + (d as u8 - 1) };
        return Some((code, KeyModifiers::empty()));
    }
    let shifted = KeyModifiers::LEFT_SHIFT;
    let none = KeyModifiers::empty();
    Some(match ch {
        '!' => (NUM_1, shifted),
        '@' => (NUM_2, shifted),
        '#' => (NUM_3, shifted),
        '$' => (NUM_4, shifted),
        '%' => (NUM_5, shifted),
        '^' => (NUM_6, shifted),
        '&' => (NUM_7, shifted),
        '*' => (NUM_8, shifted),
        '(' => (NUM_9, shifted),
        ')' => (NUM_0, shifted),
        '-' => (MINUS, none),
        '_' => (MINUS, shifted),
        '=' => (EQUAL, none),
        '+' => (EQUAL, shifted),
        '[' => (LEFT_BRACKET, none),
        '{' => (LEFT_BRACKET, shifted),
        ']' => (RIGHT_BRACKET, none),
        '}' => (RIGHT_BRACKET, shifted),
        '\\' => (BACKSLASH, none),
        '|' => (BACKSLASH, shifted),
        ';' => (SEMICOLON, none),
        ':' => (SEMICOLON, shifted),
        '\'' => (APOSTROPHE, none),
        '"' => (APOSTROPHE, shifted),
        '`' => (GRAVE, none),
        '~' => (GRAVE, shifted),
        ',' => (COMMA, none),
        '<' => (COMMA, shifted),
        '.' => (PERIOD, none),
        '>' => (PERIOD, shifted),
        '/' => (SLASH, none),
        '?' => (SLASH, shifted),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_enter() {
        let mut r = KeyboardReport::default();
        r.push_key(key::ENTER);
        let b = r.pack();
        assert_eq!(b[2], key::ENTER);
    }

    #[test]
    fn from_char_shift() {
        let (c, m) = from_char('A').unwrap();
        assert_eq!(c, key::A);
        assert!(m.contains(KeyModifiers::LEFT_SHIFT));
    }
}
