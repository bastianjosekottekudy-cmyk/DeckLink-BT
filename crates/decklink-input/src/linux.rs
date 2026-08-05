//! Linux input: prefer Valve hidraw Deck reports; grab local pointer devices while BLE is active.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use evdev::{AbsoluteAxisCode, Device, EventSummary, KeyCode, RelativeAxisCode};
use tokio::sync::mpsc;
use tracing::{info, warn};

use decklink_hid::{ControllerState, GamepadButtons};

use crate::hidraw_deck::{self, HidrawDeck};
use crate::{read_battery_percent, InputCommand, InputError, InputEvent};

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeviceRole {
    Gamepad,
    Trackpad,
    /// Swallow only — Steam/lizard/uinput pointer must not reach Desktop or host HID.
    LocalPointer,
}

fn list_devices() -> Vec<(PathBuf, String)> {
    evdev::enumerate()
        .map(|(path, device)| {
            let name = device.name().unwrap_or("").to_string();
            (path, name)
        })
        .collect()
}

fn score_gamepad(name: &str) -> i32 {
    let lower = name.to_ascii_lowercase();
    if lower.contains("mouse")
        || lower.contains("keyboard")
        || lower.contains("consumer")
        || lower.contains("power")
        || lower.contains("lid")
        || lower.contains("video bus")
        || lower.contains("gyro")
        || lower.contains("accel")
        || lower.contains("imu")
        || lower.contains("touchpad")
        || lower.contains("trackpad")
    {
        return -100;
    }
    let mut score = 0;
    if lower.contains("steam deck") {
        score += 100;
    }
    if lower.contains("x-box") || lower.contains("xbox") || lower.contains("360 pad") {
        score += 90;
    }
    if lower.contains("steam") && lower.contains("controller") {
        score += 70;
    }
    if lower.contains("joystick") || lower.contains("gamepad") || lower.contains("handheld") {
        score += 40;
    }
    score
}

fn score_trackpad(name: &str) -> i32 {
    let lower = name.to_ascii_lowercase();
    if lower.contains("touchpad") || lower.contains("trackpad") {
        return 80;
    }
    if lower.contains("steam") && lower.contains("pad") && !lower.contains("mouse") {
        return 50;
    }
    -100
}

fn score_local_pointer(name: &str) -> i32 {
    let lower = name.to_ascii_lowercase();
    if lower.contains("decklink")
        || lower.contains("power")
        || lower.contains("lid")
        || lower.contains("video bus")
        || lower.contains("hdmi")
        || lower.contains("intel")
        || lower.contains("sof-")
        || lower.contains("hda")
    {
        return -100;
    }
    if lower.contains("mouse")
        || lower.contains("touchpad")
        || lower.contains("trackpad")
        || lower.contains("extest")
        || lower.contains("xisible")
        || (lower.contains("uinput")
            && (lower.contains("mouse") || lower.contains("pointer") || lower.contains("event")))
    {
        return 100;
    }
    if (lower.contains("keyboard") || lower.contains("consumer"))
        && (lower.contains("steam")
            || lower.contains("valve")
            || lower.contains("deck")
            || lower.contains("jupiter"))
    {
        return 90;
    }
    if lower.contains("steam") && lower.contains("mouse") {
        return 95;
    }
    -100
}

fn device_has_rel_x(dev: &Device) -> bool {
    dev.supported_relative_axes()
        .is_some_and(|a| a.contains(RelativeAxisCode::REL_X))
}

/// Anything Steam/Deck uses to drive the *local* cursor — grab while advertising.
fn should_silence(name: &str, dev: Option<&Device>) -> bool {
    if score_local_pointer(name) > 0 || score_gamepad(name) > 0 || score_trackpad(name) > 0 {
        return true;
    }
    // Catch unnamed uinput / libei / Steam virtual pointers.
    if let Some(d) = dev {
        if device_has_rel_x(d) {
            let lower = name.to_ascii_lowercase();
            if !lower.contains("decklink") {
                return true;
            }
        }
    }
    false
}

fn apply_key(state: &mut ControllerState, code: KeyCode, pressed: bool) {
    let flag = match code {
        KeyCode::BTN_SOUTH => Some(GamepadButtons::A),
        KeyCode::BTN_EAST => Some(GamepadButtons::B),
        KeyCode::BTN_WEST => Some(GamepadButtons::X),
        KeyCode::BTN_NORTH => Some(GamepadButtons::Y),
        KeyCode::BTN_TL => Some(GamepadButtons::L1),
        KeyCode::BTN_TR => Some(GamepadButtons::R1),
        KeyCode::BTN_SELECT => Some(GamepadButtons::SELECT),
        KeyCode::BTN_START => Some(GamepadButtons::START),
        KeyCode::BTN_THUMBL => Some(GamepadButtons::L3),
        KeyCode::BTN_THUMBR => Some(GamepadButtons::R3),
        KeyCode::BTN_MODE => Some(GamepadButtons::GUIDE),
        KeyCode::BTN_DPAD_UP => {
            state.dpad_up = pressed;
            None
        }
        KeyCode::BTN_DPAD_DOWN => {
            state.dpad_down = pressed;
            None
        }
        KeyCode::BTN_DPAD_LEFT => {
            state.dpad_left = pressed;
            None
        }
        KeyCode::BTN_DPAD_RIGHT => {
            state.dpad_right = pressed;
            None
        }
        KeyCode::BTN_TL2 => {
            if pressed {
                state.lt = 1.0;
            } else if state.lt >= 0.99 {
                state.lt = 0.0;
            }
            None
        }
        KeyCode::BTN_TR2 => {
            if pressed {
                state.rt = 1.0;
            } else if state.rt >= 0.99 {
                state.rt = 0.0;
            }
            None
        }
        KeyCode::BTN_TRIGGER_HAPPY1 => Some(GamepadButtons::L4),
        KeyCode::BTN_TRIGGER_HAPPY2 => Some(GamepadButtons::R4),
        KeyCode::BTN_TRIGGER_HAPPY3 => Some(GamepadButtons::L5),
        KeyCode::BTN_TRIGGER_HAPPY4 => Some(GamepadButtons::R5),
        KeyCode::BTN_LEFT | KeyCode::BTN_TOOL_FINGER | KeyCode::BTN_TOUCH => {
            if matches!(code, KeyCode::BTN_LEFT) {
                state.rpad_click = pressed;
            } else {
                state.rpad_touch = pressed;
            }
            None
        }
        _ => None,
    };
    if let Some(f) = flag {
        if pressed {
            state.buttons.insert(f);
        } else {
            state.buttons.remove(f);
        }
    }
}

fn norm_axis(value: i32, min: i32, max: i32) -> f32 {
    if max == min {
        return 0.0;
    }
    let mid = (max + min) as f32 / 2.0;
    let half = (max - min) as f32 / 2.0;
    ((value as f32 - mid) / half).clamp(-1.0, 1.0)
}

fn norm_trigger(value: i32, min: i32, max: i32) -> f32 {
    if max == min {
        return 0.0;
    }
    ((value - min) as f32 / (max - min) as f32).clamp(0.0, 1.0)
}

fn role_name(role: DeviceRole) -> &'static str {
    match role {
        DeviceRole::Gamepad => "gamepad",
        DeviceRole::Trackpad => "trackpad",
        DeviceRole::LocalPointer => "local-pointer",
    }
}

fn open_silence_devices() -> Vec<(PathBuf, String, Device)> {
    let mut out = Vec::new();
    let mut used = HashSet::new();
    for (path, name) in list_devices() {
        if !used.insert(path.clone()) {
            continue;
        }
        let Ok(d) = Device::open(&path) else {
            continue;
        };
        if !should_silence(&name, Some(&d)) {
            continue;
        }
        let _ = d.set_nonblocking(true);
        info!("silence-open {} ({})", path.display(), name);
        out.push((path, name, d));
    }
    out
}

/// Add any newly appeared pointer/gamepad nodes without releasing existing grabs.
fn merge_new_silence_devices(devs: &mut Vec<(PathBuf, String, Device)>) {
    let held: HashSet<PathBuf> = devs.iter().map(|(p, _, _)| p.clone()).collect();
    for (path, name) in list_devices() {
        if held.contains(&path) {
            continue;
        }
        let Ok(mut d) = Device::open(&path) else {
            continue;
        };
        if !should_silence(&name, Some(&d)) {
            continue;
        }
        let _ = d.set_nonblocking(true);
        match d.grab() {
            Ok(()) => info!("exclusive grab (new silence) ({name}) {}", path.display()),
            Err(e) => warn!("grab new silence ({name}) failed: {e}"),
        }
        info!("silence-open {} ({})", path.display(), name);
        devs.push((path, name, d));
    }
}

fn set_silence_exclusive(devs: &mut [(PathBuf, String, Device)], exclusive: bool, quiet: bool) {
    for (_path, name, dev) in devs.iter_mut() {
        if exclusive {
            match dev.grab() {
                Ok(()) => {
                    if !quiet {
                        info!("exclusive grab (silence) ({name})");
                    }
                }
                Err(e) => warn!("grab silence ({name}) failed: {e}"),
            }
        } else {
            match dev.ungrab() {
                Ok(()) => {
                    if !quiet {
                        info!("released silence grab ({name})");
                    }
                }
                Err(e) => warn!("ungrab silence ({name}) failed: {e}"),
            }
        }
    }
}

/// Drain silence devices so the kernel queue does not back up (events discarded).
fn drain_silence(devs: &mut [(PathBuf, String, Device)]) {
    for (_path, _name, dev) in devs.iter_mut() {
        match dev.fetch_events() {
            Ok(evs) => {
                for _ in evs {}
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => {}
        }
    }
}

pub async fn spawn_input_task(
    tx: mpsc::Sender<InputEvent>,
    cmd_rx: mpsc::Receiver<InputCommand>,
) -> Result<(), InputError> {
    // Prefer hidraw: owns Deck reports and fights lizard/Steam Desktop mouse at the source.
    if let Some(deck) = hidraw_deck::open() {
        info!("input backend: hidraw ({})", deck.path().display());
        tokio::spawn(async move {
            run_hidraw_loop(deck, tx, cmd_rx).await;
        });
        return Ok(());
    }

    warn!("hidraw Deck unavailable — falling back to evdev (Steam may still move local mouse)");
    spawn_evdev_fallback(tx, cmd_rx).await
}

async fn run_hidraw_loop(
    mut deck: HidrawDeck,
    tx: mpsc::Sender<InputEvent>,
    mut cmd_rx: mpsc::Receiver<InputCommand>,
) {
    let mut silence = open_silence_devices();
    let mut exclusive = false;
    let mut state = ControllerState::default();
    let mut tick: u64 = 0;

    loop {
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                InputCommand::SetExclusive(want) => {
                    if want != exclusive {
                        exclusive = want;
                        if exclusive {
                            silence = open_silence_devices();
                            set_silence_exclusive(&mut silence, true, false);
                            deck.feed_lizard();
                            hidraw_deck::set_kernel_lizard_mode(false);
                            info!(
                                "local pointer silence ON ({} devices) — grabs held",
                                silence.len()
                            );
                        } else {
                            set_silence_exclusive(&mut silence, false, false);
                            hidraw_deck::set_kernel_lizard_mode(true);
                            crate::steam_freeze::set_steam_frozen(false);
                            info!("local pointer silence OFF");
                        }
                    }
                }
                InputCommand::SetSteamFrozen(freeze) => {
                    crate::steam_freeze::set_steam_frozen(freeze);
                }
            }
        }

        state.clear_relative();
        let _got = deck.poll(&mut state);
        state.battery_pct = read_battery_percent();

        if exclusive {
            drain_silence(&mut silence);
            // Never ungrab while exclusive — that was letting Steam steal the cursor.
            // Only attach newly appeared pointer nodes and re-assert grab on held FDs.
            if tick > 0 && tick % 250 == 0 {
                merge_new_silence_devices(&mut silence);
                set_silence_exclusive(&mut silence, true, true);
                deck.feed_lizard();
            } else if tick % 50 == 0 {
                deck.feed_lizard();
            }
        }

        tick += 1;
        if tx.send(InputEvent::State(state.clone())).await.is_err() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(4)).await;
    }

    set_silence_exclusive(&mut silence, false, false);
    hidraw_deck::set_kernel_lizard_mode(true);
}

async fn spawn_evdev_fallback(
    tx: mpsc::Sender<InputEvent>,
    mut cmd_rx: mpsc::Receiver<InputCommand>,
) -> Result<(), InputError> {
    let devices = list_devices();
    for (path, name) in &devices {
        info!(
            "input candidate: {} ({}) pad={} track={} local={}",
            path.display(),
            name,
            score_gamepad(name),
            score_trackpad(name),
            score_local_pointer(name)
        );
    }

    let mut opened: Vec<(DeviceRole, String, Device)> = Vec::new();
    let mut used_paths: HashSet<PathBuf> = HashSet::new();

    let mut pads: Vec<_> = devices
        .iter()
        .cloned()
        .map(|(path, name)| (score_gamepad(&name), path, name))
        .filter(|(s, _, _)| *s > 0)
        .collect();
    pads.sort_by(|a, b| b.0.cmp(&a.0));
    for (_score, path, name) in pads.into_iter().take(5) {
        if !used_paths.insert(path.clone()) {
            continue;
        }
        match Device::open(&path) {
            Ok(d) => {
                let _ = d.set_nonblocking(true);
                opened.push((DeviceRole::Gamepad, name, d));
            }
            Err(e) => warn!("open gamepad: {e}"),
        }
    }

    for (path, name) in &devices {
        if used_paths.contains(path) || !should_silence(name, None) {
            continue;
        }
        if score_gamepad(name) > 0 {
            continue;
        }
        used_paths.insert(path.clone());
        match Device::open(path) {
            Ok(d) => {
                let _ = d.set_nonblocking(true);
                let role = if score_trackpad(name) > 0 {
                    DeviceRole::Trackpad
                } else {
                    DeviceRole::LocalPointer
                };
                info!("opened {} {} ({name})", role_name(role), path.display());
                opened.push((role, name.clone(), d));
            }
            Err(e) => warn!("open silence: {e}"),
        }
    }

    if opened.is_empty() {
        return Err(InputError::NoDevice);
    }

    let primary_gamepad_idx = opened
        .iter()
        .position(|(r, _, _)| *r == DeviceRole::Gamepad);

    tokio::spawn(async move {
        let mut state = ControllerState::default();
        let mut abs_min_max: HashMap<(usize, u16), (i32, i32)> = HashMap::new();
        let mut exclusive = false;
        let mut pad_last: HashMap<usize, (Option<i32>, Option<i32>)> = HashMap::new();

        for (idx, (_role, _name, dev)) in opened.iter().enumerate() {
            if let Ok(abs_state) = dev.get_abs_state() {
                for (i, info) in abs_state.iter().enumerate() {
                    if info.maximum != info.minimum {
                        abs_min_max.insert((idx, i as u16), (info.minimum, info.maximum));
                    }
                }
            }
        }

        loop {
            while let Ok(cmd) = cmd_rx.try_recv() {
                match cmd {
                    InputCommand::SetExclusive(want) => {
                        if want != exclusive {
                            exclusive = want;
                            for (role, name, dev) in opened.iter_mut() {
                                if exclusive {
                                    if let Err(e) = dev.grab() {
                                        warn!("grab {} ({name}): {e}", role_name(*role));
                                    } else {
                                        info!("exclusive grab on {} ({name})", role_name(*role));
                                    }
                                } else if let Err(e) = dev.ungrab() {
                                    warn!("ungrab {} ({name}): {e}", role_name(*role));
                                }
                            }
                            hidraw_deck::set_kernel_lizard_mode(!exclusive);
                            if exclusive {
                                let _ = crate::lizard::open_and_disable();
                            } else {
                                crate::steam_freeze::set_steam_frozen(false);
                            }
                        }
                    }
                    InputCommand::SetSteamFrozen(freeze) => {
                        crate::steam_freeze::set_steam_frozen(freeze);
                    }
                }
            }

            state.clear_relative();
            state.battery_pct = read_battery_percent();
            let mut pad_saw_rel = false;

            for (idx, (role, _name, dev)) in opened.iter_mut().enumerate() {
                let events = match dev.fetch_events() {
                    Ok(ev) => ev,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                    Err(e) => {
                        let _ = tx.send(InputEvent::Error(format!("evdev: {e}"))).await;
                        continue;
                    }
                };
                for ev in events {
                    if *role == DeviceRole::LocalPointer {
                        continue;
                    }
                    match ev.destructure() {
                        EventSummary::Key(_, code, value) => {
                            if *role == DeviceRole::Gamepad && primary_gamepad_idx != Some(idx) {
                                continue;
                            }
                            let pressed = value != 0;
                            apply_key(&mut state, code, pressed);
                            if *role == DeviceRole::Trackpad
                                && matches!(code, KeyCode::BTN_TOUCH | KeyCode::BTN_TOOL_FINGER)
                                && !pressed
                            {
                                pad_last.insert(idx, (None, None));
                            }
                        }
                        EventSummary::AbsoluteAxis(_, axis, value)
                            if *role == DeviceRole::Gamepad && primary_gamepad_idx == Some(idx) =>
                        {
                            let code = axis.0;
                            let (min, max) = abs_min_max.get(&(idx, code)).copied().unwrap_or_else(
                                || match axis {
                                    AbsoluteAxisCode::ABS_X
                                    | AbsoluteAxisCode::ABS_Y
                                    | AbsoluteAxisCode::ABS_RX
                                    | AbsoluteAxisCode::ABS_RY => (-32768, 32767),
                                    AbsoluteAxisCode::ABS_Z | AbsoluteAxisCode::ABS_RZ => (0, 255),
                                    AbsoluteAxisCode::ABS_HAT0X | AbsoluteAxisCode::ABS_HAT0Y => {
                                        (-1, 1)
                                    }
                                    _ => (0, 255),
                                },
                            );
                            match axis {
                                AbsoluteAxisCode::ABS_X => state.lx = norm_axis(value, min, max),
                                AbsoluteAxisCode::ABS_Y => state.ly = norm_axis(value, min, max),
                                AbsoluteAxisCode::ABS_RX => state.rx = norm_axis(value, min, max),
                                AbsoluteAxisCode::ABS_RY => state.ry = norm_axis(value, min, max),
                                AbsoluteAxisCode::ABS_Z => {
                                    state.lt = norm_trigger(value, min, max)
                                }
                                AbsoluteAxisCode::ABS_RZ => {
                                    state.rt = norm_trigger(value, min, max)
                                }
                                AbsoluteAxisCode::ABS_HAT0X => {
                                    state.dpad_left = value < 0;
                                    state.dpad_right = value > 0;
                                }
                                AbsoluteAxisCode::ABS_HAT0Y => {
                                    state.dpad_up = value < 0;
                                    state.dpad_down = value > 0;
                                }
                                _ => {}
                            }
                        }
                        EventSummary::AbsoluteAxis(_, axis, value)
                            if *role == DeviceRole::Trackpad && !pad_saw_rel =>
                        {
                            if !state.rpad_touch {
                                continue;
                            }
                            let entry = pad_last.entry(idx).or_insert((None, None));
                            match axis {
                                AbsoluteAxisCode::ABS_X | AbsoluteAxisCode::ABS_MT_POSITION_X => {
                                    if let Some(prev) = entry.0 {
                                        let d = (value - prev) as f32;
                                        if d.abs() < 400.0 {
                                            state.rpad_dx += (d * 0.05).clamp(-12.0, 12.0);
                                        }
                                    }
                                    entry.0 = Some(value);
                                }
                                AbsoluteAxisCode::ABS_Y | AbsoluteAxisCode::ABS_MT_POSITION_Y => {
                                    if let Some(prev) = entry.1 {
                                        let d = (value - prev) as f32;
                                        if d.abs() < 400.0 {
                                            state.rpad_dy += (d * 0.05).clamp(-12.0, 12.0);
                                        }
                                    }
                                    entry.1 = Some(value);
                                }
                                _ => {}
                            }
                        }
                        EventSummary::RelativeAxis(_, axis, value)
                            if *role == DeviceRole::Trackpad =>
                        {
                            pad_saw_rel = true;
                            match axis {
                                RelativeAxisCode::REL_X => {
                                    state.rpad_dx += (value as f32).clamp(-12.0, 12.0);
                                }
                                RelativeAxisCode::REL_Y => {
                                    state.rpad_dy += (value as f32).clamp(-12.0, 12.0);
                                }
                                _ => {}
                            }
                        }
                        EventSummary::RelativeAxis(_, _, _) if *role == DeviceRole::Gamepad => {}
                        _ => {}
                    }
                }
            }

            state.rpad_dx = state.rpad_dx.clamp(-20.0, 20.0);
            state.rpad_dy = state.rpad_dy.clamp(-20.0, 20.0);

            if tx.send(InputEvent::State(state.clone())).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(8)).await;
        }
    });

    Ok(())
}
