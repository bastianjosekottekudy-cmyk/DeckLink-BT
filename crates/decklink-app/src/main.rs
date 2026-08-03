//! DeckLink BT application entry — wires input → profiles → HOGP + Slint UI.

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::Parser;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use decklink_bt::{start_hogp, BtEvent, HogpServer};
use decklink_input::{spawn_input_task, InputEvent};
use decklink_profiles::{map_state, PairedTarget, Profile, ProfileStore};
use decklink_ui::{format_targets, index_from_profile, profile_from_index, MainWindow};
use slint::ComponentHandle;

#[derive(Parser, Debug)]
#[command(
    name = "decklink-bt",
    version,
    about = "Steam Deck as a driverless BLE HOGP gamepad"
)]
struct Cli {
    /// Start without opening the Slint window (headless / CI)
    #[arg(long)]
    headless: bool,

    /// Begin advertising immediately
    #[arg(long)]
    advertise: bool,

    /// Profile: gamepad | desktop | flight | racing
    #[arg(long, default_value = "gamepad")]
    profile: String,

    /// Override BLE local name
    #[arg(long)]
    name: Option<String>,
}

struct Shared {
    store: ProfileStore,
    hogp: Option<HogpServer>,
    advertising: bool,
    connected: bool,
    peer_name: String,
    status: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "decklink_bt=info,decklink_app=info".into()),
        )
        .init();

    let cli = Cli::parse();
    let mut store = ProfileStore::load().context("load config")?;
    if let Some(p) = Profile::parse(&cli.profile) {
        store.set_profile(p);
    }
    if let Some(name) = &cli.name {
        store.config.device_name = name.clone();
    }
    if cli.advertise {
        store.config.advertise_on_start = true;
    }
    let _ = store.save();

    let (input_tx, mut input_rx) = mpsc::channel::<InputEvent>(256);
    if let Err(e) = spawn_input_task(input_tx).await {
        warn!("input capture: {e} (continuing with idle state)");
    }

    let shared = Arc::new(Mutex::new(Shared {
        store,
        hogp: None,
        advertising: false,
        connected: false,
        peer_name: "—".into(),
        status: "Ready".into(),
    }));

    if cli.headless {
        return run_headless(shared, &mut input_rx).await;
    }

    run_ui(shared, input_rx)
}

async fn ensure_advertising(shared: &Arc<Mutex<Shared>>) -> Result<()> {
    {
        let g = shared.lock().unwrap();
        if g.hogp.is_some() {
            return Ok(());
        }
    }
    let name = {
        shared.lock().unwrap().store.config.device_name.clone()
    };

    match start_hogp(name).await {
        Ok(server) => {
            let mut g = shared.lock().unwrap();
            g.advertising = true;
            g.status = "Advertising as BLE gamepad…".into();
            g.hogp = Some(server);
            info!("HOGP started");
            Ok(())
        }
        Err(e) => {
            let mut g = shared.lock().unwrap();
            g.status = format!("Bluetooth error: {e}");
            Err(e.into())
        }
    }
}

async fn stop_advertising(shared: &Arc<Mutex<Shared>>) {
    let mut g = shared.lock().unwrap();
    if let Some(h) = g.hogp.take() {
        h.stop();
    }
    g.advertising = false;
    g.connected = false;
    g.status = "Advertising stopped".into();
}

async fn run_headless(
    shared: Arc<Mutex<Shared>>,
    input_rx: &mut mpsc::Receiver<InputEvent>,
) -> Result<()> {
    info!("headless mode");
    let advertise = shared.lock().unwrap().store.config.advertise_on_start;
    if advertise {
        if let Err(e) = ensure_advertising(&shared).await {
            error!("{e}");
        }
    }

    loop {
        tokio::select! {
            ev = input_rx.recv() => {
                match ev {
                    Some(InputEvent::State(state)) => {
                        pump_reports(&shared, &state).await;
                        drain_bt_events(&shared).await;
                    }
                    Some(InputEvent::Error(e)) => warn!("input: {e}"),
                    None => break,
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {
                drain_bt_events(&shared).await;
            }
        }
    }
    Ok(())
}

async fn pump_reports(shared: &Arc<Mutex<Shared>>, state: &decklink_hid::ControllerState) {
    let (packets, report_tx) = {
        let g = shared.lock().unwrap();
        let profile = g.store.config.active_profile;
        let mapped = map_state(profile, state);
        if let Some(h) = g.hogp.as_ref() {
            h.set_battery(mapped.battery_pct);
        }
        let tx = g.hogp.as_ref().map(|h| h.report_tx.clone());
        (mapped.packets, tx)
    };
    if let Some(tx) = report_tx {
        for pkt in packets {
            let _ = tx.send(pkt).await;
        }
    }
}

async fn drain_bt_events(shared: &Arc<Mutex<Shared>>) {
    loop {
        let ev = {
            let mut g = shared.lock().unwrap();
            g.hogp.as_mut().and_then(|h| h.event_rx.try_recv().ok())
        };
        let Some(ev) = ev else { break };
        let mut g = shared.lock().unwrap();
        match ev {
            BtEvent::Advertising(on) => {
                g.advertising = on;
                g.status = if on {
                    "Advertising as BLE gamepad…".into()
                } else {
                    "Advertising stopped".into()
                };
            }
            BtEvent::Connected { address, name } => {
                g.connected = true;
                g.peer_name = name.clone();
                g.status = format!("Connected to {name}");
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs().to_string())
                    .unwrap_or_default();
                g.store.upsert_target(PairedTarget {
                    name,
                    address,
                    last_connected: Some(now),
                });
                let _ = g.store.save();
            }
            BtEvent::Disconnected { address } => {
                g.connected = false;
                g.status = format!("Disconnected ({address})");
            }
            BtEvent::Error(e) => {
                g.status = format!("BT error: {e}");
                error!("{e}");
            }
        }
    }
}

fn run_ui(shared: Arc<Mutex<Shared>>, mut input_rx: mpsc::Receiver<InputEvent>) -> Result<()> {
    let ui = MainWindow::new().context("create Slint window")?;

    {
        let g = shared.lock().unwrap();
        ui.set_selected_profile(index_from_profile(g.store.config.active_profile));
        ui.set_targets_text(format_targets(&g.store.config.paired_targets).into());
        ui.set_status_text(g.status.clone().into());
        ui.set_battery_pct(100);
    }

    // Commands from UI → background via channels
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<UiCommand>();

    {
        let tx = cmd_tx.clone();
        ui.on_start_advertise(move || {
            let _ = tx.send(UiCommand::StartAdvertise);
        });
    }
    {
        let tx = cmd_tx.clone();
        ui.on_stop_advertise(move || {
            let _ = tx.send(UiCommand::StopAdvertise);
        });
    }
    {
        let shared_prof = shared.clone();
        ui.on_profile_changed(move |idx| {
            let mut g = shared_prof.lock().unwrap();
            let p = profile_from_index(idx);
            g.store.set_profile(p);
            g.status = format!("Profile: {p}");
            let _ = g.store.save();
        });
    }
    {
        let shared_ref = shared.clone();
        let ui_weak = ui.as_weak();
        ui.on_refresh_targets(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let g = shared_ref.lock().unwrap();
                ui.set_targets_text(format_targets(&g.store.config.paired_targets).into());
            }
        });
    }

    let auto = shared.lock().unwrap().store.config.advertise_on_start;
    if auto {
        let _ = cmd_tx.send(UiCommand::StartAdvertise);
    }

    let shared_bg = shared.clone();
    let ui_weak = ui.as_weak();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio");
        rt.block_on(async move {
            loop {
                tokio::select! {
                    Some(cmd) = cmd_rx.recv() => {
                        match cmd {
                            UiCommand::StartAdvertise => {
                                if let Err(e) = ensure_advertising(&shared_bg).await {
                                    error!("{e}");
                                }
                            }
                            UiCommand::StopAdvertise => {
                                stop_advertising(&shared_bg).await;
                            }
                        }
                        push_ui(&shared_bg, &ui_weak, None);
                    }
                    ev = input_rx.recv() => {
                        match ev {
                            Some(InputEvent::State(state)) => {
                                pump_reports(&shared_bg, &state).await;
                                drain_bt_events(&shared_bg).await;
                                push_ui(&shared_bg, &ui_weak, Some(state.battery_pct));
                            }
                            Some(InputEvent::Error(e)) => warn!("input: {e}"),
                            None => break,
                        }
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                        drain_bt_events(&shared_bg).await;
                        push_ui(&shared_bg, &ui_weak, None);
                    }
                }
            }
        });
    });

    ui.run().context("UI run")?;
    // Best-effort stop after UI closes
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(stop_advertising(&shared));
    Ok(())
}

enum UiCommand {
    StartAdvertise,
    StopAdvertise,
}

fn push_ui(
    shared: &Arc<Mutex<Shared>>,
    ui_weak: &slint::Weak<MainWindow>,
    battery: Option<u8>,
) {
    let snapshot = {
        let g = shared.lock().unwrap();
        (
            g.status.clone(),
            g.advertising,
            g.connected,
            g.peer_name.clone(),
            battery.unwrap_or(100),
            format_targets(&g.store.config.paired_targets),
            index_from_profile(g.store.config.active_profile),
        )
    };
    let ui_weak = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_status_text(snapshot.0.into());
            ui.set_advertising(snapshot.1);
            ui.set_connected(snapshot.2);
            ui.set_peer_name(snapshot.3.into());
            ui.set_battery_pct(snapshot.4 as i32);
            ui.set_targets_text(snapshot.5.into());
            ui.set_selected_profile(snapshot.6);
        }
    });
}
