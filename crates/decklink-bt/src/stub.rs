//! Non-Linux stub HOGP server for UI / profile development.

use tokio::sync::{mpsc, watch};
use tracing::info;

use decklink_hid::HidPacket;

use crate::{BtError, BtEvent, HogpServer};

pub async fn start_hogp(device_name: String) -> Result<HogpServer, BtError> {
    info!(
        "decklink-bt stub: simulating HOGP advertise as '{}' (BlueZ only on Linux/SteamOS)",
        device_name
    );

    let (report_tx, mut report_rx) = mpsc::channel::<HidPacket>(128);
    let (battery_tx, _battery_rx) = watch::channel(100u8);
    let (event_tx, event_rx) = mpsc::channel::<BtEvent>(32);
    let (stop_tx, mut stop_rx) = watch::channel(false);

    let _ = event_tx.send(BtEvent::Advertising(true)).await;

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = stop_rx.changed() => {
                    if *stop_rx.borrow() {
                        let _ = event_tx.send(BtEvent::Advertising(false)).await;
                        break;
                    }
                }
                pkt = report_rx.recv() => {
                    if pkt.is_none() { break; }
                    // discard in stub
                }
            }
        }
    });

    Ok(HogpServer {
        report_tx,
        battery_tx,
        event_rx,
        stop_tx,
    })
}
