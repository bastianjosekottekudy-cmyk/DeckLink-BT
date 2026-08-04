//! Linux evdev capture for Steam Deck controls + trackpads.

use std::collections::HashMap;
use std::time::Duration;

use evdev::{AbsoluteAxisCode, Device, EventSummary, KeyCode, RelativeAxisCode};
use tokio::sync::mpsc;
use tracing::{info, warn};

use decklink_hid::{ControllerState, GamepadButtons};

use crate::{read_battery_percent, InputError, InputEvent};

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeviceRole {
    Gamepad,
    Trackpad,
}

fn list_devices() -> Vec<(std::path::PathBuf, String)> {
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
    // Deck often exposes pads under Valve / Steam names with relative axes only —
    // we probe after open; give mild score to Steam Deck non-pad siblings.
    if lower.contains("steam") && (lower.contains("pad") || lower.contains("mouse")) {
        return 50;
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
        KeyCode::BTN_LEFT => {
            state.trackpad_click = pressed;
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

pub async fn spawn_input_task(tx: mpsc::Sender<InputEvent>) -> Result<(), InputError> {
    let devices = list_devices();
    for (path, name) in &devices {
        info!(
            "input candidate: {} ({}) pad={} track={}",
            path.display(),
            name,
            score_gamepad(name),
            score_trackpad(name)
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
        .into_iter()
        .map(|(path, name)| (score_trackpad(&name), path, name))
        .filter(|(s, _, _)| *s > 0)
        .collect();
    tracks.sort_by(|a, b| b.0.cmp(&a.0));

    let mut opened: Vec<(DeviceRole, String, Device)> = Vec::new();

    // One primary gamepad only — IMU/extra pads were overwriting stick axes.
    if let Some((_score, path, name)) = pads.into_iter().next() {
        match Device::open(&path) {
            Ok(mut d) => {
                let _ = d.set_nonblocking(true);
                if let Err(e) = d.grab() {
                    warn!("grab {} failed (continuing): {e}", path.display());
                } else {
                    info!("grabbed gamepad {}", path.display());
                }
                info!("opening gamepad {} ({})", path.display(), name);
                opened.push((DeviceRole::Gamepad, name, d));
            }
            Err(e) => warn!("failed to open gamepad {}: {}", path.display(), e),
        }
    }

    for (_score, path, name) in tracks.into_iter().take(2) {
        match Device::open(&path) {
            Ok(mut d) => {
                let _ = d.set_nonblocking(true);
                info!("opening trackpad {} ({})", path.display(), name);
                opened.push((DeviceRole::Trackpad, name, d));
            }
            Err(e) => warn!("failed to open trackpad {}: {}", path.display(), e),
        }
    }

    if opened.is_empty() {
        return Err(InputError::NoDevice);
    }

    tokio::spawn(async move {
        let mut state = ControllerState::default();
        let mut abs_min_max: HashMap<(usize, u16), (i32, i32)> = HashMap::new();

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
            state.clear_relative();
            state.battery_pct = read_battery_percent();
            let mut got_event = false;

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
                    match ev.destructure() {
                        EventSummary::Key(_, code, value) => {
                            apply_key(&mut state, code, value != 0);
                        }
                        EventSummary::AbsoluteAxis(_, axis, value) if *role == DeviceRole::Gamepad => {
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
                        EventSummary::AbsoluteAxis(_, axis, value) if *role == DeviceRole::Trackpad => {
                            // Absolute touchpads: treat deltas from center-ish movement poorly;
                            // prefer REL when available. Map ABS_X/Y lightly as cursor nudge.
                            match axis {
                                AbsoluteAxisCode::ABS_X => {
                                    state.trackpad_dx += norm_axis(value, 0, 1000) * 8.0;
                                }
                                AbsoluteAxisCode::ABS_Y => {
                                    state.trackpad_dy += norm_axis(value, 0, 1000) * 8.0;
                                }
                                _ => {}
                            }
                        }
                        EventSummary::RelativeAxis(_, axis, value) => match axis {
                            RelativeAxisCode::REL_X => state.trackpad_dx += value as f32,
                            RelativeAxisCode::REL_Y => state.trackpad_dy += value as f32,
                            RelativeAxisCode::REL_WHEEL => {
                                state.trackpad_dy += value as f32 * 2.0;
                            }
                            _ => {}
                        },
                        _ => {}
                    }
                }
            }

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
