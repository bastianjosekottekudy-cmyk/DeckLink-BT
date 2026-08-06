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
            // Xbox only — sticks/triggers/buttons → ViGEm. Never emit keyboard/mouse.
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

/// Keyboard & Mouse profile:
/// - Left/right Steam trackpads → host mouse (move / click / two-finger scroll)
/// - Face buttons / D-pad → keys (no left-stick WASD — that belongs to Xbox mode)
/// - Select+Start reserved for profile toggle (not Alt+Tab)
fn map_keyboard_mouse(state: &ControllerState) -> Vec<HidPacket> {
    let mut packets = Vec::new();

    let both = state.lpad_touch && state.rpad_touch;
    let mut mx = 0.0f32;
    let mut my = 0.0f32;
    let mut wheel = 0i8;
    let mut src = "none";

    if both {
        let scroll_y = (state.lpad_dy + state.rpad_dy) * 0.5;
        wheel = (scroll_y * 0.5).round().clamp(-3.0, 3.0) as i8;
        src = "scroll";
    } else {
        if state.lpad_touch {
            mx += state.lpad_dx;
            my += state.lpad_dy;
            src = "lpad";
        }
        if state.rpad_touch {
            mx += state.rpad_dx;
            my += state.rpad_dy;
            src = if src == "lpad" { "both_pads" } else { "rpad" };
        }
    }
    let dx = mx.round().clamp(-6.0, 6.0) as i8;
    let dy = my.round().clamp(-6.0, 6.0) as i8;

    let mut buttons = MouseButtons::empty();
    if state.lpad_click {
        buttons |= MouseButtons::LEFT;
    }
    if state.rpad_click {
        buttons |= MouseButtons::RIGHT;
    }
    if state.buttons.contains(GamepadButtons::R3) {
        buttons |= MouseButtons::MIDDLE;
    }
    let btn_bits = buttons.bits();
    let prev_btns = LAST_MOUSE_BUTTONS.swap(btn_bits, Ordering::Relaxed);
    if dx != 0 || dy != 0 || wheel != 0 || btn_bits != 0 || prev_btns != 0 {
        if diag_on() && (dx != 0 || dy != 0 || wheel != 0) && diag_mouse_throttle() {
            info!(
                "DIAG mouse hid dx={dx} dy={dy} wheel={wheel} src={src} \
                 lpad_d=({:.2},{:.2}) rpad_d=({:.2},{:.2})",
                state.lpad_dx, state.lpad_dy, state.rpad_dx, state.rpad_dy,
            );
        }
        packets.push(HidPacket {
            report_id: MOUSE_REPORT_ID,
            data: MouseReport {
                buttons,
                dx,
                dy,
                wheel,
            }
            .pack()
            .to_vec(),
        });
    }

    let chord = state.buttons.contains(GamepadButtons::SELECT)
        && state.buttons.contains(GamepadButtons::START);

    let mut kb = KeyboardReport::default();
    if state.buttons.contains(GamepadButtons::L1) {
        kb.modifiers |= KeyModifiers::LEFT_CTRL;
    }
    if state.buttons.contains(GamepadButtons::R1) {
        kb.modifiers |= KeyModifiers::LEFT_SHIFT;
    }
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
    // Intentionally no left-stick WASD — sticks are for Xbox Controller mode.

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
    fn gamepad_emits_gamepad_only_with_sticks() {
        let mut s = ControllerState::default();
        s.lx = 0.5;
        s.ly = -0.5;
        s.buttons.insert(GamepadButtons::A);
        let o = map_state(Profile::Gamepad, &s);
        assert_eq!(o.packets.len(), 1);
        assert_eq!(o.packets[0].report_id, GAMEPAD_REPORT_ID);
        // packed lx at bytes 3-4
        let lx = i16::from_le_bytes([o.packets[0].data[3], o.packets[0].data[4]]);
        assert!(lx > 0, "left stick X must reach gamepad report");
        assert!(
            o.packets.iter().all(|p| p.report_id != KEYBOARD_REPORT_ID),
            "Xbox mode must not emit keyboard/WASD"
        );
    }

    #[test]
    fn desktop_stick_not_wasd() {
        let mut s = ControllerState::default();
        s.lx = -0.9;
        s.ly = -0.9;
        let o = map_state(Profile::Desktop, &s);
        let kb = o
            .packets
            .iter()
            .find(|p| p.report_id == KEYBOARD_REPORT_ID)
            .expect("kb");
        assert!(!kb.data[2..8].contains(&hid_key::W));
        assert!(!kb.data[2..8].contains(&hid_key::A));
        assert!(!kb.data[2..8].contains(&hid_key::S));
        assert!(!kb.data[2..8].contains(&hid_key::D));
    }

    #[test]
    fn trackpad_drives_mouse() {
        let mut s = ControllerState::default();
        s.rpad_touch = true;
        s.rpad_dx = 4.0;
        s.rpad_dy = -3.0;
        let o = map_state(Profile::Desktop, &s);
        let mouse = o
            .packets
            .iter()
            .find(|p| p.report_id == MOUSE_REPORT_ID)
            .expect("mouse");
        assert_eq!(mouse.data[1], 4u8);
        assert_eq!(mouse.data[2] as i8, -3);
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
        assert_eq!(kb.data[0] & 0x04, 0);
        assert!(!kb.data[2..8].contains(&hid_key::TAB));
    }

    #[test]
    fn idle_release_clears_all_collections() {
        let pkts = idle_release_packets();
        assert_eq!(pkts.len(), 3);
        assert_eq!(pkts[0].report_id, GAMEPAD_REPORT_ID);
        assert_eq!(pkts[1].report_id, MOUSE_REPORT_ID);
        assert_eq!(pkts[2].report_id, KEYBOARD_REPORT_ID);
    }
}
