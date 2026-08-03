//! Linux evdev capture for Steam Deck controls + IMU.

use std::collections::HashMap;
use std::time::Duration;

use evdev::{
    AbsoluteAxisCode, Device, EventSummary, KeyCode, RelativeAxisCode,
};
use tokio::sync::mpsc;
use tracing::{info, warn};

use decklink_hid::{ControllerState, GamepadButtons};

use crate::{read_battery_percent, InputError, InputEvent};

const DECK_NAME_HINTS: &[&str] = &[
    "Steam Deck",
    "Microsoft X-Box 360 pad",
    "Valve Software Steam Controller",
    "Handheld",
];

fn list_devices() -> Vec<(std::path::PathBuf, String)> {
    let mut out = Vec::new();
    for (path, device) in evdev::enumerate() {
        let name = device.name().unwrap_or("").to_string();
        out.push((path, name));
    }
    out
}

fn is_candidate(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower.contains("mouse") || lower.contains("keyboard") || lower.contains("consumer") {
        return false;
    }
    DECK_NAME_HINTS.iter().any(|h| name.contains(h))
        || lower.contains("steam")
        || lower.contains("x-box")
        || lower.contains("xbox")
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
        KeyCode::BTN_TOUCH => {
            state.trackpad_touch = pressed;
            Some(GamepadButtons::TOUCH)
        }
        KeyCode::BTN_TOOL_FINGER | KeyCode::BTN_LEFT => {
            state.trackpad_click = pressed;
            None
        }
        KeyCode::BTN_TRIGGER_HAPPY1 => Some(GamepadButtons::L4),
        KeyCode::BTN_TRIGGER_HAPPY2 => Some(GamepadButtons::R4),
        KeyCode::BTN_TRIGGER_HAPPY3 => Some(GamepadButtons::L5),
        KeyCode::BTN_TRIGGER_HAPPY4 => Some(GamepadButtons::R5),
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
    let mut opened: Vec<Device> = Vec::new();

    for (path, name) in &devices {
        if !is_candidate(name) {
            continue;
        }
        match Device::open(path) {
            Ok(d) => {
                info!("opening input device {} ({})", path.display(), name);
                opened.push(d);
            }
            Err(e) => warn!("failed to open {}: {}", path.display(), e),
        }
    }

    for (path, name) in &devices {
        let lower = name.to_ascii_lowercase();
        if lower.contains("gyro")
            || lower.contains("accel")
            || lower.contains("imu")
            || lower.contains("motion")
        {
            if let Ok(d) = Device::open(path) {
                info!("opening motion device {} ({})", path.display(), name);
                opened.push(d);
            }
        }
    }

    if opened.is_empty() {
        for (path, name) in &devices {
            if let Ok(d) = Device::open(path) {
                info!("fallback open {} ({})", path.display(), name);
                opened.push(d);
                break;
            }
        }
    }

    if opened.is_empty() {
        return Err(InputError::NoDevice);
    }

    tokio::spawn(async move {
        let mut state = ControllerState::default();
        let mut abs_min_max: HashMap<(usize, AbsoluteAxisCode), (i32, i32)> = HashMap::new();

        for (idx, dev) in opened.iter().enumerate() {
            if let Ok(abs_state) = dev.get_abs_state() {
                for (i, info) in abs_state.iter().enumerate() {
                    if info.maximum != info.minimum {
                        let code = AbsoluteAxisCode(i as u16);
                        abs_min_max.insert((idx, code), (info.minimum, info.maximum));
                    }
                }
            }
        }

        loop {
            state.clear_relative();
            state.battery_pct = read_battery_percent();

            for (idx, dev) in opened.iter_mut().enumerate() {
                let events = match dev.fetch_events() {
                    Ok(ev) => ev,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                    Err(e) => {
                        let _ = tx.send(InputEvent::Error(format!("evdev: {e}"))).await;
                        continue;
                    }
                };

                for ev in events {
                    match ev.destructure() {
                        EventSummary::Key(_, code, value) => {
                            apply_key(&mut state, code, value != 0);
                        }
                        EventSummary::AbsoluteAxis(_, axis, value) => {
                            let (min, max) = abs_min_max
                                .get(&(idx, axis))
                                .copied()
                                .unwrap_or((0, 255));
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
                                AbsoluteAxisCode::ABS_MISC => {
                                    state.gyro_z = value as f32 * 0.001;
                                }
                                _ => {}
                            }
                        }
                        EventSummary::RelativeAxis(_, axis, value) => match axis {
                            RelativeAxisCode::REL_X => {
                                state.trackpad_dx += value as f32;
                            }
                            RelativeAxisCode::REL_Y => {
                                state.trackpad_dy += value as f32;
                            }
                            RelativeAxisCode::REL_WHEEL => {
                                state.trackpad_dy += value as f32 * 2.0;
                            }
                            RelativeAxisCode::REL_RX => {
                                state.gyro_x = value as f32 * 0.001;
                            }
                            RelativeAxisCode::REL_RY => {
                                state.gyro_y = value as f32 * 0.001;
                            }
                            RelativeAxisCode::REL_RZ => {
                                state.gyro_z = value as f32 * 0.001;
                            }
                            _ => {}
                        },
                        _ => {}
                    }
                }
            }

            if tx.send(InputEvent::State(state.clone())).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(4)).await;
        }
    });

    Ok(())
}
