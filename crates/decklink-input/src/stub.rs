//! Non-Linux stub: idle state pump so the app/UI can run for development.

use std::time::Duration;

use tokio::sync::mpsc;
use tracing::info;

use decklink_hid::ControllerState;

use crate::{read_battery_percent, InputEvent};

pub async fn spawn_input_task(tx: mpsc::Sender<InputEvent>) -> Result<(), crate::InputError> {
    info!("decklink-input: stub mode (non-Linux) — emitting idle controller state");
    tokio::spawn(async move {
        loop {
            let mut state = ControllerState::default();
            state.battery_pct = read_battery_percent();
            if tx.send(InputEvent::State(state)).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(16)).await;
        }
    });
    Ok(())
}
