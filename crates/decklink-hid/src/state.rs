use serde::{Deserialize, Serialize};

use crate::{
    GamepadButtons, GamepadReport, Hat, HidPacket, KeyModifiers, KeyboardReport, MediaKeys,
    MediaReport, MouseButtons, MouseReport, GAMEPAD_REPORT_ID, KEYBOARD_REPORT_ID,
    MEDIA_REPORT_ID, MOUSE_REPORT_ID,
};

/// Aggregated live controller state from Deck hardware (pre-profile mapping).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ControllerState {
    pub buttons: GamepadButtons,
    pub dpad_up: bool,
    pub dpad_down: bool,
    pub dpad_left: bool,
    pub dpad_right: bool,
    pub lx: f32,
    pub ly: f32,
    pub rx: f32,
    pub ry: f32,
    pub lt: f32,
    pub rt: f32,
    /// Relative trackpad motion (right pad preferred for mouse).
    pub trackpad_dx: f32,
    pub trackpad_dy: f32,
    pub trackpad_touch: bool,
    pub trackpad_click: bool,
    /// Gyro angular rate (rad/s) — pitch, yaw, roll.
    pub gyro_x: f32,
    pub gyro_y: f32,
    pub gyro_z: f32,
    /// Accelerometer (g).
    pub accel_x: f32,
    pub accel_y: f32,
    pub accel_z: f32,
    /// Deck battery 0..100.
    pub battery_pct: u8,
}

impl ControllerState {
    pub fn hat(&self) -> Hat {
        Hat::from_dpad(
            self.dpad_up,
            self.dpad_down,
            self.dpad_left,
            self.dpad_right,
        )
    }

    pub fn clear_relative(&mut self) {
        self.trackpad_dx = 0.0;
        self.trackpad_dy = 0.0;
    }

    pub fn to_gamepad_report(&self) -> GamepadReport {
        GamepadReport {
            buttons: self.buttons,
            hat: self.hat(),
            lx: GamepadReport::axis_from_f32(self.lx),
            ly: GamepadReport::axis_from_f32(self.ly),
            rx: GamepadReport::axis_from_f32(self.rx),
            ry: GamepadReport::axis_from_f32(self.ry),
            lt: GamepadReport::trigger_from_f32(self.lt),
            rt: GamepadReport::trigger_from_f32(self.rt),
        }
    }

    pub fn gamepad_packet(&self) -> HidPacket {
        let r = self.to_gamepad_report();
        HidPacket {
            report_id: GAMEPAD_REPORT_ID,
            data: r.pack().to_vec(),
        }
    }

    pub fn mouse_packet(&self, buttons: MouseButtons, dx: i8, dy: i8, wheel: i8) -> HidPacket {
        let r = MouseReport {
            buttons,
            dx,
            dy,
            wheel,
        };
        HidPacket {
            report_id: MOUSE_REPORT_ID,
            data: r.pack().to_vec(),
        }
    }

    pub fn media_packet(&self, keys: MediaKeys) -> HidPacket {
        let r = MediaReport { keys };
        HidPacket {
            report_id: MEDIA_REPORT_ID,
            data: r.pack().to_vec(),
        }
    }

    pub fn keyboard_packet(&self, modifiers: KeyModifiers, keys: [u8; 6]) -> HidPacket {
        let r = KeyboardReport { modifiers, keys };
        HidPacket {
            report_id: KEYBOARD_REPORT_ID,
            data: r.pack().to_vec(),
        }
    }
}
