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

/// Control exclusive device grab from the app (Bluetooth connect/disconnect).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputCommand {
    /// true = grab sticks/pads away from Desktop mouse; false = release for local use
    SetExclusive(bool),
}

/// Shared handle for latest state.
#[derive(Debug, Default, Clone)]
pub struct InputHandle {
    pub state: ControllerState,
}
