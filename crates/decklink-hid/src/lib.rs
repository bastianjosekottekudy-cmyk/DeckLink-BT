//! HID report descriptors and packers for DeckLink BT (HOGP gamepad + mouse + keyboard).

mod descriptor;
mod gamepad;
mod mouse;
mod keyboard;
mod media;
mod state;

pub use descriptor::{HID_REPORT_MAP, APPEARANCE_GAMEPAD};
pub use gamepad::{GamepadButtons, GamepadReport, Hat, GAMEPAD_REPORT_ID, GAMEPAD_REPORT_LEN};
pub use mouse::{MouseButtons, MouseReport, MOUSE_REPORT_ID, MOUSE_REPORT_LEN};
pub use keyboard::{
    from_char as hid_from_char, key as hid_key, KeyModifiers, KeyboardReport, KEYBOARD_REPORT_ID,
    KEYBOARD_REPORT_LEN,
};
pub use media::{MediaKeys, MediaReport, MEDIA_REPORT_ID, MEDIA_REPORT_LEN};
pub use state::ControllerState;

/// Combined outbound HID payload (report id + body) ready for GATT notify.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HidPacket {
    pub report_id: u8,
    pub data: Vec<u8>,
}

impl HidPacket {
    pub fn as_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(1 + self.data.len());
        out.push(self.report_id);
        out.extend_from_slice(&self.data);
        out
    }
}

/// Neutral reports for gamepad + mouse + keyboard (clears sticky host state on profile switch).
pub fn idle_release_packets() -> Vec<HidPacket> {
    vec![
        HidPacket {
            report_id: GAMEPAD_REPORT_ID,
            data: GamepadReport::default().pack().to_vec(),
        },
        HidPacket {
            report_id: MOUSE_REPORT_ID,
            data: MouseReport::default().pack().to_vec(),
        },
        HidPacket {
            report_id: KEYBOARD_REPORT_ID,
            data: KeyboardReport::default().pack().to_vec(),
        },
    ]
}
