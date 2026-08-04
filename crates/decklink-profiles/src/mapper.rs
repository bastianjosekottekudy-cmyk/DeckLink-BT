use decklink_hid::{
    hid_key, ControllerState, GamepadButtons, HidPacket, KeyModifiers, KeyboardReport,
    MouseButtons, MouseReport, GAMEPAD_REPORT_ID, KEYBOARD_REPORT_ID, MOUSE_REPORT_ID,
};

use crate::Profile;

/// Alias kept for UI / docs clarity.
pub type ProfileKind = Profile;

#[derive(Debug, Clone, Default)]
pub struct MappedOutput {
    pub packets: Vec<HidPacket>,
    pub battery_pct: u8,
}

/// Map live Deck state into one or more HID reports for the active profile.
pub fn map_state(profile: Profile, state: &ControllerState) -> MappedOutput {
    let mut out = MappedOutput {
        battery_pct: state.battery_pct,
        packets: Vec::new(),
    };

    match profile {
        Profile::Gamepad => {
            out.packets.push(state.gamepad_packet());
        }
        Profile::Desktop => {
            out.packets.extend(map_keyboard_mouse(state));
        }
    }

    out
}

/// Keyboard & Mouse: right trackpad → mouse; buttons/D-pad/left-stick → keys.
fn map_keyboard_mouse(state: &ControllerState) -> Vec<HidPacket> {
    let mut packets = Vec::new();

    let sens = 28.0;
    let dx = (state.trackpad_dx * sens).clamp(-127.0, 127.0) as i8;
    let dy = (state.trackpad_dy * sens).clamp(-127.0, 127.0) as i8;
    let mut buttons = MouseButtons::empty();
    if state.rt > 0.4 || state.trackpad_click {
        buttons |= MouseButtons::LEFT;
    }
    if state.lt > 0.4 {
        buttons |= MouseButtons::RIGHT;
    }
    if state.buttons.contains(GamepadButtons::R3) {
        buttons |= MouseButtons::MIDDLE;
    }
    // Always notify so button releases reach the host
    let mouse = MouseReport {
        buttons,
        dx,
        dy,
        wheel: 0,
    };
    packets.push(HidPacket {
        report_id: MOUSE_REPORT_ID,
        data: mouse.pack().to_vec(),
    });

    let mut kb = KeyboardReport::default();
    if state.buttons.contains(GamepadButtons::L1) {
        kb.modifiers |= KeyModifiers::LEFT_CTRL;
    }
    if state.buttons.contains(GamepadButtons::R1) {
        kb.modifiers |= KeyModifiers::LEFT_SHIFT;
    }
    if state.buttons.contains(GamepadButtons::SELECT) {
        kb.modifiers |= KeyModifiers::LEFT_ALT;
    }
    if state.buttons.contains(GamepadButtons::GUIDE) {
        kb.modifiers |= KeyModifiers::LEFT_GUI;
    }

    if state.dpad_up {
        kb.push_key(hid_key::UP);
    }
    if state.dpad_down {
        kb.push_key(hid_key::DOWN);
    }
    if state.dpad_left {
        kb.push_key(hid_key::LEFT);
    }
    if state.dpad_right {
        kb.push_key(hid_key::RIGHT);
    }
    if state.buttons.contains(GamepadButtons::A) {
        kb.push_key(hid_key::ENTER);
    }
    if state.buttons.contains(GamepadButtons::B) {
        kb.push_key(hid_key::ESCAPE);
    }
    if state.buttons.contains(GamepadButtons::X) {
        kb.push_key(hid_key::BACKSPACE);
    }
    if state.buttons.contains(GamepadButtons::Y) {
        kb.push_key(hid_key::SPACE);
    }
    if state.buttons.contains(GamepadButtons::START) {
        kb.push_key(hid_key::TAB);
    }
    if state.buttons.contains(GamepadButtons::L4) {
        kb.push_key(hid_key::PAGE_UP);
    }
    if state.buttons.contains(GamepadButtons::R4) {
        kb.push_key(hid_key::PAGE_DOWN);
    }

    // Left stick → WASD
    if state.ly < -0.45 {
        kb.push_key(hid_key::W);
    }
    if state.ly > 0.45 {
        kb.push_key(hid_key::S);
    }
    if state.lx < -0.45 {
        kb.push_key(hid_key::A);
    }
    if state.lx > 0.45 {
        kb.push_key(hid_key::D);
    }

    packets.push(HidPacket {
        report_id: KEYBOARD_REPORT_ID,
        data: kb.pack().to_vec(),
    });

    packets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gamepad_emits_one_packet() {
        let s = ControllerState::default();
        let o = map_state(Profile::Gamepad, &s);
        assert_eq!(o.packets.len(), 1);
        assert_eq!(o.packets[0].report_id, GAMEPAD_REPORT_ID);
    }

    #[test]
    fn keyboard_mouse_emits_mouse_and_keyboard() {
        let mut s = ControllerState::default();
        s.trackpad_dx = 2.0;
        s.buttons.insert(GamepadButtons::A);
        let o = map_state(Profile::Desktop, &s);
        assert_eq!(o.packets.len(), 2);
        assert_eq!(o.packets[0].report_id, MOUSE_REPORT_ID);
        assert_eq!(o.packets[1].report_id, KEYBOARD_REPORT_ID);
        assert_eq!(o.packets[1].data[2], hid_key::ENTER);
    }
}
