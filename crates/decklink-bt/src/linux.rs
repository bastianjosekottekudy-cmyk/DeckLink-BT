//! BlueZ HOGP GATT server via bluer.

use std::collections::BTreeSet;
use std::sync::Arc;

use bluer::adv::{Advertisement, Type as AdvType};
use bluer::agent::Agent;
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

/// Input Report characteristic (Report Reference type = 0x01).
///
/// Security: do **not** set encrypt_read. Requiring encryption here commonly
/// leaves hosts "connected" with CCCD never enabled, so no HID input arrives.
/// Bonding/encryption is still negotiated by BlueZ/the host as needed.
fn input_report_characteristic(
    report_id: u8,
    latest: Arc<Mutex<Vec<u8>>>,
    notifier_reg: mpsc::Sender<mpsc::Sender<Vec<u8>>>,
) -> Characteristic {
    let report_ref = vec![report_id, 0x01]; // Report ID, Input Report
    let latest_r = latest.clone();
    let latest_n = latest.clone();

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
                let latest_n = latest_n.clone();
                Box::pin(async move {
                    info!("host subscribed to HID input report {report_id}");
                    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(64);
                    if notifier_reg.send(tx).await.is_err() {
                        warn!("failed to register notifier for report {report_id}");
                        return;
                    }
                    // Push current value so the host HID stack wakes immediately.
                    let initial = latest_n.lock().await.clone();
                    if notifier.notify(initial).await.is_err() {
                        warn!("initial notify failed for report {report_id}");
                        return;
                    }
                    // Keep this Fun alive for the whole notification session.
                    loop {
                        tokio::select! {
                            _ = notifier.stopped() => break,
                            msg = rx.recv() => {
                                match msg {
                                    Some(value) => {
                                        if notifier.notify(value).await.is_err() {
                                            break;
                                        }
                                    }
                                    None => break,
                                }
                            }
                        }
                    }
                    info!("HID input report {report_id} notification session ended");
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

/// Keyboard LED Output Report (Report Reference type = 0x02). Some hosts
/// expect this during HID init even if they never write LEDs.
fn keyboard_output_report_characteristic() -> Characteristic {
    let report_ref = vec![3u8, 0x02];
    let leds = Arc::new(Mutex::new(vec![0u8]));
    let leds_r = leds.clone();
    Characteristic {
        uuid: UUID_REPORT,
        read: Some(CharacteristicRead {
            read: true,
            fun: Box::new(move |_req| {
                let leds_r = leds_r.clone();
                Box::pin(async move { Ok(leds_r.lock().await.clone()) })
            }),
            ..Default::default()
        }),
        write: Some(CharacteristicWrite {
            write: true,
            write_without_response: true,
            method: CharacteristicWriteMethod::Fun(Box::new(move |value, _req| {
                let leds = leds.clone();
                Box::pin(async move {
                    *leds.lock().await = value;
                    Ok(())
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

    // Windows shows the adapter Alias after connect (not just the adv local_name).
    // Keep it as DeckLink BT for the whole session — do NOT flip back to "steamdeck"
    // (that creates a second Bluetooth identity on the host).
    let previous_alias = adapter.alias().await.unwrap_or_default();
    if let Err(e) = adapter.set_alias(device_name.clone()).await {
        warn!("set_alias({device_name}): {e}");
    } else {
        info!("adapter alias set to '{device_name}' (was '{previous_alias}')");
    }

    // Become the BlueZ agent and auto-accept pairing/service auth so SteamOS/KDE
    // does not pop "Authorize" for DeckLink BT (same advertised name).
    let agent = Agent {
        request_default: true,
        request_authorization: Some(Box::new(|req| {
            Box::pin(async move {
                info!("auto-accept pairing authorization from {}", req.device);
                Ok(())
            })
        })),
        authorize_service: Some(Box::new(|req| {
            Box::pin(async move {
                info!(
                    "auto-accept service {} from {}",
                    req.service, req.device
                );
                Ok(())
            })
        })),
        request_confirmation: Some(Box::new(|req| {
            Box::pin(async move {
                info!(
                    "auto-confirm passkey {} for {}",
                    req.passkey, req.device
                );
                Ok(())
            })
        })),
        ..Default::default()
    };
    let agent_handle = match session.register_agent(agent).await {
        Ok(h) => {
            info!("registered BlueZ pairing agent (auto-accept)");
            Some(h)
        }
        Err(e) => {
            warn!("register_agent: {e} — system Bluetooth UI may ask to authorize");
            None
        }
    };

    // Pairable for bonding, but do NOT set adapter Discoverable — that also
    // advertises Classic BT as "steamdeck" alongside LE "DeckLink BT".
    if let Err(e) = adapter.set_discoverable(false).await {
        warn!("set_discoverable(false): {e}");
    }
    if let Err(e) = adapter.set_pairable(true).await {
        warn!("set_pairable: {e}");
    }

    let (report_tx, mut report_rx) = mpsc::channel::<HidPacket>(128);
    let (battery_tx, battery_rx) = watch::channel(100u8);
    let (event_tx, event_rx) = mpsc::channel::<BtEvent>(32);
    let (stop_tx, mut stop_rx) = watch::channel(false);

    let latest1 = Arc::new(Mutex::new(vec![0u8; 13])); // gamepad
    let latest2 = Arc::new(Mutex::new(vec![0u8; 4])); // mouse
    let latest3 = Arc::new(Mutex::new(vec![0u8; 8])); // keyboard

    let notifiers1: Arc<Mutex<Vec<mpsc::Sender<Vec<u8>>>>> = Arc::new(Mutex::new(Vec::new()));
    let notifiers2: Arc<Mutex<Vec<mpsc::Sender<Vec<u8>>>>> = Arc::new(Mutex::new(Vec::new()));
    let notifiers3: Arc<Mutex<Vec<mpsc::Sender<Vec<u8>>>>> = Arc::new(Mutex::new(Vec::new()));

    let (reg1_tx, mut reg1_rx) = mpsc::channel(8);
    let (reg2_tx, mut reg2_rx) = mpsc::channel(8);
    let (reg3_tx, mut reg3_rx) = mpsc::channel(8);

    {
        let n1 = notifiers1.clone();
        tokio::spawn(async move {
            while let Some(tx) = reg1_rx.recv().await {
                info!("HID report 1 (gamepad) notify session registered");
                n1.lock().await.push(tx);
            }
        });
        let n2 = notifiers2.clone();
        tokio::spawn(async move {
            while let Some(tx) = reg2_rx.recv().await {
                info!("HID report 2 (mouse) notify session registered");
                n2.lock().await.push(tx);
            }
        });
        let n3 = notifiers3.clone();
        tokio::spawn(async move {
            while let Some(tx) = reg3_rx.recv().await {
                info!("HID report 3 (keyboard) notify session registered");
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
                                // bcdHID=1.11, country=0, flags=RemoteWake|NormallyConnectable
                                Box::pin(async move { Ok(vec![0x11, 0x01, 0x00, 0x03]) })
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
                    input_report_characteristic(1, latest1.clone(), reg1_tx),
                    input_report_characteristic(2, latest2.clone(), reg2_tx),
                    input_report_characteristic(3, latest3.clone(), reg3_tx),
                    keyboard_output_report_characteristic(),
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
                                    // Bluetooth vendor source, VID 0x28DE (Valve), PID 0x1101, ver 1.0
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
                                    loop {
                                        tokio::select! {
                                            _ = notifier.stopped() => break,
                                            res = battery.changed() => {
                                                if res.is_err() {
                                                    break;
                                                }
                                                let v = *battery.borrow();
                                                if notifier.notify(vec![v]).await.is_err() {
                                                    break;
                                                }
                                            }
                                        }
                                    }
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
    uuids.insert(UUID_BATTERY);

    let le_adv = Advertisement {
        advertisement_type: AdvType::Peripheral,
        service_uuids: uuids,
        discoverable: Some(true),
        local_name: Some(device_name.clone()),
        appearance: Some(APPEARANCE_GAMEPAD),
        ..Default::default()
    };

    let adv_handle = adapter
        .advertise(le_adv)
        .await
        .map_err(|e| BtError::Message(format!("advertise failed: {e}")))?;

    let _ = event_tx.send(BtEvent::Advertising(true)).await;
    info!("HOGP advertising as '{}'", device_name);

    let mut device_events = adapter
        .events()
        .await
        .map_err(|e| BtError::Message(e.to_string()))?;
    let event_tx2 = event_tx.clone();
    let adapter2 = adapter.clone();
    let n1c = notifiers1.clone();
    let n2c = notifiers2.clone();
    let n3c = notifiers3.clone();
    tokio::spawn(async move {
        while let Some(evt) = device_events.next().await {
            match evt {
                AdapterEvent::DeviceAdded(addr) => {
                    if let Ok(dev) = adapter2.device(addr) {
                        if matches!(dev.is_connected().await, Ok(true)) {
                            if let Err(e) = dev.set_trusted(true).await {
                                warn!("set_trusted({addr}): {e}");
                            }
                            let name = dev
                                .name()
                                .await
                                .ok()
                                .flatten()
                                .unwrap_or_else(|| addr.to_string());
                            // Only treat as DeckLink host once HID notifies are on.
                            // DeviceAdded alone fires for many non-HOGP Bluetooth peers.
                            let n1c = n1c.clone();
                            let n2c = n2c.clone();
                            let n3c = n3c.clone();
                            let event_tx2 = event_tx2.clone();
                            let addr_s = addr.to_string();
                            tokio::spawn(async move {
                                for _ in 0..20 {
                                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                                    let c1 = n1c.lock().await.len();
                                    let c2 = n2c.lock().await.len();
                                    let c3 = n3c.lock().await.len();
                                    if c1 + c2 + c3 > 0 {
                                        info!(
                                            "HID host {name} ({addr_s}) subscriptions: \
                                             gamepad={c1} mouse={c2} keyboard={c3}"
                                        );
                                        let _ = event_tx2
                                            .send(BtEvent::Connected {
                                                address: addr_s,
                                                name,
                                            })
                                            .await;
                                        return;
                                    }
                                }
                                warn!(
                                    "BT peer {name} connected but never subscribed to HID — \
                                     ignoring (not a DeckLink host)"
                                );
                            });
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
            let mut dead = Vec::new();
            {
                let ns = notifiers.lock().await;
                for (i, n) in ns.iter().enumerate() {
                    if n.send(pkt.data.clone()).await.is_err() {
                        dead.push(i);
                    }
                }
            }
            if !dead.is_empty() {
                let mut ns = notifiers.lock().await;
                for i in dead.into_iter().rev() {
                    ns.swap_remove(i);
                }
            }
        }
    });

    let adapter_restore = adapter.clone();
    tokio::spawn(async move {
        // Keep agent registered until stop (dropping unregisters it).
        let _agent_handle = agent_handle;
        loop {
            if stop_rx.changed().await.is_err() {
                break;
            }
            if *stop_rx.borrow() {
                info!("stopping HOGP server");
                drop(adv_handle);
                drop(app_handle);
                // Leave alias as DeckLink BT so the host does not grow a second
                // "steamdeck" identity. Only restore if we changed it this session.
                if previous_alias != device_name
                    && !previous_alias.is_empty()
                    && previous_alias.to_ascii_lowercase() != "decklink bt"
                {
                    // Keep DeckLink BT — intentional (see comment above).
                    info!("leaving adapter alias as '{device_name}' (was '{previous_alias}')");
                }
                let _ = adapter_restore; // silence unused if alias kept
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
