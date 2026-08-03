use decklink_hid::{
    ControllerState, GamepadButtons, GamepadReport, HidPacket, MediaKeys, MouseButtons,
    GAMEPAD_REPORT_ID, MEDIA_REPORT_ID, MOUSE_REPORT_ID,
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
            out.packets.extend(map_desktop(state));
        }
        Profile::Flight => {
            out.packets.push(map_flight(state));
        }
        Profile::Racing => {
            out.packets.push(map_racing(state));
        }
    }

    out
}

fn map_desktop(state: &ControllerState) -> Vec<HidPacket> {
    let mut packets = Vec::new();

    // Right trackpad → mouse; L2/R2 → right/left click (plan: RT left, LT right)
    let sens = 24.0;
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
    let mouse = MouseReportLite { buttons, dx, dy, wheel: 0 };
    if !mouse.is_idle() {
        packets.push(HidPacket {
            report_id: MOUSE_REPORT_ID,
            data: mouse.pack().to_vec(),
        });
    }

    // Media: face / bumpers
    let mut keys = MediaKeys::empty();
    if state.buttons.contains(GamepadButtons::A) {
        keys |= MediaKeys::PLAY;
    }
    if state.buttons.contains(GamepadButtons::R1) {
        keys |= MediaKeys::NEXT;
    }
    if state.buttons.contains(GamepadButtons::L1) {
        keys |= MediaKeys::PREV;
    }
    if state.buttons.contains(GamepadButtons::Y) {
        keys |= MediaKeys::VOL_UP;
    }
    if state.buttons.contains(GamepadButtons::X) {
        keys |= MediaKeys::VOL_DOWN;
    }
    if state.buttons.contains(GamepadButtons::B) {
        keys |= MediaKeys::MUTE;
    }
    if state.buttons.contains(GamepadButtons::GUIDE) {
        keys |= MediaKeys::HOME;
    }
    if !keys.is_empty() {
        packets.push(HidPacket {
            report_id: MEDIA_REPORT_ID,
            data: [keys.bits()].to_vec(),
        });
    }

    // Still emit a quiet gamepad so hosts that expect one stay happy
    packets.push(state.gamepad_packet());
    packets
}

/// Flight: sticks as axes, triggers as throttle/rudder blend on Z/Rz already present.
fn map_flight(state: &ControllerState) -> HidPacket {
    let mut g = state.to_gamepad_report();
    // Exaggerate stick precision for flight; use L4/R4 as extra buttons already in mask
    g.lx = GamepadReport::axis_from_f32(state.lx);
    g.ly = GamepadReport::axis_from_f32(state.ly);
    g.rx = GamepadReport::axis_from_f32(state.rx);
    g.ry = GamepadReport::axis_from_f32(state.ry);
    // Left stick Y also drives “throttle feel” onto LT when LT idle
    if state.lt < 0.05 {
        g.lt = GamepadReport::trigger_from_f32((-state.ly).clamp(0.0, 1.0));
    }
    HidPacket {
        report_id: GAMEPAD_REPORT_ID,
        data: g.pack().to_vec(),
    }
}

/// Racing: yaw gyro steers left stick X; accelerate/brake on triggers.
fn map_racing(state: &ControllerState) -> HidPacket {
    let mut g = state.to_gamepad_report();
    // gyro_y ~ yaw rate; scale into steer axis
    let steer = (state.gyro_y * 0.35 + state.lx * 0.25).clamp(-1.0, 1.0);
    g.lx = GamepadReport::axis_from_f32(steer);
    g.ly = 0;
    g.lt = GamepadReport::trigger_from_f32(state.lt); // brake
    g.rt = GamepadReport::trigger_from_f32(state.rt); // throttle
    HidPacket {
        report_id: GAMEPAD_REPORT_ID,
        data: g.pack().to_vec(),
    }
}

/// Local mouse helper to avoid re-export churn.
struct MouseReportLite {
    buttons: MouseButtons,
    dx: i8,
    dy: i8,
    wheel: i8,
}

impl MouseReportLite {
    fn pack(&self) -> [u8; 4] {
        [self.buttons.bits(), self.dx as u8, self.dy as u8, self.wheel as u8]
    }
    fn is_idle(&self) -> bool {
        self.buttons.is_empty() && self.dx == 0 && self.dy == 0 && self.wheel == 0
    }
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
    fn racing_uses_gyro() {
        let mut s = ControllerState::default();
        s.gyro_y = 2.0;
        let o = map_state(Profile::Racing, &s);
        assert_eq!(o.packets.len(), 1);
        let data = &o.packets[0].data;
        let lx = i16::from_le_bytes([data[3], data[4]]);
        assert!(lx != 0);
    }
}
