//! DeckLink BT application entry — wires input → profiles → HOGP + Slint UI.

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::Parser;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use decklink_bt::{start_hogp, BtEvent, HogpServer};
use decklink_hid::{
    hid_from_char, KeyModifiers, KeyboardReport, MouseButtons, MouseReport, HidPacket,
    KEYBOARD_REPORT_ID, MOUSE_REPORT_ID,
};
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
    /// Start without opening the Slint window (needed under Gaming Mode / gamescope)
    #[arg(long)]
    headless: bool,

    /// Begin advertising immediately
    #[arg(long)]
    advertise: bool,

    /// Gaming Mode preset: implies --headless --advertise
    #[arg(long)]
    gaming: bool,

    /// Profile: gamepad (Xbox) | desktop (keyboard+mouse)
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
    /// TapBoard-style sticky modifiers for soft keyboard (cleared after one key).
    sticky_mods: u8,
    soft_mouse_buttons: u8,
    type_sent_len: usize,
}

#[tokio::main]
async fn main() -> Result<()> {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        "decklink_bt=info,decklink_app=info,decklink_input=info".into()
    });
    if let Some(dir) = dirs::data_local_dir() {
        let log_dir = dir.join("decklink-bt");
        let _ = std::fs::create_dir_all(&log_dir);
        if let Ok(file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_dir.join("decklink.log"))
        {
            use tracing_subscriber::prelude::*;
            let _ = tracing_subscriber::registry()
                .with(env_filter)
                .with(tracing_subscriber::fmt::layer())
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_ansi(false)
                        .with_writer(std::sync::Mutex::new(file)),
                )
                .try_init();
        } else {
            tracing_subscriber::fmt().with_env_filter(env_filter).init();
        }
    } else {
        tracing_subscriber::fmt().with_env_filter(env_filter).init();
    }

    let mut cli = Cli::parse();
    // Only force headless when explicitly requested — do not treat Desktop Plasma as Gaming Mode.
    let gaming_env = std::env::var("DECKLINK_GAMING_MODE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if cli.gaming || gaming_env {
        cli.headless = true;
        cli.advertise = true;
        info!("Gaming Mode profile: headless + advertise");
    }

    if !cli.headless {
        if std::env::var_os("WINIT_UNIX_BACKEND").is_none() {
            std::env::set_var("WINIT_UNIX_BACKEND", "x11");
        }
        if std::env::var_os("SLINT_BACKEND").is_none() {
            std::env::set_var("SLINT_BACKEND", "winit");
        }
    }

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
        sticky_mods: 0,
        soft_mouse_buttons: 0,
        type_sent_len: 0,
    }));

    if cli.headless {
        return run_headless(shared, &mut input_rx).await;
    }

    match run_ui(shared.clone(), input_rx) {
        Ok(()) => Ok(()),
        Err(e) => {
            error!("UI failed: {e}");
            {
                let mut g = shared.lock().unwrap();
                g.store.config.advertise_on_start = true;
                g.status = format!("UI failed ({e}); headless advertising");
            }
            let (input_tx, mut input_rx2) = mpsc::channel::<InputEvent>(256);
            let _ = spawn_input_task(input_tx).await;
            run_headless(shared, &mut input_rx2).await
        }
    }
}

async fn ensure_advertising(shared: &Arc<Mutex<Shared>>) -> Result<()> {
    {
        let g = shared.lock().unwrap();
        if g.hogp.is_some() {
            return Ok(());
        }
    }
    let name = { shared.lock().unwrap().store.config.device_name.clone() };

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
        ui.set_sticky_mods(g.sticky_mods as i32);
    }

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
    {
        let tx = cmd_tx.clone();
        ui.on_key_tap(move |code| {
            let _ = tx.send(UiCommand::KeyTap(code as u8));
        });
    }
    {
        let tx = cmd_tx.clone();
        ui.on_mod_toggle(move |mask| {
            let _ = tx.send(UiCommand::ModToggle(mask as u8));
        });
    }
    {
        let tx = cmd_tx.clone();
        ui.on_mouse_delta(move |dx, dy| {
            let _ = tx.send(UiCommand::MouseDelta(dx, dy));
        });
    }
    {
        let tx = cmd_tx.clone();
        ui.on_mouse_button(move |mask, down| {
            let _ = tx.send(UiCommand::MouseButton(mask as u8, down));
        });
    }
    {
        let tx = cmd_tx.clone();
        ui.on_type_char(move |s| {
            let _ = tx.send(UiCommand::TypeBuffer(s.to_string()));
        });
    }
    {
        let tx = cmd_tx.clone();
        ui.on_clear_mods(move || {
            let _ = tx.send(UiCommand::ClearMods);
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
                            UiCommand::KeyTap(code) => {
                                soft_key_tap(&shared_bg, code).await;
                            }
                            UiCommand::ModToggle(mask) => {
                                soft_mod_toggle(&shared_bg, mask).await;
                            }
                            UiCommand::MouseDelta(dx, dy) => {
                                soft_mouse_move(&shared_bg, dx, dy).await;
                            }
                            UiCommand::MouseButton(mask, down) => {
                                soft_mouse_button(&shared_bg, mask, down).await;
                            }
                            UiCommand::TypeBuffer(s) => {
                                soft_type_buffer(&shared_bg, &s).await;
                            }
                            UiCommand::ClearMods => {
                                soft_clear_mods(&shared_bg).await;
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
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(stop_advertising(&shared));
    Ok(())
}

enum UiCommand {
    StartAdvertise,
    StopAdvertise,
    KeyTap(u8),
    ModToggle(u8),
    MouseDelta(f32, f32),
    MouseButton(u8, bool),
    TypeBuffer(String),
    ClearMods,
}

async fn hogp_send(shared: &Arc<Mutex<Shared>>, pkt: HidPacket) {
    let tx = shared.lock().unwrap().hogp.as_ref().map(|h| h.report_tx.clone());
    if let Some(tx) = tx {
        let _ = tx.send(pkt).await;
    }
}

async fn soft_key_tap(shared: &Arc<Mutex<Shared>>, code: u8) {
    let mods = {
        let mut g = shared.lock().unwrap();
        let m = g.sticky_mods;
        g.sticky_mods = 0;
        KeyModifiers::from_bits_truncate(m)
    };
    for pkt in KeyboardReport::tap_packets(mods, code) {
        hogp_send(shared, pkt).await;
    }
}

async fn soft_mod_toggle(shared: &Arc<Mutex<Shared>>, mask: u8) {
    {
        let mut g = shared.lock().unwrap();
        if g.sticky_mods & mask != 0 {
            g.sticky_mods &= !mask;
        } else {
            g.sticky_mods |= mask;
        }
    }
    let mods = {
        let g = shared.lock().unwrap();
        KeyModifiers::from_bits_truncate(g.sticky_mods)
    };
    hogp_send(shared, KeyboardReport::packet(mods, [0; 6])).await;
}

async fn soft_clear_mods(shared: &Arc<Mutex<Shared>>) {
    {
        let mut g = shared.lock().unwrap();
        g.sticky_mods = 0;
    }
    hogp_send(
        shared,
        KeyboardReport::packet(KeyModifiers::empty(), [0; 6]),
    )
    .await;
}

async fn soft_mouse_move(shared: &Arc<Mutex<Shared>>, dx: f32, dy: f32) {
    let buttons = {
        let g = shared.lock().unwrap();
        g.soft_mouse_buttons
    };
    let mut rem_x = dx.round() as i32;
    let mut rem_y = dy.round() as i32;
    if rem_x == 0 && rem_y == 0 && (dx != 0.0 || dy != 0.0) {
        rem_x = if dx < 0.0 { -1 } else { 1 };
        rem_y = if dy < 0.0 {
            -1
        } else if dy > 0.0 {
            1
        } else {
            0
        };
        if dx == 0.0 {
            rem_x = 0;
        }
    }
    while rem_x != 0 || rem_y != 0 {
        let sx = rem_x.clamp(-127, 127) as i8;
        let sy = rem_y.clamp(-127, 127) as i8;
        rem_x -= sx as i32;
        rem_y -= sy as i32;
        let r = MouseReport {
            buttons: MouseButtons::from_bits_truncate(buttons),
            dx: sx,
            dy: sy,
            wheel: 0,
        };
        hogp_send(
            shared,
            HidPacket {
                report_id: MOUSE_REPORT_ID,
                data: r.pack().to_vec(),
            },
        )
        .await;
    }
}

async fn soft_mouse_button(shared: &Arc<Mutex<Shared>>, mask: u8, down: bool) {
    let buttons = {
        let mut g = shared.lock().unwrap();
        if down {
            g.soft_mouse_buttons |= mask;
        } else {
            g.soft_mouse_buttons &= !mask;
        }
        g.soft_mouse_buttons
    };
    let r = MouseReport {
        buttons: MouseButtons::from_bits_truncate(buttons),
        dx: 0,
        dy: 0,
        wheel: 0,
    };
    hogp_send(
        shared,
        HidPacket {
            report_id: MOUSE_REPORT_ID,
            data: r.pack().to_vec(),
        },
    )
    .await;
}

async fn soft_type_buffer(shared: &Arc<Mutex<Shared>>, s: &str) {
    let (prev_len, to_send, backs) = {
        let mut g = shared.lock().unwrap();
        let prev = g.type_sent_len;
        let chars: Vec<char> = s.chars().collect();
        if chars.len() < prev {
            let backs = prev - chars.len();
            g.type_sent_len = chars.len();
            (prev, Vec::new(), backs)
        } else {
            let extra: String = chars[prev..].iter().collect();
            g.type_sent_len = chars.len();
            (prev, extra.chars().collect::<Vec<_>>(), 0)
        }
    };
    let _ = prev_len;
    for _ in 0..backs {
        for pkt in KeyboardReport::tap_packets(KeyModifiers::empty(), decklink_hid::hid_key::BACKSPACE)
        {
            hogp_send(shared, pkt).await;
        }
    }
    for ch in to_send {
        if let Some((code, extra)) = hid_from_char(ch) {
            for pkt in KeyboardReport::tap_packets(extra, code) {
                hogp_send(shared, pkt).await;
            }
        }
    }
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
            g.sticky_mods as i32,
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
            ui.set_sticky_mods(snapshot.7);
        }
    });
}
