//! BlueZ HOGP GATT server via bluer.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use bluer::adv::{Advertisement, Type as AdvType};
use bluer::gatt::local::{
    Application, Characteristic, CharacteristicNotify, CharacteristicNotifyMethod,
    CharacteristicRead, CharacteristicWrite, CharacteristicWriteMethod, Descriptor,
    DescriptorRead, Service,
};
use bluer::{AdapterEvent, Session, Uuid};
use futures::StreamExt;
use tokio::sync::{mpsc, watch, Mutex};
use tracing::{info, warn};

use decklink_hid::{HidPacket, APPEARANCE_GAMEPAD, HID_REPORT_MAP};

use crate::{BtError, BtEvent, HogpServer};

/// Bluetooth SIG 16-bit UUID → full 128-bit UUID (const-safe).
const fn uuid16(u: u16) -> Uuid {
    Uuid::from_u128(((u as u128) << 96) | 0x0000_1000_8000_0080_5f9b_34fb)
}

const UUID_HID: Uuid = uuid16(0x1812);
const UUID_DIS: Uuid = uuid16(0x180A);
const UUID_BATTERY: Uuid = uuid16(0x180F);
const UUID_HID_INFO: Uuid = uuid16(0x2A4A);
const UUID_REPORT_MAP: Uuid = uuid16(0x2A4B);
const UUID_HID_CTRL: Uuid = uuid16(0x2A4C);
const UUID_REPORT: Uuid = uuid16(0x2A4D);
const UUID_PROTOCOL_MODE: Uuid = uuid16(0x2A4E);
const UUID_REPORT_REF: Uuid = uuid16(0x2908);
const UUID_MANUFACTURER: Uuid = uuid16(0x2A29);
const UUID_MODEL: Uuid = uuid16(0x2A24);
const UUID_PNP_ID: Uuid = uuid16(0x2A50);
const UUID_BATTERY_LEVEL: Uuid = uuid16(0x2A19);

fn report_characteristic(
    report_id: u8,
    latest: Arc<Mutex<Vec<u8>>>,
    notifier_reg: mpsc::Sender<mpsc::Sender<Vec<u8>>>,
) -> Characteristic {
    let report_ref = vec![report_id, 0x01];
    let latest_r = latest.clone();

    Characteristic {
        uuid: UUID_REPORT,
        read: Some(CharacteristicRead {
            read: true,
            fun: Box::new(move |_req| {
                let latest_r = latest_r.clone();
                Box::pin(async move { Ok(latest_r.lock().await.clone()) })
            }),
            ..Default::default()
        }),
        notify: Some(CharacteristicNotify {
            notify: true,
            method: CharacteristicNotifyMethod::Fun(Box::new(move |mut notifier| {
                let notifier_reg = notifier_reg.clone();
                Box::pin(async move {
                    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(64);
                    let _ = notifier_reg.send(tx).await;
                    tokio::spawn(async move {
                        while let Some(value) = rx.recv().await {
                            if notifier.notify(value).await.is_err() {
                                break;
                            }
                        }
                    });
                })
            })),
            ..Default::default()
        }),
        descriptors: vec![Descriptor {
            uuid: UUID_REPORT_REF,
            read: Some(DescriptorRead {
                read: true,
                fun: Box::new(move |_req| {
                    let v = report_ref.clone();
                    Box::pin(async move { Ok(v) })
                }),
                ..Default::default()
            }),
            ..Default::default()
        }],
        ..Default::default()
    }
}

pub async fn start_hogp(device_name: String) -> Result<HogpServer, BtError> {
    let session = Session::new()
        .await
        .map_err(|e| BtError::Unavailable(e.to_string()))?;
    let adapter = session
        .default_adapter()
        .await
        .map_err(|e| BtError::Unavailable(e.to_string()))?;
    adapter
        .set_powered(true)
        .await
        .map_err(|e| BtError::Unavailable(e.to_string()))?;

    let (report_tx, mut report_rx) = mpsc::channel::<HidPacket>(128);
    let (battery_tx, battery_rx) = watch::channel(100u8);
    let (event_tx, event_rx) = mpsc::channel::<BtEvent>(32);
    let (stop_tx, mut stop_rx) = watch::channel(false);

    let latest1 = Arc::new(Mutex::new(vec![0u8; 13]));
    let latest2 = Arc::new(Mutex::new(vec![0u8; 4]));
    let latest3 = Arc::new(Mutex::new(vec![0u8; 1]));

    let notifiers1: Arc<Mutex<Vec<mpsc::Sender<Vec<u8>>>>> = Arc::new(Mutex::new(Vec::new()));
    let notifiers2: Arc<Mutex<Vec<mpsc::Sender<Vec<u8>>>>> = Arc::new(Mutex::new(Vec::new()));
    let notifiers3: Arc<Mutex<Vec<mpsc::Sender<Vec<u8>>>>> = Arc::new(Mutex::new(Vec::new()));

    let (reg1_tx, mut reg1_rx) = mpsc::channel(4);
    let (reg2_tx, mut reg2_rx) = mpsc::channel(4);
    let (reg3_tx, mut reg3_rx) = mpsc::channel(4);

    {
        let n1 = notifiers1.clone();
        tokio::spawn(async move {
            while let Some(tx) = reg1_rx.recv().await {
                n1.lock().await.push(tx);
            }
        });
        let n2 = notifiers2.clone();
        tokio::spawn(async move {
            while let Some(tx) = reg2_rx.recv().await {
                n2.lock().await.push(tx);
            }
        });
        let n3 = notifiers3.clone();
        tokio::spawn(async move {
            while let Some(tx) = reg3_rx.recv().await {
                n3.lock().await.push(tx);
            }
        });
    }

    let protocol_mode = Arc::new(Mutex::new(vec![0x01u8]));
    let battery_for_read = battery_rx.clone();

    let app = Application {
        services: vec![
            Service {
                uuid: UUID_HID,
                primary: true,
                characteristics: vec![
                    Characteristic {
                        uuid: UUID_PROTOCOL_MODE,
                        read: Some(CharacteristicRead {
                            read: true,
                            fun: Box::new({
                                let protocol_mode = protocol_mode.clone();
                                move |_req| {
                                    let protocol_mode = protocol_mode.clone();
                                    Box::pin(async move {
                                        Ok(protocol_mode.lock().await.clone())
                                    })
                                }
                            }),
                            ..Default::default()
                        }),
                        write: Some(CharacteristicWrite {
                            write: true,
                            write_without_response: true,
                            method: CharacteristicWriteMethod::Fun(Box::new({
                                let protocol_mode = protocol_mode.clone();
                                move |new_value, _req| {
                                    let protocol_mode = protocol_mode.clone();
                                    Box::pin(async move {
                                        *protocol_mode.lock().await = new_value;
                                        Ok(())
                                    })
                                }
                            })),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    Characteristic {
                        uuid: UUID_REPORT_MAP,
                        read: Some(CharacteristicRead {
                            read: true,
                            fun: Box::new(|_req| {
                                Box::pin(async move { Ok(HID_REPORT_MAP.to_vec()) })
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    Characteristic {
                        uuid: UUID_HID_INFO,
                        read: Some(CharacteristicRead {
                            read: true,
                            fun: Box::new(|_req| {
                                Box::pin(async move { Ok(vec![0x11, 0x01, 0x00, 0x02]) })
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    Characteristic {
                        uuid: UUID_HID_CTRL,
                        write: Some(CharacteristicWrite {
                            write: true,
                            write_without_response: true,
                            method: CharacteristicWriteMethod::Fun(Box::new(|_v, _r| {
                                Box::pin(async move { Ok(()) })
                            })),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    report_characteristic(1, latest1.clone(), reg1_tx),
                    report_characteristic(2, latest2.clone(), reg2_tx),
                    report_characteristic(3, latest3.clone(), reg3_tx),
                ],
                ..Default::default()
            },
            Service {
                uuid: UUID_DIS,
                primary: true,
                characteristics: vec![
                    Characteristic {
                        uuid: UUID_MANUFACTURER,
                        read: Some(CharacteristicRead {
                            read: true,
                            fun: Box::new(|_req| {
                                Box::pin(async move { Ok(b"DeckLink".to_vec()) })
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    Characteristic {
                        uuid: UUID_MODEL,
                        read: Some(CharacteristicRead {
                            read: true,
                            fun: Box::new(|_req| {
                                Box::pin(async move { Ok(b"DeckLink BT".to_vec()) })
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    Characteristic {
                        uuid: UUID_PNP_ID,
                        read: Some(CharacteristicRead {
                            read: true,
                            fun: Box::new(|_req| {
                                Box::pin(async move {
                                    Ok(vec![0x01, 0xDE, 0x28, 0x01, 0x11, 0x00, 0x01])
                                })
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            Service {
                uuid: UUID_BATTERY,
                primary: true,
                characteristics: vec![Characteristic {
                    uuid: UUID_BATTERY_LEVEL,
                    read: Some(CharacteristicRead {
                        read: true,
                        fun: Box::new(move |_req| {
                            let battery = battery_for_read.clone();
                            Box::pin(async move { Ok(vec![*battery.borrow()]) })
                        }),
                        ..Default::default()
                    }),
                    notify: Some(CharacteristicNotify {
                        notify: true,
                        method: CharacteristicNotifyMethod::Fun(Box::new({
                            let battery = battery_rx.clone();
                            move |mut notifier| {
                                let mut battery = battery.clone();
                                Box::pin(async move {
                                    tokio::spawn(async move {
                                        loop {
                                            if battery.changed().await.is_err() {
                                                break;
                                            }
                                            let v = *battery.borrow();
                                            if notifier.notify(vec![v]).await.is_err() {
                                                break;
                                            }
                                        }
                                    });
                                })
                            }
                        })),
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let app_handle = adapter
        .serve_gatt_application(app)
        .await
        .map_err(|e| BtError::Message(format!("GATT serve failed: {e}")))?;

    let mut uuids = BTreeSet::new();
    uuids.insert(UUID_HID);

    let le_adv = Advertisement {
        advertisement_type: AdvType::Peripheral,
        service_uuids: uuids,
        discoverable: Some(true),
        local_name: Some(device_name.clone()),
        appearance: Some(APPEARANCE_GAMEPAD),
        duration: Some(Duration::from_secs(0)),
        ..Default::default()
    };

    let adv_handle = adapter
        .advertise(le_adv)
        .await
        .map_err(|e| BtError::Message(format!("advertise failed: {e}")))?;

    let _ = event_tx.send(BtEvent::Advertising(true)).await;
    info!("HOGP advertising as '{}'", device_name);
    warn!("connection interval is host-negotiated; prefer 7.5–11.25 ms on the host");

    let mut device_events = adapter
        .events()
        .await
        .map_err(|e| BtError::Message(e.to_string()))?;
    let event_tx2 = event_tx.clone();
    let adapter2 = adapter.clone();
    tokio::spawn(async move {
        while let Some(evt) = device_events.next().await {
            match evt {
                AdapterEvent::DeviceAdded(addr) => {
                    if let Ok(dev) = adapter2.device(addr) {
                        if matches!(dev.is_connected().await, Ok(true)) {
                            let name = dev
                                .name()
                                .await
                                .ok()
                                .flatten()
                                .unwrap_or_else(|| addr.to_string());
                            let _ = event_tx2
                                .send(BtEvent::Connected {
                                    address: addr.to_string(),
                                    name,
                                })
                                .await;
                        }
                    }
                }
                AdapterEvent::DeviceRemoved(addr) => {
                    let _ = event_tx2
                        .send(BtEvent::Disconnected {
                            address: addr.to_string(),
                        })
                        .await;
                }
                _ => {}
            }
        }
    });

    let l1 = latest1.clone();
    let l2 = latest2.clone();
    let l3 = latest3.clone();
    let n1 = notifiers1.clone();
    let n2 = notifiers2.clone();
    let n3 = notifiers3.clone();
    tokio::spawn(async move {
        while let Some(pkt) = report_rx.recv().await {
            let (latest, notifiers) = match pkt.report_id {
                1 => (l1.clone(), n1.clone()),
                2 => (l2.clone(), n2.clone()),
                3 => (l3.clone(), n3.clone()),
                _ => continue,
            };
            *latest.lock().await = pkt.data.clone();
            for n in notifiers.lock().await.iter() {
                let _ = n.send(pkt.data.clone()).await;
            }
        }
    });

    tokio::spawn(async move {
        loop {
            if stop_rx.changed().await.is_err() {
                break;
            }
            if *stop_rx.borrow() {
                info!("stopping HOGP server");
                drop(adv_handle);
                drop(app_handle);
                let _ = event_tx.send(BtEvent::Advertising(false)).await;
                break;
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
