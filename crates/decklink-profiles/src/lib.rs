//! Profile definitions and mapping from Deck ControllerState → HID packets.

mod mapper;
mod store;

pub use mapper::{map_state, MappedOutput, ProfileKind};
pub use store::{AppConfig, PairedTarget, ProfileStore};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    #[default]
    Gamepad,
    Desktop,
    Flight,
    Racing,
}

impl Profile {
    pub fn all() -> &'static [Profile] {
        &[
            Profile::Gamepad,
            Profile::Desktop,
            Profile::Flight,
            Profile::Racing,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            Profile::Gamepad => "Gamepad (Xbox)",
            Profile::Desktop => "Desktop & Media",
            Profile::Flight => "Flight Sim",
            Profile::Racing => "Racing (Gyro)",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Profile::Gamepad => "gamepad",
            Profile::Desktop => "desktop",
            Profile::Flight => "flight",
            Profile::Racing => "racing",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "gamepad" | "xbox" => Some(Profile::Gamepad),
            "desktop" | "media" => Some(Profile::Desktop),
            "flight" => Some(Profile::Flight),
            "racing" | "gyro" => Some(Profile::Racing),
            _ => None,
        }
    }
}

impl std::fmt::Display for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}
