use thiserror::Error;

use decklink_hid::HidPacket;

#[derive(Debug, Error)]
pub enum BtError {
    #[error("bluetooth unavailable: {0}")]
    Unavailable(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Message(String),
}

#[derive(Debug, Clone)]
pub enum BtEvent {
    Advertising(bool),
    Connected { address: String, name: String },
    Disconnected { address: String },
    Error(String),
}

#[derive(Debug, Clone, Default)]
pub struct BtStatus {
    pub advertising: bool,
    pub connected: bool,
    pub peer_address: Option<String>,
    pub peer_name: Option<String>,
}

/// Control handle for the HOGP server.
pub struct HogpServer {
    pub report_tx: tokio::sync::mpsc::Sender<HidPacket>,
    pub battery_tx: tokio::sync::watch::Sender<u8>,
    pub event_rx: tokio::sync::mpsc::Receiver<BtEvent>,
    pub stop_tx: tokio::sync::watch::Sender<bool>,
}

impl HogpServer {
    pub async fn send_report(&self, packet: HidPacket) -> Result<(), BtError> {
        self.report_tx
            .send(packet)
            .await
            .map_err(|_| BtError::Message("HOGP report channel closed".into()))
    }

    pub fn set_battery(&self, pct: u8) {
        let _ = self.battery_tx.send(pct.min(100));
    }

    pub fn stop(&self) {
        let _ = self.stop_tx.send(true);
    }
}
