use bitflags::bitflags;
use serde::{Deserialize, Serialize};

pub const MOUSE_REPORT_ID: u8 = 2;
pub const MOUSE_REPORT_LEN: usize = 4;

bitflags! {
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct MouseButtons: u8 {
        const LEFT   = 1 << 0;
        const RIGHT  = 1 << 1;
        const MIDDLE = 1 << 2;
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MouseReport {
    pub buttons: MouseButtons,
    pub dx: i8,
    pub dy: i8,
    pub wheel: i8,
}

impl MouseReport {
    pub fn pack(&self) -> [u8; MOUSE_REPORT_LEN] {
        [self.buttons.bits(), self.dx as u8, self.dy as u8, self.wheel as u8]
    }

    pub fn unpack(data: &[u8]) -> Option<Self> {
        if data.len() < MOUSE_REPORT_LEN {
            return None;
        }
        Some(Self {
            buttons: MouseButtons::from_bits_truncate(data[0]),
            dx: data[1] as i8,
            dy: data[2] as i8,
            wheel: data[3] as i8,
        })
    }

    pub fn is_idle(&self) -> bool {
        self.buttons.is_empty() && self.dx == 0 && self.dy == 0 && self.wheel == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_click() {
        let r = MouseReport {
            buttons: MouseButtons::LEFT,
            dx: 5,
            dy: -3,
            wheel: 0,
        };
        assert_eq!(r.pack(), [1, 5, 253, 0]);
    }
}
