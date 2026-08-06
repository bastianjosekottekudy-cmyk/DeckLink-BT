use decklink_hid::{
    hid_key, idle_release_packets, ControllerState, GamepadButtons, HidPacket, KeyModifiers,
    KeyboardReport, GAMEPAD_REPORT_ID, KEYBOARD_REPORT_ID,
};

use crate::Profile;

/// Alias kept for UI / docs clarity.
pub type ProfileKind = Profile;

#[derive(Debug, Clone, Default)]
pub struct MappedOutput {
    pub packets: Vec<HidPacket>,
    pub battery_pct: u8,
}

/// Map live Deck state into HID reports for the Wi‑Fi host.
pub fn map_state(profile: Profile, state: &ControllerState) -> MappedOutput {
    let mut out = MappedOutput {
        battery_pct: state.battery_pct,
        packets: Vec::with_capacity(3),
    };

    match profile {
        Profile::Gamepad => {
            out.packets.push(state.gamepad_packet());
        }
        Profile::Desktop => {
            out.packets.push(idle_gamepad());
            out.packets.extend(map_keyboard_mouse(state));
        }
    }

    out
}

fn idle_gamepad() -> HidPacket {
    idle_release_packets()
        .into_iter()
        .find(|p| p.report_id == GAMEPAD_REPORT_ID)
        .expect("idle gamepad")
}

/// Keyboard & Mouse: face/D-pad/left-stick → keys.
///
/// Physical Steam Deck trackpads are **not** mapped — they stay for the Deck/Desktop.
/// Host mouse comes only from the soft UI (buttons / on-screen pad).
/// Select+Start is reserved for profile toggle (must not emit Alt+Tab).
fn map_keyboard_mouse(state: &ControllerState) -> Vec<HidPacket> {
    let mut packets = Vec::new();

    let chord = state.buttons.contains(GamepadButtons::SELECT)
        && state.buttons.contains(GamepadButtons::START);

    let mut kb = KeyboardReport::default();
    if state.buttons.contains(GamepadButtons::L1) {
        kb.modifiers |= KeyModifiers::LEFT_CTRL;
    }
    if state.buttons.contains(GamepadButtons::R1) {
        kb.modifiers |= KeyModifiers::LEFT_SHIFT;
    }
    // Skip SELECT/START while chord held — otherwise Desktop maps to Alt+Tab.
    if !chord && state.buttons.contains(GamepadButtons::SELECT) {
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
    if !chord && state.buttons.contains(GamepadButtons::START) {
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
    use decklink_hid::MOUSE_REPORT_ID;

    #[test]
    fn gamepad_emits_gamepad_only() {
        let s = ControllerState::default();
        let o = map_state(Profile::Gamepad, &s);
        assert_eq!(o.packets.len(), 1);
        assert_eq!(o.packets[0].report_id, GAMEPAD_REPORT_ID);
    }

    #[test]
    fn keyboard_emits_keys_without_pad_mouse() {
        let mut s = ControllerState::default();
        s.rpad_touch = true;
        s.rpad_dx = 2.0;
        s.buttons.insert(GamepadButtons::A);
        let o = map_state(Profile::Desktop, &s);
        assert!(o.packets.len() >= 2);
        assert_eq!(o.packets[0].report_id, GAMEPAD_REPORT_ID);
        assert!(o
            .packets
            .iter()
            .any(|p| p.report_id == KEYBOARD_REPORT_ID
                && p.data.get(2) == Some(&hid_key::ENTER)));
        assert!(
            o.packets
                .iter()
                .filter(|p| p.report_id == MOUSE_REPORT_ID)
                .all(|p| p.data[1] == 0 && p.data[2] == 0),
            "trackpads must not move host cursor"
        );
    }

    #[test]
    fn stick_does_not_drive_mouse() {
        let mut s = ControllerState::default();
        s.rx = 0.9;
        s.ry = -0.9;
        let o = map_state(Profile::Desktop, &s);
        assert!(
            o.packets
                .iter()
                .find(|p| p.report_id == MOUSE_REPORT_ID)
                .is_none(),
            "stick must not move cursor"
        );
    }

    #[test]
    fn trackpad_does_not_drive_mouse() {
        let mut s = ControllerState::default();
        s.rpad_touch = true;
        s.rpad_dx = 4.0;
        s.rpad_dy = -3.0;
        s.lpad_click = true;
        let o = map_state(Profile::Desktop, &s);
        assert!(
            o.packets
                .iter()
                .find(|p| p.report_id == MOUSE_REPORT_ID)
                .is_none(),
            "physical pads stay on Deck"
        );
    }

    #[test]
    fn select_start_chord_not_alt_tab() {
        let mut s = ControllerState::default();
        s.buttons.insert(GamepadButtons::SELECT);
        s.buttons.insert(GamepadButtons::START);
        let o = map_state(Profile::Desktop, &s);
        let kb = o
            .packets
            .iter()
            .find(|p| p.report_id == KEYBOARD_REPORT_ID)
            .expect("keyboard");
        // modifiers byte: no Left Alt (bit 2)
        assert_eq!(kb.data[0] & 0x04, 0, "chord must not send Alt");
        // no Tab keycode in slots
        assert!(!kb.data[2..8].contains(&hid_key::TAB), "chord must not send Tab");
    }

    #[test]
    fn idle_release_clears_all_collections() {
        let pkts = idle_release_packets();
        assert_eq!(pkts.len(), 3);
        assert_eq!(pkts[0].report_id, GAMEPAD_REPORT_ID);
        assert_eq!(pkts[1].report_id, MOUSE_REPORT_ID);
        assert_eq!(pkts[2].report_id, KEYBOARD_REPORT_ID);
        assert_eq!(pkts[0].data[2], 8);
        assert!(pkts[1].data.iter().all(|&b| b == 0));
        assert!(pkts[2].data.iter().all(|&b| b == 0));
    }
}
