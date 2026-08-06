use thiserror::Error;

use decklink_hid::ControllerState;

#[derive(Debug, Error)]
pub enum InputError {
    #[error("no suitable Steam Deck input device found")]
    NoDevice,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Message(String),
}

#[derive(Debug, Clone)]
pub enum InputEvent {
    State(ControllerState),
    Error(String),
}

/// Control exclusive device grab from the app (Wi-Fi connect/disconnect).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputCommand {
    /// true = grab sticks away from Desktop; false = release. Trackpads are never grabbed.
    SetExclusive(bool),
    /// Freeze Steam (SIGSTOP) so stick→mouse cannot reach Desktop. Use only while linked.
    SetSteamFrozen(bool),
}

/// Shared handle for latest state.
#[derive(Debug, Default, Clone)]
pub struct InputHandle {
    pub state: ControllerState,
}
