use decklink_hid::{
    hid_key, idle_release_packets, ControllerState, GamepadButtons, HidPacket, KeyModifiers,
    KeyboardReport, MouseButtons, MouseReport, GAMEPAD_REPORT_ID, KEYBOARD_REPORT_ID,
    MOUSE_REPORT_ID,
};

use crate::Profile;

/// Alias kept for UI / docs clarity.
pub type ProfileKind = Profile;

#[derive(Debug, Clone, Default)]
pub struct MappedOutput {
    pub packets: Vec<HidPacket>,
    pub battery_pct: u8,
}

/// Map live Deck state into HID reports.
///
/// Always emits **gamepad + mouse + keyboard** (report IDs 1–3) so the host keeps
/// all three collections subscribed on one BLE link. Switching Xbox ↔ Keyboard+Mouse
/// never requires disconnect/re-pair — only which collection carries live data changes.
pub fn map_state(profile: Profile, state: &ControllerState) -> MappedOutput {
    let mut out = MappedOutput {
        battery_pct: state.battery_pct,
        packets: Vec::with_capacity(3),
    };

    match profile {
        Profile::Gamepad => {
            out.packets.push(state.gamepad_packet());
            // Keep mouse/keyboard endpoints alive (idle) for instant profile switch.
            out.packets.extend(idle_mouse_keyboard());
        }
        Profile::Desktop => {
            // Keep gamepad endpoint alive (neutral) while KM is active.
            out.packets.push(idle_gamepad());
            out.packets.extend(map_keyboard_mouse(state));
        }
    }

    debug_assert_eq!(out.packets.len(), 3);
    out
}

fn idle_gamepad() -> HidPacket {
    idle_release_packets()
        .into_iter()
        .find(|p| p.report_id == GAMEPAD_REPORT_ID)
        .expect("idle gamepad")
}

fn idle_mouse_keyboard() -> Vec<HidPacket> {
    idle_release_packets()
        .into_iter()
        .filter(|p| p.report_id == MOUSE_REPORT_ID || p.report_id == KEYBOARD_REPORT_ID)
        .collect()
}

/// Keyboard & Mouse: right stick (primary) + right-pad deltas; face/D-pad/left-stick → keys.
fn map_keyboard_mouse(state: &ControllerState) -> Vec<HidPacket> {
    let mut packets = Vec::new();

    // Large deadzone: resting sticks must never produce host mouse motion.
    let mut mx = 0.0f32;
    let mut my = 0.0f32;
    const STICK_DZ: f32 = 0.35;
    if state.rx.abs() > STICK_DZ {
        mx += (state.rx.signum() * (state.rx.abs() - STICK_DZ)) * 12.0;
    }
    if state.ry.abs() > STICK_DZ {
        my += (state.ry.signum() * (state.ry.abs() - STICK_DZ)) * 12.0;
    }
    // Right-pad deltas from hidraw (already touch-gated upstream).
    if state.trackpad_touch {
        mx += state.trackpad_dx;
        my += state.trackpad_dy;
    }
    let dx = mx.round().clamp(-20.0, 20.0) as i8;
    let dy = my.round().clamp(-20.0, 20.0) as i8;
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
    fn gamepad_emits_all_three_collections() {
        let s = ControllerState::default();
        let o = map_state(Profile::Gamepad, &s);
        assert_eq!(o.packets.len(), 3);
        assert_eq!(o.packets[0].report_id, GAMEPAD_REPORT_ID);
        assert_eq!(o.packets[1].report_id, MOUSE_REPORT_ID);
        assert_eq!(o.packets[2].report_id, KEYBOARD_REPORT_ID);
    }

    #[test]
    fn keyboard_mouse_emits_all_three_collections() {
        let mut s = ControllerState::default();
        s.trackpad_dx = 2.0;
        s.buttons.insert(GamepadButtons::A);
        let o = map_state(Profile::Desktop, &s);
        assert_eq!(o.packets.len(), 3);
        assert_eq!(o.packets[0].report_id, GAMEPAD_REPORT_ID);
        assert_eq!(o.packets[1].report_id, MOUSE_REPORT_ID);
        assert_eq!(o.packets[2].report_id, KEYBOARD_REPORT_ID);
        assert_eq!(o.packets[2].data[2], hid_key::ENTER);
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
