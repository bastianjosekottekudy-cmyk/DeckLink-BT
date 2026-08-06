//! Relative mouse + keyboard via Win32 SendInput.

use std::collections::HashSet;

use decklink_hid::{KeyModifiers, KeyboardReport, MouseButtons, MouseReport};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN,
    MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_WHEEL, MOUSEINPUT, VIRTUAL_KEY, KEYBDINPUT,
};

pub struct Injector;

impl Injector {
    pub fn new() -> Self {
        Self
    }

    /// Release every tracked key/modifier/mouse button and clear tracking state.
    pub fn reset(
        &mut self,
        last_keys: &mut HashSet<u8>,
        last_mods: &mut KeyModifiers,
        last_mouse: &mut MouseButtons,
    ) {
        for code in last_keys.iter() {
            if let Some(vk) = hid_to_vk(*code) {
                key_event(vk, false);
            }
        }
        last_keys.clear();

        if last_mods.intersects(KeyModifiers::LEFT_CTRL | KeyModifiers::RIGHT_CTRL) {
            key_event(0x11, false);
        }
        if last_mods.intersects(KeyModifiers::LEFT_SHIFT | KeyModifiers::RIGHT_SHIFT) {
            key_event(0x10, false);
        }
        if last_mods.intersects(KeyModifiers::LEFT_ALT | KeyModifiers::RIGHT_ALT) {
            key_event(0x12, false);
        }
        if last_mods.contains(KeyModifiers::LEFT_GUI) {
            key_event(0x5B, false);
        }
        if last_mods.contains(KeyModifiers::RIGHT_GUI) {
            key_event(0x5C, false);
        }
        *last_mods = KeyModifiers::empty();

        if last_mouse.contains(MouseButtons::LEFT) {
            mouse_btn(MOUSEEVENTF_LEFTUP);
        }
        if last_mouse.contains(MouseButtons::RIGHT) {
            mouse_btn(MOUSEEVENTF_RIGHTUP);
        }
        if last_mouse.contains(MouseButtons::MIDDLE) {
            mouse_btn(MOUSEEVENTF_MIDDLEUP);
        }
        *last_mouse = MouseButtons::empty();
    }

    pub fn apply_mouse(&mut self, r: &MouseReport, last: &mut MouseButtons) {
        if r.dx != 0 || r.dy != 0 {
            mouse_move(r.dx as i32, r.dy as i32);
        }
        if r.wheel != 0 {
            mouse_wheel(r.wheel as i32 * 120);
        }

        let down = |mask: MouseButtons, down_f, up_f| {
            let was = last.contains(mask);
            let now = r.buttons.contains(mask);
            if now && !was {
                mouse_btn(down_f);
            } else if !now && was {
                mouse_btn(up_f);
            }
        };
        down(
            MouseButtons::LEFT,
            MOUSEEVENTF_LEFTDOWN,
            MOUSEEVENTF_LEFTUP,
        );
        down(
            MouseButtons::RIGHT,
            MOUSEEVENTF_RIGHTDOWN,
            MOUSEEVENTF_RIGHTUP,
        );
        down(
            MouseButtons::MIDDLE,
            MOUSEEVENTF_MIDDLEDOWN,
            MOUSEEVENTF_MIDDLEUP,
        );
        *last = r.buttons;
    }

    pub fn apply_keyboard(
        &mut self,
        r: &KeyboardReport,
        last_keys: &mut HashSet<u8>,
        last_mods: &mut KeyModifiers,
    ) {
        sync_mod(
            last_mods,
            r.modifiers,
            KeyModifiers::LEFT_CTRL | KeyModifiers::RIGHT_CTRL,
            0x11,
        );
        sync_mod(
            last_mods,
            r.modifiers,
            KeyModifiers::LEFT_SHIFT | KeyModifiers::RIGHT_SHIFT,
            0x10,
        );
        sync_mod(
            last_mods,
            r.modifiers,
            KeyModifiers::LEFT_ALT | KeyModifiers::RIGHT_ALT,
            0x12,
        );
        sync_mod(last_mods, r.modifiers, KeyModifiers::LEFT_GUI, 0x5B);
        sync_mod(last_mods, r.modifiers, KeyModifiers::RIGHT_GUI, 0x5C);
        *last_mods = r.modifiers;

        let now: HashSet<u8> = r.keys.iter().copied().filter(|&k| k != 0).collect();
        for code in last_keys.difference(&now) {
            if let Some(vk) = hid_to_vk(*code) {
                key_event(vk, false);
            }
        }
        for code in now.difference(last_keys) {
            if let Some(vk) = hid_to_vk(*code) {
                key_event(vk, true);
            }
        }
        *last_keys = now;
    }
}

fn sync_mod(last: &KeyModifiers, now: KeyModifiers, mask: KeyModifiers, vk: u16) {
    let was = last.intersects(mask);
    let is = now.intersects(mask);
    if is && !was {
        key_event(vk, true);
    } else if !is && was {
        key_event(vk, false);
    }
}

fn vk_needs_extended(vk: u16) -> bool {
    matches!(
        vk,
        0x21 | 0x22 | 0x23 | 0x24 | 0x25 | 0x26 | 0x27 | 0x28 | 0x2D | 0x2E | 0x2C | 0x13
    )
}

fn key_event(vk: u16, down: bool) {
    let mut flags = if down {
        KEYBD_EVENT_FLAGS(0)
    } else {
        KEYEVENTF_KEYUP
    };
    if vk_needs_extended(vk) {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        let _ = SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

fn mouse_move(dx: i32, dy: i32) {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: 0,
                dwFlags: MOUSEEVENTF_MOVE,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        let _ = SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

fn mouse_wheel(delta: i32) {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: delta as u32,
                dwFlags: MOUSEEVENTF_WHEEL,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        let _ = SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

fn mouse_btn(flags: windows::Win32::UI::Input::KeyboardAndMouse::MOUSE_EVENT_FLAGS) {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        let _ = SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

/// USB HID usage → Windows virtual-key.
fn hid_to_vk(hid: u8) -> Option<u16> {
    Some(match hid {
        0x04..=0x1D => 0x41 + (hid - 0x04) as u16, // A-Z
        0x1E..=0x26 => 0x31 + (hid - 0x1E) as u16, // 1-9
        0x27 => 0x30,                              // 0
        0x28 => 0x0D,                              // Enter
        0x29 => 0x1B,                              // Esc
        0x2A => 0x08,                              // Backspace
        0x2B => 0x09,                              // Tab
        0x2C => 0x20,                              // Space
        0x2D => 0xBD,                              // -
        0x2E => 0xBB,                              // =
        0x2F => 0xDB,                              // [
        0x30 => 0xDD,                              // ]
        0x31 => 0xDC,                              // \
        0x33 => 0xBA,                              // ;
        0x34 => 0xDE,                              // '
        0x35 => 0xC0,                              // `
        0x36 => 0xBC,                              // ,
        0x37 => 0xBE,                              // .
        0x38 => 0xBF,                              // /
        0x39 => 0x14,                              // CapsLock
        0x3A..=0x45 => 0x70 + (hid - 0x3A) as u16, // F1-F12
        0x46 => 0x2C,                              // PrintScreen
        0x47 => 0x91,                              // ScrollLock
        0x48 => 0x13,                              // Pause
        0x49 => 0x2D,                              // Insert
        0x4A => 0x24,                              // Home
        0x4B => 0x21,                              // PageUp
        0x4C => 0x2E,                              // Delete
        0x4D => 0x23,                              // End
        0x4E => 0x22,                              // PageDown
        0x4F => 0x27,                              // Right
        0x50 => 0x25,                              // Left
        0x51 => 0x28,                              // Down
        0x52 => 0x26,                              // Up
        _ => return None,
    })
}
