//! Disable Steam Deck "lizard mode" (firmware keyboard/mouse) via hidraw.
//! Keeps sticks/pads from driving the local Desktop cursor while DeckLink is active.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use tracing::{info, warn};

const VID_VALVE: u16 = 0x28DE;
const PID_STEAM_DECK: u16 = 0x1205;

const ID_CLEAR_DIGITAL_MAPPINGS: u8 = 0x81;
const ID_SET_SETTINGS_VALUES: u8 = 0x87;

const SETTING_MOUSE_POINTER_ENABLED: u8 = 9;
const SETTING_SMOOTH_ABSOLUTE_MOUSE: u8 = 24;
const SETTING_STEAM_WATCHDOG_ENABLE: u8 = 71;

const FEATURE_LEN: usize = 65; // report-id byte + 64

nix::ioctl_readwrite_buf!(hid_sfeature, b'H', 0x06, u8);
nix::ioctl_readwrite_buf!(hid_gfeature, b'H', 0x07, u8);

pub struct LizardGuard {
    file: File,
}

fn parse_hid_id(uevent: &str) -> Option<(u16, u16)> {
    for line in uevent.lines() {
        // HID_ID=0003:000028DE:00001205
        let Some(rest) = line.strip_prefix("HID_ID=") else {
            continue;
        };
        let mut parts = rest.split(':');
        let _bus = parts.next()?;
        let vid = u16::from_str_radix(parts.next()?, 16).ok()?;
        let pid = u16::from_str_radix(parts.next()?, 16).ok()?;
        return Some((vid, pid));
    }
    None
}

fn list_valve_hidraw() -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir("/dev") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for ent in entries.flatten() {
        let name = ent.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("hidraw") {
            continue;
        }
        let path = ent.path();
        let sys = Path::new("/sys/class/hidraw")
            .join(name.as_ref())
            .join("device/uevent");
        let Ok(uevent) = std::fs::read_to_string(&sys) else {
            continue;
        };
        let Some((vid, pid)) = parse_hid_id(&uevent) else {
            continue;
        };
        if vid == VID_VALVE && pid == PID_STEAM_DECK {
            out.push(path);
        }
    }
    out.sort();
    out
}

fn send_feature(file: &File, payload: &[u8]) -> std::io::Result<()> {
    let mut buf = [0u8; FEATURE_LEN];
    let n = payload.len().min(FEATURE_LEN - 1);
    buf[1..1 + n].copy_from_slice(&payload[..n]);
    let fd = file.as_raw_fd();
    match unsafe { hid_sfeature(fd, &mut buf[..FEATURE_LEN]) } {
        Ok(_) => Ok(()),
        Err(e) => Err(std::io::Error::from(e)),
    }
}

fn discard_feature(file: &File) {
    let mut buf = [0u8; FEATURE_LEN];
    let fd = file.as_raw_fd();
    let _ = unsafe { hid_gfeature(fd, &mut buf[..FEATURE_LEN]) };
}

fn build_settings(pairs: &[(u8, u16)]) -> Vec<u8> {
    let mut cmd = vec![ID_SET_SETTINGS_VALUES, 0u8];
    for &(reg, val) in pairs {
        cmd.push(reg);
        cmd.push((val & 0xff) as u8);
        cmd.push((val >> 8) as u8);
        cmd[1] = cmd[1].wrapping_add(3);
    }
    cmd
}

fn apply_disable(file: &File) -> std::io::Result<()> {
    // Clear stick/button → keyboard/mouse mappings. Keep trackpad modes so we can
    // still read the grabbed lizard mouse node and forward it to the host.
    send_feature(file, &[ID_CLEAR_DIGITAL_MAPPINGS])?;
    let settings = build_settings(&[
        (SETTING_MOUSE_POINTER_ENABLED, 0),
        (SETTING_SMOOTH_ABSOLUTE_MOUSE, 0),
        (SETTING_STEAM_WATCHDOG_ENABLE, 0),
    ]);
    send_feature(file, &settings)?;
    discard_feature(file);
    Ok(())
}

fn apply_watchdog_feed(file: &File) -> std::io::Result<()> {
    send_feature(file, &[ID_CLEAR_DIGITAL_MAPPINGS])?;
    let settings = build_settings(&[(SETTING_MOUSE_POINTER_ENABLED, 0)]);
    send_feature(file, &settings)?;
    discard_feature(file);
    Ok(())
}

/// Open a Deck controller hidraw and disable lizard mode.
pub fn open_and_disable() -> Option<LizardGuard> {
    for path in list_valve_hidraw() {
        let file = match OpenOptions::new().read(true).write(true).open(&path) {
            Ok(f) => f,
            Err(e) => {
                warn!("hidraw open {}: {e}", path.display());
                continue;
            }
        };
        match apply_disable(&file) {
            Ok(()) => {
                info!("lizard mode disabled via {}", path.display());
                return Some(LizardGuard { file });
            }
            Err(e) => warn!("lizard disable on {}: {e}", path.display()),
        }
    }
    None
}

impl LizardGuard {
    pub fn feed_watchdog(&self) {
        if let Err(e) = apply_watchdog_feed(&self.file) {
            warn!("lizard watchdog feed: {e}");
        }
    }
}
