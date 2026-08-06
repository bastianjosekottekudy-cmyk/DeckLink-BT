use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use decklink_hid::{
    hid_key, idle_release_packets, ControllerState, GamepadButtons, HidPacket, KeyModifiers,
    KeyboardReport, MouseButtons, MouseReport, GAMEPAD_REPORT_ID, KEYBOARD_REPORT_ID,
    MOUSE_REPORT_ID,
};
use tracing::info;

use crate::Profile;

static LAST_MOUSE_BUTTONS: AtomicU8 = AtomicU8::new(0);

fn diag_on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        matches!(
            std::env::var("DECKLINK_DIAG").as_deref(),
            Ok("1") | Ok("true") | Ok("yes")
        )
    })
}

fn diag_mouse_throttle() -> bool {
    use std::sync::Mutex;
    static LAST: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    let last = LAST.get_or_init(|| Mutex::new(None));
    let mut g = last.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    if let Some(prev) = *g {
        if now.duration_since(prev) < Duration::from_millis(40) {
            return false;
        }
    }
    *g = Some(now);
    true
}

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
fn map_keyboard_mouse(state: &ControllerState) -> Vec<HidPacket> {
    let mut packets = Vec::new();

    // Optional middle-click from R3 only (no pad mouse).
    let mut buttons = MouseButtons::empty();
    if state.buttons.contains(GamepadButtons::R3) {
        buttons |= MouseButtons::MIDDLE;
    }
    let btn_bits = buttons.bits();
    let prev_btns = LAST_MOUSE_BUTTONS.swap(btn_bits, Ordering::Relaxed);
    if btn_bits != 0 || prev_btns != 0 {
        if diag_on() && diag_mouse_throttle() {
            info!("DIAG mouse hid buttons={btn_bits:#x} (no trackpad mapping)");
        }
        packets.push(HidPacket {
            report_id: MOUSE_REPORT_ID,
            data: MouseReport {
                buttons,
                dx: 0,
                dy: 0,
                wheel: 0,
            }
            .pack()
            .to_vec(),
        });
    }

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
        LAST_MOUSE_BUTTONS.store(0, Ordering::Relaxed);
        let mut s = ControllerState::default();
        s.rx = 0.9;
        s.ry = -0.9;
        let o = map_state(Profile::Desktop, &s);
        let mouse = o.packets.iter().find(|p| p.report_id == MOUSE_REPORT_ID);
        assert!(
            mouse.is_none() || (mouse.unwrap().data[1] == 0 && mouse.unwrap().data[2] == 0),
            "stick must not move cursor"
        );
    }

    #[test]
    fn trackpad_does_not_drive_mouse() {
        LAST_MOUSE_BUTTONS.store(0, Ordering::Relaxed);
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
