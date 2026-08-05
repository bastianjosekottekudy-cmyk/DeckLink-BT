//! Non-Linux stub: idle state pump so the app/UI can run for development.

use std::time::Duration;

use tokio::sync::mpsc;
use tracing::info;

use decklink_hid::ControllerState;

use crate::{read_battery_percent, InputCommand, InputEvent};

pub async fn spawn_input_task(
    tx: mpsc::Sender<InputEvent>,
    mut cmd_rx: mpsc::Receiver<InputCommand>,
) -> Result<(), crate::InputError> {
    info!("decklink-input: stub mode (non-Linux) — emitting idle controller state");
    tokio::spawn(async move {
        loop {
            while let Ok(cmd) = cmd_rx.try_recv() {
                match cmd {
                    InputCommand::SetExclusive(on) => info!("stub exclusive grab = {on}"),
                    InputCommand::SetSteamFrozen(on) => info!("stub steam freeze = {on}"),
                }
            }
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
