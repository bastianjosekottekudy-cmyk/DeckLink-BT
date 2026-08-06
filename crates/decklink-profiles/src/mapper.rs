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
            // Gamepad only — do not stream idle mouse/keyboard (Windows BLE mouse spam).
            // Profile switch still flushes idle_release_packets() from the app.
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

/// Keyboard & Mouse: Deck trackpads + right stick; face/D-pad/left-stick → keys.
///
/// - Either pad alone → move cursor
/// - Left pad click → left mouse button; right pad click → right mouse button
/// - Both pads touched together → vertical scroll (no cursor move)
fn map_keyboard_mouse(state: &ControllerState) -> Vec<HidPacket> {
    let mut packets = Vec::new();

    let both = state.lpad_touch && state.rpad_touch;
    let mut mx = 0.0f32;
    let mut my = 0.0f32;
    let mut wheel = 0i8;
    let mut src = "none";

    if both {
        // Two-finger scroll: average vertical motion from both pads.
        let scroll_y = (state.lpad_dy + state.rpad_dy) * 0.5;
        wheel = (scroll_y * 0.35).round().clamp(-7.0, 7.0) as i8;
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
        if !state.lpad_touch && !state.rpad_touch {
            const STICK_DZ: f32 = 0.45;
            if state.rx.abs() > STICK_DZ {
                mx += (state.rx.signum() * (state.rx.abs() - STICK_DZ)) * 8.0;
                src = "stick";
            }
            if state.ry.abs() > STICK_DZ {
                my += (state.ry.signum() * (state.ry.abs() - STICK_DZ)) * 8.0;
                src = "stick";
            }
        }
    }
    let dx = mx.round().clamp(-20.0, 20.0) as i8;
    let dy = my.round().clamp(-20.0, 20.0) as i8;

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
                 lpad_d=({:.2},{:.2}) rpad_d=({:.2},{:.2}) stick=({:.3},{:.3}) \
                 touch=L{}R{} click=L{}R{}",
                state.lpad_dx,
                state.lpad_dy,
                state.rpad_dx,
                state.rpad_dy,
                state.rx,
                state.ry,
                state.lpad_touch as u8,
                state.rpad_touch as u8,
                state.lpad_click as u8,
                state.rpad_click as u8,
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
    fn keyboard_mouse_emits_gamepad_and_keyboard() {
        let mut s = ControllerState::default();
        s.rx = 0.9; // past mouse deadzone
        s.buttons.insert(GamepadButtons::A);
        let o = map_state(Profile::Desktop, &s);
        assert!(o.packets.len() >= 2);
        assert_eq!(o.packets[0].report_id, GAMEPAD_REPORT_ID);
        assert!(o
            .packets
            .iter()
            .any(|p| p.report_id == KEYBOARD_REPORT_ID
                && p.data.get(2) == Some(&hid_key::ENTER)));
        assert!(o.packets.iter().any(|p| p.report_id == MOUSE_REPORT_ID));
    }

    #[test]
    fn trackpad_touch_drives_mouse() {
        let mut s = ControllerState::default();
        s.rpad_touch = true;
        s.rpad_dx = 4.0;
        s.rpad_dy = -3.0;
        let o = map_state(Profile::Desktop, &s);
        let mouse = o
            .packets
            .iter()
            .find(|p| p.report_id == MOUSE_REPORT_ID)
            .expect("mouse packet");
        assert_eq!(mouse.data[1], 4u8);
        assert_eq!(mouse.data[2] as i8, -3);
    }

    #[test]
    fn left_click_right_click_from_pads() {
        let mut s = ControllerState::default();
        s.lpad_click = true;
        s.rpad_click = true;
        let o = map_state(Profile::Desktop, &s);
        let mouse = o
            .packets
            .iter()
            .find(|p| p.report_id == MOUSE_REPORT_ID)
            .expect("mouse");
        assert_eq!(mouse.data[0] & 0b11, 0b11); // left+right
    }

    #[test]
    fn both_pads_scroll_not_move() {
        let mut s = ControllerState::default();
        s.lpad_touch = true;
        s.rpad_touch = true;
        s.lpad_dy = 10.0;
        s.rpad_dy = 10.0;
        s.lpad_dx = 5.0;
        s.rpad_dx = 5.0;
        let o = map_state(Profile::Desktop, &s);
        let mouse = o
            .packets
            .iter()
            .find(|p| p.report_id == MOUSE_REPORT_ID)
            .expect("mouse");
        assert_eq!(mouse.data[1], 0); // no dx while scrolling
        assert_eq!(mouse.data[2], 0); // no dy
        assert_ne!(mouse.data[3], 0); // wheel
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
