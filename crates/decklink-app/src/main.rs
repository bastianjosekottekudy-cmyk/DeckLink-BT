//! DeckLink application — input → profiles → Wi-Fi UDP + Slint UI.

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::Parser;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use decklink_hid::{
    hid_from_char, idle_release_packets, KeyModifiers, KeyboardReport, MouseButtons, MouseReport,
    HidPacket, GamepadButtons, MOUSE_REPORT_ID,
};
use decklink_input::{spawn_input_task, InputCommand, InputEvent};
use decklink_net::NetClient;
use decklink_profiles::{map_state, PairedTarget, Profile, ProfileStore};
use decklink_ui::{format_targets, index_from_profile, profile_from_index, MainWindow};
use slint::ComponentHandle;

#[derive(Parser, Debug)]
#[command(
    name = "decklink-bt",
    version,
    about = "Steam Deck as a Wi-Fi gamepad / keyboard+mouse"
)]
struct Cli {
    /// Start without opening the Slint window
    #[arg(long)]
    headless: bool,

    /// Connect immediately (needs --host or saved host_addr)
    #[arg(long)]
    connect: bool,

    /// PC host IP or ip:port (default port 31415)
    #[arg(long)]
    host: Option<String>,

    /// Override saved profile: gamepad | desktop
    #[arg(long)]
    profile: Option<String>,

    /// Device name sent in Hello
    #[arg(long)]
    name: Option<String>,

    /// Verbose diagnostics → ~/.local/share/decklink-bt/decklink.log
    #[arg(long)]
    diag: bool,
}

struct Shared {
    store: ProfileStore,
    link: Option<NetClient>,
    connecting: bool,
    /// Session armed (connecting or connected).
    linking: bool,
    connected: bool,
    peer_name: String,
    status: String,
    sticky_mods: u8,
    soft_mouse_buttons: u8,
    type_sent_len: usize,
    profile_chord_held: bool,
    input_cmd: Option<mpsc::Sender<InputCommand>>,
    last_heartbeat: std::time::Instant,
}

fn sync_input_grab(shared: &Arc<Mutex<Shared>>) {
    let g = shared.lock().unwrap();
    let want = g.linking || g.connected;
    let freeze = g.connected;
    if let Some(tx) = &g.input_cmd {
        let _ = tx.try_send(InputCommand::SetExclusive(want));
        let _ = tx.try_send(InputCommand::SetSteamFrozen(freeze));
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.diag {
        std::env::set_var("DECKLINK_DIAG", "1");
    }
    let diag = matches!(
        std::env::var("DECKLINK_DIAG").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    );

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        if diag {
            "decklink_app=info,decklink_net=info,decklink_input=info,decklink_profiles=info".into()
        } else {
            "decklink_app=info,decklink_net=info,decklink_input=info".into()
        }
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

    if diag {
        info!("DIAG capture ON — log: ~/.local/share/decklink-bt/decklink.log");
    }

    let _instance_lock = acquire_instance_lock().context(
        "DeckLink is already running — close the other window/shortcut and try again",
    )?;

    if !cli.headless {
        if std::env::var_os("WINIT_UNIX_BACKEND").is_none() {
            std::env::set_var("WINIT_UNIX_BACKEND", "x11");
        }
        if std::env::var_os("SLINT_BACKEND").is_none() {
            std::env::set_var("SLINT_BACKEND", "winit");
        }
    }

    let mut store = ProfileStore::load().context("load config")?;
    if let Some(ref s) = cli.profile {
        if let Some(p) = Profile::parse(s) {
            store.set_profile(p);
        }
    }
    if let Some(name) = &cli.name {
        store.config.device_name = name.clone();
    }
    if let Some(host) = &cli.host {
        store.config.host_addr = host.clone();
    }
    if cli.connect {
        store.config.connect_on_start = true;
    }
    let _ = store.save();

    let (input_tx, mut input_rx) = mpsc::channel::<InputEvent>(256);
    let (input_cmd_tx, input_cmd_rx) = mpsc::channel::<InputCommand>(8);
    if let Err(e) = spawn_input_task(input_tx, input_cmd_rx).await {
        warn!("input capture: {e} (continuing with idle state)");
    }

    let shared = Arc::new(Mutex::new(Shared {
        store,
        link: None,
        connecting: false,
        linking: false,
        connected: false,
        peer_name: "—".into(),
        status: format!(
            "Ready v{} — enter PC IP, Connect. PC needs decklink-host + ViGEmBus.",
            env!("CARGO_PKG_VERSION")
        ),
        sticky_mods: 0,
        soft_mouse_buttons: 0,
        type_sent_len: 0,
        profile_chord_held: false,
        input_cmd: Some(input_cmd_tx),
        last_heartbeat: std::time::Instant::now(),
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
                g.store.config.connect_on_start = true;
                g.status = format!("UI failed ({e}); headless connect");
            }
            let (input_tx, mut input_rx2) = mpsc::channel::<InputEvent>(256);
            let (cmd_tx, cmd_rx) = mpsc::channel::<InputCommand>(8);
            {
                let mut g = shared.lock().unwrap();
                g.input_cmd = Some(cmd_tx);
            }
            let _ = spawn_input_task(input_tx, cmd_rx).await;
            run_headless(shared, &mut input_rx2).await
        }
    }
}

async fn ensure_connected(shared: &Arc<Mutex<Shared>>) -> Result<()> {
    let (host, name) = {
        let mut g = shared.lock().unwrap();
        if g.link.is_some() || g.connecting {
            return Ok(());
        }
        let host = g.store.config.host_addr.trim().to_string();
        if host.is_empty() {
            g.status = "Set PC IP first (e.g. 192.168.1.20)".into();
            anyhow::bail!("no host_addr");
        }
        g.connecting = true;
        g.linking = true;
        g.status = format!("Connecting to {host}…");
        (host, g.store.config.device_name.clone())
    };
    sync_input_grab(shared);

    // Blocking UDP hello on a worker so we don't freeze the async runtime long.
    let result = tokio::task::spawn_blocking(move || NetClient::connect(&host, &name)).await?;

    match result {
        Ok(client) => {
            let peer = client.peer_addr().to_string();
            let peer_name = client.peer_name.clone();
            {
                let mut g = shared.lock().unwrap();
                g.connecting = false;
                g.linking = true;
                g.connected = true;
                g.peer_name = peer_name.clone();
                g.status = format!("Linked to {peer_name} ({peer}) — Steam frozen");
                g.link = Some(client);
                g.last_heartbeat = std::time::Instant::now();
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs().to_string())
                    .unwrap_or_default();
                g.store.upsert_target(PairedTarget {
                    name: peer_name,
                    address: peer,
                    last_connected: Some(now),
                });
                let _ = g.store.save();
            }
            sync_input_grab(shared);
            info!("Wi-Fi linked");
            Ok(())
        }
        Err(e) => {
            let mut g = shared.lock().unwrap();
            g.connecting = false;
            g.linking = false;
            g.connected = false;
            g.status = format!("Connect failed: {e}");
            drop(g);
            sync_input_grab(shared);
            Err(e.into())
        }
    }
}

fn acquire_instance_lock() -> Result<std::fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let dir = dirs::data_local_dir()
            .ok_or_else(|| anyhow::anyhow!("no data dir"))?
            .join("decklink-bt");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("decklink.lock");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open(&path)?;
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            anyhow::bail!("lock busy");
        }
        return Ok(file);
    }
    #[cfg(not(unix))]
    {
        let dir = dirs::data_local_dir()
            .ok_or_else(|| anyhow::anyhow!("no data dir"))?
            .join("decklink-bt");
        std::fs::create_dir_all(&dir)?;
        Ok(std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open(dir.join("decklink.lock"))?)
    }
}

async fn stop_link(shared: &Arc<Mutex<Shared>>) {
    {
        let mut g = shared.lock().unwrap();
        if let Some(mut link) = g.link.take() {
            let _ = link.send_goodbye();
        }
        g.linking = false;
        g.connected = false;
        g.connecting = false;
        g.status = "Disconnected — Deck sticks/trackpads returned to Desktop".into();
    }
    sync_input_grab(shared);
}

async fn run_headless(
    shared: Arc<Mutex<Shared>>,
    input_rx: &mut mpsc::Receiver<InputEvent>,
) -> Result<()> {
    info!("headless mode");
    let auto = shared.lock().unwrap().store.config.connect_on_start;
    if auto {
        if let Err(e) = ensure_connected(&shared).await {
            error!("{e}");
        }
    }

    loop {
        tokio::select! {
            ev = input_rx.recv() => {
                match ev {
                    Some(InputEvent::State(state)) => {
                        pump_reports(&shared, &state).await;
                        poll_link(&shared);
                    }
                    Some(InputEvent::Error(e)) => {
                        warn!("input: {e}");
                        shared.lock().unwrap().status = e;
                    }
                    None => break,
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {
                poll_link(&shared);
            }
        }
    }
    Ok(())
}

fn poll_link(shared: &Arc<Mutex<Shared>>) {
    let mut drop_link = false;
    {
        let mut g = shared.lock().unwrap();
        let need_hb = g.last_heartbeat.elapsed() > std::time::Duration::from_millis(500);
        if let Some(link) = g.link.as_mut() {
            if need_hb {
                let _ = link.send_heartbeat();
            }
            match link.poll() {
                Ok(true) => {}
                Ok(false) => {
                    g.status = "Host disconnected".into();
                    drop_link = true;
                }
                Err(e) => {
                    g.status = format!("Link error: {e}");
                    drop_link = true;
                }
            }
        }
        if need_hb && g.link.is_some() {
            g.last_heartbeat = std::time::Instant::now();
        }
        if drop_link {
            let _ = g.link.take();
            g.connected = false;
            g.linking = false;
        }
    }
    if drop_link {
        sync_input_grab(shared);
    }
}

async fn pump_reports(shared: &Arc<Mutex<Shared>>, state: &decklink_hid::ControllerState) {
    let _ = maybe_toggle_profile_chord(shared, state).await;
    let packets = {
        let g = shared.lock().unwrap();
        let profile = g.store.config.active_profile;
        map_state(profile, state).packets
    };
    let mut g = shared.lock().unwrap();
    if let Some(link) = g.link.as_mut() {
        for pkt in packets {
            if let Err(e) = link.send_hid(&pkt) {
                warn!("send_hid: {e}");
                g.status = format!("Send failed: {e}");
                let _ = g.link.take();
                g.connected = false;
                g.linking = false;
                drop(g);
                sync_input_grab(shared);
                return;
            }
        }
    }
}

async fn maybe_toggle_profile_chord(
    shared: &Arc<Mutex<Shared>>,
    state: &decklink_hid::ControllerState,
) -> bool {
    let chord = state.buttons.contains(GamepadButtons::SELECT)
        && state.buttons.contains(GamepadButtons::START);
    let should_switch = {
        let mut g = shared.lock().unwrap();
        if chord && !g.profile_chord_held {
            g.profile_chord_held = true;
            true
        } else {
            if !chord {
                g.profile_chord_held = false;
            }
            false
        }
    };
    if !should_switch {
        return false;
    }
    apply_profile_switch(shared, None).await;
    true
}

async fn apply_profile_switch(shared: &Arc<Mutex<Shared>>, to: Option<Profile>) {
    let next = {
        let mut g = shared.lock().unwrap();
        let next = to.unwrap_or_else(|| match g.store.config.active_profile {
            Profile::Gamepad => Profile::Desktop,
            Profile::Desktop => Profile::Gamepad,
        });
        g.store.set_profile(next);
        g.sticky_mods = 0;
        g.soft_mouse_buttons = 0;
        g.type_sent_len = 0;
        g.status = format!("Profile: {next} — Select+Start toggles");
        let _ = g.store.save();
        next
    };
    info!("profile switch → {next}");
    for pkt in idle_release_packets() {
        link_send(shared, pkt);
    }
}

fn run_ui(shared: Arc<Mutex<Shared>>, mut input_rx: mpsc::Receiver<InputEvent>) -> Result<()> {
    let ui = MainWindow::new().context("create Slint window")?;

    {
        let g = shared.lock().unwrap();
        ui.set_selected_profile(index_from_profile(g.store.config.active_profile));
        ui.set_targets_text(format_targets(&g.store.config.paired_targets).into());
        ui.set_status_text(g.status.clone().into());
        ui.set_host_addr(g.store.config.host_addr.clone().into());
        ui.set_battery_pct(100);
        ui.set_sticky_mods(g.sticky_mods as i32);
    }

    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<UiCommand>();

    {
        let tx = cmd_tx.clone();
        ui.on_start_connect(move || {
            let _ = tx.send(UiCommand::Connect);
        });
    }
    {
        let tx = cmd_tx.clone();
        ui.on_stop_connect(move || {
            let _ = tx.send(UiCommand::Disconnect);
        });
    }
    {
        let tx = cmd_tx.clone();
        ui.on_host_addr_edited(move |s| {
            let _ = tx.send(UiCommand::SetHost(s.to_string()));
        });
    }
    {
        let tx = cmd_tx.clone();
        ui.on_profile_changed(move |idx| {
            let _ = tx.send(UiCommand::SetProfile(idx));
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

    let auto = shared.lock().unwrap().store.config.connect_on_start;
    if auto {
        let _ = cmd_tx.send(UiCommand::Connect);
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
                            UiCommand::Connect => {
                                if let Err(e) = ensure_connected(&shared_bg).await {
                                    error!("{e}");
                                }
                            }
                            UiCommand::Disconnect => {
                                stop_link(&shared_bg).await;
                            }
                            UiCommand::SetHost(s) => {
                                let mut g = shared_bg.lock().unwrap();
                                g.store.config.host_addr = s;
                                let _ = g.store.save();
                            }
                            UiCommand::SetProfile(idx) => {
                                apply_profile_switch(&shared_bg, Some(profile_from_index(idx)))
                                    .await;
                            }
                            UiCommand::KeyTap(code) => {
                                soft_key_tap(&shared_bg, code);
                            }
                            UiCommand::ModToggle(mask) => {
                                soft_mod_toggle(&shared_bg, mask);
                            }
                            UiCommand::MouseDelta(dx, dy) => {
                                soft_mouse_move(&shared_bg, dx, dy);
                            }
                            UiCommand::MouseButton(mask, down) => {
                                soft_mouse_button(&shared_bg, mask, down);
                            }
                            UiCommand::TypeBuffer(s) => {
                                soft_type_buffer(&shared_bg, &s);
                            }
                            UiCommand::ClearMods => {
                                soft_clear_mods(&shared_bg);
                            }
                        }
                        push_ui(&shared_bg, &ui_weak, None);
                    }
                    ev = input_rx.recv() => {
                        match ev {
                            Some(InputEvent::State(state)) => {
                                pump_reports(&shared_bg, &state).await;
                                poll_link(&shared_bg);
                                push_ui(&shared_bg, &ui_weak, Some(state.battery_pct));
                            }
                            Some(InputEvent::Error(e)) => {
                                warn!("input: {e}");
                                shared_bg.lock().unwrap().status = e;
                                push_ui(&shared_bg, &ui_weak, None);
                            }
                            None => break,
                        }
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                        poll_link(&shared_bg);
                        push_ui(&shared_bg, &ui_weak, None);
                    }
                }
            }
        });
    });

    ui.run().context("UI run")?;
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(stop_link(&shared));
    Ok(())
}

enum UiCommand {
    Connect,
    Disconnect,
    SetHost(String),
    SetProfile(i32),
    KeyTap(u8),
    ModToggle(u8),
    MouseDelta(f32, f32),
    MouseButton(u8, bool),
    TypeBuffer(String),
    ClearMods,
}

fn link_send(shared: &Arc<Mutex<Shared>>, pkt: HidPacket) {
    let mut g = shared.lock().unwrap();
    if let Some(link) = g.link.as_mut() {
        if let Err(e) = link.send_hid(&pkt) {
            warn!("send: {e}");
        }
    }
}

fn soft_key_tap(shared: &Arc<Mutex<Shared>>, code: u8) {
    let mods = {
        let mut g = shared.lock().unwrap();
        let m = g.sticky_mods;
        g.sticky_mods = 0;
        KeyModifiers::from_bits_truncate(m)
    };
    for pkt in KeyboardReport::tap_packets(mods, code) {
        link_send(shared, pkt);
    }
}

fn soft_mod_toggle(shared: &Arc<Mutex<Shared>>, mask: u8) {
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
    link_send(shared, KeyboardReport::packet(mods, [0; 6]));
}

fn soft_clear_mods(shared: &Arc<Mutex<Shared>>) {
    {
        let mut g = shared.lock().unwrap();
        g.sticky_mods = 0;
    }
    link_send(
        shared,
        KeyboardReport::packet(KeyModifiers::empty(), [0; 6]),
    );
}

fn soft_mouse_move(shared: &Arc<Mutex<Shared>>, dx: f32, dy: f32) {
    if !dx.is_finite() || !dy.is_finite() || dx.abs() > 24.0 || dy.abs() > 24.0 {
        return;
    }
    let buttons = {
        let g = shared.lock().unwrap();
        g.soft_mouse_buttons
    };
    let sx = dx.round().clamp(-12.0, 12.0) as i8;
    let sy = dy.round().clamp(-12.0, 12.0) as i8;
    if sx == 0 && sy == 0 {
        return;
    }
    let r = MouseReport {
        buttons: MouseButtons::from_bits_truncate(buttons),
        dx: sx,
        dy: sy,
        wheel: 0,
    };
    link_send(
        shared,
        HidPacket {
            report_id: MOUSE_REPORT_ID,
            data: r.pack().to_vec(),
        },
    );
}

fn soft_mouse_button(shared: &Arc<Mutex<Shared>>, mask: u8, down: bool) {
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
    link_send(
        shared,
        HidPacket {
            report_id: MOUSE_REPORT_ID,
            data: r.pack().to_vec(),
        },
    );
}

fn soft_type_buffer(shared: &Arc<Mutex<Shared>>, s: &str) {
    let (to_send, backs) = {
        let mut g = shared.lock().unwrap();
        let prev = g.type_sent_len;
        let chars: Vec<char> = s.chars().collect();
        if chars.len() < prev {
            let backs = prev - chars.len();
            g.type_sent_len = chars.len();
            (Vec::new(), backs)
        } else {
            let extra: String = chars[prev..].iter().collect();
            g.type_sent_len = chars.len();
            (extra.chars().collect::<Vec<_>>(), 0)
        }
    };
    for _ in 0..backs {
        for pkt in KeyboardReport::tap_packets(KeyModifiers::empty(), decklink_hid::hid_key::BACKSPACE)
        {
            link_send(shared, pkt);
        }
    }
    for ch in to_send {
        if let Some((code, extra)) = hid_from_char(ch) {
            for pkt in KeyboardReport::tap_packets(extra, code) {
                link_send(shared, pkt);
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
            g.linking,
            g.connected,
            g.peer_name.clone(),
            battery.unwrap_or(100),
            format_targets(&g.store.config.paired_targets),
            index_from_profile(g.store.config.active_profile),
            g.sticky_mods as i32,
            g.store.config.host_addr.clone(),
        )
    };
    let ui_weak = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_status_text(snapshot.0.into());
            ui.set_linking(snapshot.1);
            ui.set_connected(snapshot.2);
            ui.set_peer_name(snapshot.3.into());
            ui.set_battery_pct(snapshot.4 as i32);
            ui.set_targets_text(snapshot.5.into());
            ui.set_selected_profile(snapshot.6);
            ui.set_sticky_mods(snapshot.7);
            ui.set_host_addr(snapshot.8.into());
        }
    });
}
