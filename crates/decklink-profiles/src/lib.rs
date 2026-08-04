//! Profile definitions and mapping from Deck ControllerState → HID packets.

mod mapper;
mod store;

pub use mapper::{map_state, MappedOutput, ProfileKind};
pub use store::{AppConfig, PairedTarget, ProfileStore};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Profile {
    #[default]
    Gamepad,
    /// Trackpad mouse + face-button / stick keyboard.
    Desktop,
}

impl Profile {
    pub fn all() -> &'static [Profile] {
        &[Profile::Gamepad, Profile::Desktop]
    }

    pub fn label(self) -> &'static str {
        match self {
            Profile::Gamepad => "Xbox Controller",
            Profile::Desktop => "Keyboard & Mouse",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Profile::Gamepad => "gamepad",
            Profile::Desktop => "desktop",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "gamepad" | "xbox" | "controller" => Some(Profile::Gamepad),
            "desktop" | "media" | "keyboard" | "mouse" | "keyboard_mouse" | "km" => {
                Some(Profile::Desktop)
            }
            // Legacy profiles → Xbox
            "flight" | "racing" | "gyro" => Some(Profile::Gamepad),
            _ => None,
        }
    }
}

impl Serialize for Profile {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Profile {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Profile::parse(&s).unwrap_or_default())
    }
}

impl std::fmt::Display for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}
