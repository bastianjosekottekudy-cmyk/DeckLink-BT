use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::Profile;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedTarget {
    pub name: String,
    pub address: String,
    pub last_connected: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub active_profile: Profile,
    pub device_name: String,
    pub paired_targets: Vec<PairedTarget>,
    pub advertise_on_start: bool,
    pub mouse_sensitivity: f32,
    pub gyro_steer_gain: f32,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            active_profile: Profile::Gamepad,
            device_name: "DeckLink BT".into(),
            paired_targets: Vec::new(),
            advertise_on_start: true,
            mouse_sensitivity: 24.0,
            gyro_steer_gain: 0.35,
        }
    }
}

pub struct ProfileStore {
    path: PathBuf,
    pub config: AppConfig,
}

impl ProfileStore {
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("decklink-bt")
    }

    pub fn default_path() -> PathBuf {
        Self::config_dir().join("config.json")
    }

    pub fn load() -> Result<Self, StoreError> {
        let path = Self::default_path();
        if path.exists() {
            let raw = fs::read_to_string(&path)?;
            let config: AppConfig = serde_json::from_str(&raw)?;
            Ok(Self { path, config })
        } else {
            Ok(Self {
                path,
                config: AppConfig::default(),
            })
        }
    }

    pub fn save(&self) -> Result<(), StoreError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(&self.config)?;
        fs::write(&self.path, raw)?;
        Ok(())
    }

    pub fn set_profile(&mut self, profile: Profile) {
        self.config.active_profile = profile;
    }

    pub fn upsert_target(&mut self, target: PairedTarget) {
        if let Some(existing) = self
            .config
            .paired_targets
            .iter_mut()
            .find(|t| t.address == target.address)
        {
            *existing = target;
        } else {
            self.config.paired_targets.push(target);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_roundtrip_json() {
        let c = AppConfig::default();
        let s = serde_json::to_string(&c).unwrap();
        let back: AppConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(back.device_name, "DeckLink BT");
    }
}
