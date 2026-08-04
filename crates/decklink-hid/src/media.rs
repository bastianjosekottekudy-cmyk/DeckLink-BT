use bitflags::bitflags;
use serde::{Deserialize, Serialize};

pub const MEDIA_REPORT_ID: u8 = 4; // unused in current Report Map (keyboard owns id 3)
pub const MEDIA_REPORT_LEN: usize = 1;

bitflags! {
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct MediaKeys: u8 {
        const NEXT     = 1 << 0;
        const PREV     = 1 << 1;
        const STOP     = 1 << 2;
        const PLAY     = 1 << 3;
        const MUTE     = 1 << 4;
        const VOL_UP   = 1 << 5;
        const VOL_DOWN = 1 << 6;
        const HOME     = 1 << 7;
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaReport {
    pub keys: MediaKeys,
}

impl MediaReport {
    pub fn pack(&self) -> [u8; MEDIA_REPORT_LEN] {
        [self.keys.bits()]
    }

    pub fn is_idle(&self) -> bool {
        self.keys.is_empty()
    }
}
