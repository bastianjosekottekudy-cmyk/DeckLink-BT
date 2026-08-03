use bitflags::bitflags;
use serde::{Deserialize, Serialize};

pub const GAMEPAD_REPORT_ID: u8 = 1;
/// buttons(2) + hat(1) + lx(2)+ly(2)+rx(2)+ry(2) + lt(1)+rt(1) = 13 bytes
pub const GAMEPAD_REPORT_LEN: usize = 13;

bitflags! {
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct GamepadButtons: u16 {
        const A      = 1 << 0;
        const B      = 1 << 1;
        const X      = 1 << 2;
        const Y      = 1 << 3;
        const L1     = 1 << 4;
        const R1     = 1 << 5;
        const SELECT = 1 << 6;
        const START  = 1 << 7;
        const L3     = 1 << 8;
        const R3     = 1 << 9;
        const L4     = 1 << 10;
        const R4     = 1 << 11;
        const L5     = 1 << 12;
        const R5     = 1 << 13;
        const GUIDE  = 1 << 14;
        const TOUCH  = 1 << 15;
    }
}

/// Hat / D-pad encoding per HID (0=N … 7=NW, 8=null/center).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum Hat {
    #[default]
    Center = 8,
    North = 0,
    NorthEast = 1,
    East = 2,
    SouthEast = 3,
    South = 4,
    SouthWest = 5,
    West = 6,
    NorthWest = 7,
}

impl Hat {
    pub fn from_dpad(up: bool, down: bool, left: bool, right: bool) -> Self {
        match (up, down, left, right) {
            (true, false, false, false) => Hat::North,
            (true, false, false, true) => Hat::NorthEast,
            (false, false, false, true) => Hat::East,
            (false, true, false, true) => Hat::SouthEast,
            (false, true, false, false) => Hat::South,
            (false, true, true, false) => Hat::SouthWest,
            (false, false, true, false) => Hat::West,
            (true, false, true, false) => Hat::NorthWest,
            _ => Hat::Center,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GamepadReport {
    pub buttons: GamepadButtons,
    pub hat: Hat,
    /// -32768..32767
    pub lx: i16,
    pub ly: i16,
    pub rx: i16,
    pub ry: i16,
    /// 0..255
    pub lt: u8,
    pub rt: u8,
}

impl Default for GamepadReport {
    fn default() -> Self {
        Self {
            buttons: GamepadButtons::empty(),
            hat: Hat::Center,
            lx: 0,
            ly: 0,
            rx: 0,
            ry: 0,
            lt: 0,
            rt: 0,
        }
    }
}

impl GamepadReport {
    pub fn pack(&self) -> [u8; GAMEPAD_REPORT_LEN] {
        let btn = self.buttons.bits();
        let mut out = [0u8; GAMEPAD_REPORT_LEN];
        out[0] = (btn & 0xFF) as u8;
        out[1] = (btn >> 8) as u8;
        out[2] = self.hat as u8;
        out[3..5].copy_from_slice(&self.lx.to_le_bytes());
        out[5..7].copy_from_slice(&self.ly.to_le_bytes());
        out[7..9].copy_from_slice(&self.rx.to_le_bytes());
        out[9..11].copy_from_slice(&self.ry.to_le_bytes());
        out[11] = self.lt;
        out[12] = self.rt;
        out
    }

    /// Convert float -1.0..1.0 stick axis to i16 HID value.
    pub fn axis_from_f32(v: f32) -> i16 {
        let c = v.clamp(-1.0, 1.0);
        (c * 32767.0) as i16
    }

    pub fn trigger_from_f32(v: f32) -> u8 {
        (v.clamp(0.0, 1.0) * 255.0) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_neutral() {
        let r = GamepadReport::default();
        let b = r.pack();
        assert_eq!(b.len(), GAMEPAD_REPORT_LEN);
        assert_eq!(b[2], 8); // center hat
    }

    #[test]
    fn pack_buttons_and_axes() {
        let mut r = GamepadReport::default();
        r.buttons = GamepadButtons::A | GamepadButtons::START;
        r.lx = 1000;
        r.lt = 200;
        let b = r.pack();
        assert_eq!(b[0] & 1, 1);
        assert_eq!(b[0] & 0x80, 0x80);
        assert_eq!(i16::from_le_bytes([b[3], b[4]]), 1000);
        assert_eq!(b[11], 200);
    }

    #[test]
    fn hat_from_dpad() {
        assert_eq!(Hat::from_dpad(true, false, false, false), Hat::North);
        assert_eq!(Hat::from_dpad(false, false, true, false), Hat::West);
        assert_eq!(Hat::from_dpad(false, false, false, false), Hat::Center);
    }
}
