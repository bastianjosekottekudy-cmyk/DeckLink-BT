//! Linux evdev capture for Steam Deck controls + trackpads.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use evdev::{AbsoluteAxisCode, Device, EventSummary, KeyCode, RelativeAxisCode};
use tokio::sync::mpsc;
use tracing::{info, warn};

use decklink_hid::{ControllerState, GamepadButtons};

use crate::{read_battery_percent, InputCommand, InputError, InputEvent};

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeviceRole {
    Gamepad,
    Trackpad,
    /// Lizard / Steam Desktop mouse+keyboard — grab only (never feed host HID).
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
    // Real Deck touchpads — not the firmware "… Mouse" lizard interface.
    if lower.contains("touchpad") || lower.contains("trackpad") {
        return 80;
    }
    if lower.contains("steam") && lower.contains("pad") && !lower.contains("mouse") {
        return 50;
    }
    -100
}

/// Devices that move the *local* Desktop cursor (lizard mode / Steam Desktop Config).
fn score_local_pointer(name: &str) -> i32 {
    let lower = name.to_ascii_lowercase();
    if lower.contains("decklink") {
        return -100;
    }
    if (lower.contains("mouse") || lower.contains("keyboard") || lower.contains("consumer"))
        && (lower.contains("steam")
            || lower.contains("valve")
            || lower.contains("deck")
            || lower.contains("jupiter"))
    {
        return 90;
    }
    if lower.contains("extest") || (lower.contains("uinput") && lower.contains("mouse")) {
        return 80;
    }
    if lower.contains("steam") && lower.contains("mouse") {
        return 85;
    }
    -100
}

fn apply_key(state: &mut ControllerState, code: KeyCode, pressed: bool) {
    let flag = match code {
        KeyCode::BTN_SOUTH => Some(GamepadButtons::A),
        KeyCode::BTN_EAST => Some(GamepadButtons::B),
        KeyCode::BTN_NORTH => Some(GamepadButtons::X),
        KeyCode::BTN_WEST => Some(GamepadButtons::Y),
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
                state.trackpad_click = pressed;
            } else {
                state.trackpad_touch = pressed;
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

fn open_device(path: &PathBuf, name: &str, role: DeviceRole) -> Result<Device, String> {
    let d = Device::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let _ = d.set_nonblocking(true);
    info!("opened {} {} ({name})", role_name(role), path.display());
    Ok(d)
}

/// Valve (0x28DE) interfaces that look like mouse/keyboard — grab to stop Desktop cursor.
fn looks_like_valve_pointer(dev: &Device, name: &str) -> bool {
    if score_local_pointer(name) > 0 {
        return true;
    }
    let id = dev.input_id();
    if id.vendor() != 0x28DE {
        return false;
    }
    // Virtual Steam gamepad is 0x11FF — leave that as gamepad, not pointer.
    if id.product() == 0x11FF {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    lower.contains("mouse")
        || lower.contains("keyboard")
        || lower.contains("consumer")
        || dev
            .supported_relative_axes()
            .is_some_and(|a| a.contains(RelativeAxisCode::REL_X))
}

fn role_name(role: DeviceRole) -> &'static str {
    match role {
        DeviceRole::Gamepad => "gamepad",
        DeviceRole::Trackpad => "trackpad",
        DeviceRole::LocalPointer => "local-pointer",
    }
}

fn set_devices_exclusive(
    opened: &mut [(DeviceRole, String, Device)],
    exclusive: bool,
    quiet: bool,
) {
    for (role, name, dev) in opened.iter_mut() {
        let role_name = role_name(*role);
        if exclusive {
            match dev.grab() {
                Ok(()) => {
                    if !quiet {
                        info!("exclusive grab on {role_name} ({name})");
                    }
                }
                Err(e) => warn!("grab {role_name} ({name}) failed: {e}"),
            }
        } else {
            match dev.ungrab() {
                Ok(()) => info!("released grab on {role_name} ({name}) — desktop controls restored"),
                Err(e) => warn!("ungrab {role_name} ({name}) failed: {e}"),
            }
        }
    }
}

pub async fn spawn_input_task(
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

    let mut pads: Vec<_> = devices
        .iter()
        .cloned()
        .map(|(path, name)| (score_gamepad(&name), path, name))
        .filter(|(s, _, _)| *s > 0)
        .collect();
    pads.sort_by(|a, b| b.0.cmp(&a.0));

    let mut tracks: Vec<_> = devices
        .iter()
        .cloned()
        .map(|(path, name)| (score_trackpad(&name), path, name))
        .filter(|(s, _, _)| *s > 0)
        .collect();
    tracks.sort_by(|a, b| b.0.cmp(&a.0));

    let mut locals: Vec<_> = devices
        .iter()
        .cloned()
        .map(|(path, name)| (score_local_pointer(&name), path, name))
        .filter(|(s, _, _)| *s > 0)
        .collect();
    locals.sort_by(|a, b| b.0.cmp(&a.0));

    let mut opened: Vec<(DeviceRole, String, Device)> = Vec::new();
    let mut used_paths: HashSet<PathBuf> = HashSet::new();

    // Open without grab — exclusive grab is enabled only while BLE is active
    // so sticks return to Desktop mouse when disconnected.
    for (_score, path, name) in pads.into_iter().take(5) {
        if !used_paths.insert(path.clone()) {
            continue;
        }
        match open_device(&path, &name, DeviceRole::Gamepad) {
            Ok(d) => opened.push((DeviceRole::Gamepad, name, d)),
            Err(e) => warn!("{e}"),
        }
    }

    for (_score, path, name) in tracks.into_iter().take(2) {
        if !used_paths.insert(path.clone()) {
            continue;
        }
        match open_device(&path, &name, DeviceRole::Trackpad) {
            Ok(d) => opened.push((DeviceRole::Trackpad, name, d)),
            Err(e) => warn!("{e}"),
        }
    }

    for (_score, path, name) in locals.into_iter().take(6) {
        if !used_paths.insert(path.clone()) {
            continue;
        }
        match open_device(&path, &name, DeviceRole::LocalPointer) {
            Ok(d) => opened.push((DeviceRole::LocalPointer, name, d)),
            Err(e) => warn!("{e}"),
        }
    }

    // Second pass: any remaining Valve mouse/keyboard nodes missed by name scoring.
    for (path, name) in &devices {
        if used_paths.contains(path) {
            continue;
        }
        let Ok(probe) = Device::open(path) else {
            continue;
        };
        if !looks_like_valve_pointer(&probe, name) {
            continue;
        }
        drop(probe);
        if !used_paths.insert(path.clone()) {
            continue;
        }
        match open_device(path, name, DeviceRole::LocalPointer) {
            Ok(d) => opened.push((DeviceRole::LocalPointer, name.clone(), d)),
            Err(e) => warn!("{e}"),
        }
    }

    if opened.is_empty() {
        return Err(InputError::NoDevice);
    }

    // Only the first successfully opened gamepad drives stick/button state.
    let primary_gamepad_idx = opened
        .iter()
        .position(|(r, _, _)| *r == DeviceRole::Gamepad);

    tokio::spawn(async move {
        let mut state = ControllerState::default();
        let mut abs_min_max: HashMap<(usize, u16), (i32, i32)> = HashMap::new();
        let mut exclusive = false;
        let mut lizard: Option<crate::lizard::LizardGuard> = None;
        // Absolute trackpad → relative mouse via deltas (never feed raw ABS as motion).
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

        let mut tick: u64 = 0;
        loop {
            while let Ok(cmd) = cmd_rx.try_recv() {
                let InputCommand::SetExclusive(want) = cmd;
                if want != exclusive {
                    exclusive = want;
                    set_devices_exclusive(&mut opened, exclusive, false);
                    if exclusive {
                        lizard = crate::lizard::open_and_disable();
                        if lizard.is_none() {
                            warn!(
                                "could not disable lizard mode via hidraw — hold MENU (⋯) once if Deck mouse still moves"
                            );
                        }
                    } else {
                        lizard = None;
                        info!("lizard guard released — firmware Desktop mouse can return");
                    }
                }
            }

            // Re-assert grab + lizard heartbeat; Steam/hid-steam may steal focus back.
            if exclusive {
                if tick > 0 && tick % 100 == 0 {
                    set_devices_exclusive(&mut opened, true, true);
                    if let Some(g) = &lizard {
                        g.feed_watchdog();
                    } else {
                        lizard = crate::lizard::open_and_disable();
                    }
                } else if tick % 100 == 50 {
                    if let Some(g) = &lizard {
                        g.feed_watchdog();
                    }
                }
            }

            state.clear_relative();
            state.battery_pct = read_battery_percent();
            let mut got_event = false;
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
                    got_event = true;
                    // Swallow lizard/Steam Desktop pointer (often stick→mouse).
                    // Never forward it to the host — that pins the PC cursor in a corner.
                    if *role == DeviceRole::LocalPointer {
                        continue;
                    }
                    match ev.destructure() {
                        EventSummary::Key(_, code, value) => {
                            // Keys from any grabbed gamepad/trackpad.
                            if *role == DeviceRole::Gamepad
                                && primary_gamepad_idx != Some(idx)
                            {
                                // Extra gamepad nodes: still swallow via grab, ignore state.
                                continue;
                            }
                            let pressed = value != 0;
                            apply_key(&mut state, code, pressed);
                            if *role == DeviceRole::Trackpad
                                && matches!(
                                    code,
                                    KeyCode::BTN_TOUCH | KeyCode::BTN_TOOL_FINGER
                                )
                                && !pressed
                            {
                                pad_last.insert(idx, (None, None));
                            }
                        }
                        EventSummary::AbsoluteAxis(_, axis, value)
                            if *role == DeviceRole::Gamepad
                                && primary_gamepad_idx == Some(idx) =>
                        {
                            let code = axis.0;
                            let (min, max) = abs_min_max
                                .get(&(idx, code))
                                .copied()
                                .unwrap_or_else(|| match axis {
                                    AbsoluteAxisCode::ABS_X
                                    | AbsoluteAxisCode::ABS_Y
                                    | AbsoluteAxisCode::ABS_RX
                                    | AbsoluteAxisCode::ABS_RY => (-32768, 32767),
                                    AbsoluteAxisCode::ABS_Z | AbsoluteAxisCode::ABS_RZ => (0, 255),
                                    AbsoluteAxisCode::ABS_HAT0X | AbsoluteAxisCode::ABS_HAT0Y => {
                                        (-1, 1)
                                    }
                                    _ => (0, 255),
                                });
                            match axis {
                                AbsoluteAxisCode::ABS_X => {
                                    state.lx = norm_axis(value, min, max);
                                }
                                AbsoluteAxisCode::ABS_Y => {
                                    state.ly = norm_axis(value, min, max);
                                }
                                AbsoluteAxisCode::ABS_RX => {
                                    state.rx = norm_axis(value, min, max);
                                }
                                AbsoluteAxisCode::ABS_RY => {
                                    state.ry = norm_axis(value, min, max);
                                }
                                AbsoluteAxisCode::ABS_Z => {
                                    state.lt = norm_trigger(value, min, max);
                                }
                                AbsoluteAxisCode::ABS_RZ => {
                                    state.rt = norm_trigger(value, min, max);
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
                            // Deltas only while finger is down. Ignore hover/rest ABS
                            // (resting near the bottom edge was slamming the host cursor).
                            let touching = state.trackpad_touch;
                            let entry = pad_last.entry(idx).or_insert((None, None));
                            match axis {
                                AbsoluteAxisCode::ABS_X
                                | AbsoluteAxisCode::ABS_MT_POSITION_X => {
                                    if touching {
                                        if let Some(prev) = entry.0 {
                                            let d = (value - prev) as f32;
                                            if d.abs() < 400.0 {
                                                state.trackpad_dx +=
                                                    (d * 0.05).clamp(-12.0, 12.0);
                                            }
                                        }
                                        entry.0 = Some(value);
                                    } else {
                                        *entry = (None, entry.1);
                                    }
                                }
                                AbsoluteAxisCode::ABS_Y
                                | AbsoluteAxisCode::ABS_MT_POSITION_Y => {
                                    if touching {
                                        if let Some(prev) = entry.1 {
                                            let d = (value - prev) as f32;
                                            if d.abs() < 400.0 {
                                                state.trackpad_dy +=
                                                    (d * 0.05).clamp(-12.0, 12.0);
                                            }
                                        }
                                        entry.1 = Some(value);
                                    } else {
                                        *entry = (entry.0, None);
                                    }
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
                                    state.trackpad_dx += (value as f32).clamp(-12.0, 12.0);
                                }
                                RelativeAxisCode::REL_Y => {
                                    state.trackpad_dy += (value as f32).clamp(-12.0, 12.0);
                                }
                                RelativeAxisCode::REL_WHEEL => {
                                    state.trackpad_dy +=
                                        (value as f32 * 2.0).clamp(-8.0, 8.0);
                                }
                                _ => {}
                            }
                        }
                        // Swallow relative events on gamepad nodes (prevents stick→mouse leaks).
                        EventSummary::RelativeAxis(_, _, _) if *role == DeviceRole::Gamepad => {}
                        _ => {}
                    }
                }
            }

            // Cap per-frame mouse travel so a bad sample cannot slam the cursor.
            state.trackpad_dx = state.trackpad_dx.clamp(-20.0, 20.0);
            state.trackpad_dy = state.trackpad_dy.clamp(-20.0, 20.0);

            tick += 1;
            if got_event || tick % 2 == 0 {
                if tx.send(InputEvent::State(state.clone())).await.is_err() {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(8)).await;
        }
    });

    Ok(())
}
