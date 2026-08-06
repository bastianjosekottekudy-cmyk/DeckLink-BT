//! Read Steam Deck controls directly from Valve hidraw (report 0x09).
//! Opening the controller hidraw makes hid-steam treat us as the HID client and
//! stops kernel lizard-mode filtering; we also clear digital mappings ourselves.

use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use decklink_hid::{ControllerState, GamepadButtons};
use tracing::{info, warn};

use crate::lizard::{self, LizardGuard};

/// Temporary capture: `DECKLINK_DIAG=1` or `--diag` (sets the env from main).
fn diag_on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        matches!(
            std::env::var("DECKLINK_DIAG").as_deref(),
            Ok("1") | Ok("true") | Ok("yes")
        )
    })
}

fn diag_throttle(slot: usize, min_ms: u64) -> bool {
    use std::sync::Mutex;
    static LAST: OnceLock<Mutex<[Option<Instant>; 3]>> = OnceLock::new();
    let last = LAST.get_or_init(|| Mutex::new([None, None, None]));
    let mut g = last.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    if let Some(prev) = g[slot] {
        if now.duration_since(prev) < Duration::from_millis(min_ms) {
            return false;
        }
    }
    g[slot] = Some(now);
    true
}

const VID_VALVE: u16 = 0x28DE;
const PID_STEAM_DECK: u16 = 0x1205;
const ID_CONTROLLER_DECK_STATE: u8 = 0x09;

const LIZARD_SYSFS: &str = "/sys/module/hid_steam/parameters/lizard_mode";

pub struct HidrawDeck {
    file: File,
    path: PathBuf,
    /// Kept so lizard disable stays applied / watchdog can feed.
    lizard: Option<LizardGuard>,
    /// Last abs sample + settle remaining per pad: (left, right).
    /// Settle ignores deltas for the first reports after touch (hidraw drain).
    pad_last: (Option<(i32, i32, u8)>, Option<(i32, i32, u8)>),
}

fn parse_hid_id(uevent: &str) -> Option<(u16, u16)> {
    for line in uevent.lines() {
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

fn list_deck_hidraw() -> Vec<PathBuf> {
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

fn set_nonblocking(file: &File) -> std::io::Result<()> {
    let fd = file.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let rc = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if rc < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn i16_le(data: &[u8], off: usize) -> i16 {
    i16::from_le_bytes([data[off], data[off + 1]])
}

fn u16_le(data: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([data[off], data[off + 1]])
}

fn norm_stick(v: i16) -> f32 {
    (v as f32 / 32767.0).clamp(-1.0, 1.0)
}

fn norm_trigger(v: u16) -> f32 {
    (v as f32 / 32767.0).clamp(0.0, 1.0)
}

/// Best-effort kernel lizard disable (needs write access to sysfs).
pub fn set_kernel_lizard_mode(enabled: bool) {
    let val = if enabled { "Y" } else { "N" };
    if std::fs::write(LIZARD_SYSFS, val).is_ok() {
        info!("hid_steam lizard_mode -> {val}");
        return;
    }
    // Passwordless sudo if the installer configured it.
    let _ = std::process::Command::new("sudo")
        .args(["-n", "tee", LIZARD_SYSFS])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .and_then(|mut c| {
            use std::io::Write;
            if let Some(mut stdin) = c.stdin.take() {
                let _ = stdin.write_all(val.as_bytes());
            }
            c.wait()
        });
}

pub fn open() -> Option<HidrawDeck> {
    for path in list_deck_hidraw() {
        let file = match OpenOptions::new().read(true).write(true).open(&path) {
            Ok(f) => f,
            Err(e) => {
                warn!("deck hidraw open {}: {e}", path.display());
                continue;
            }
        };
        if let Err(e) = set_nonblocking(&file) {
            warn!("deck hidraw nonblocking {}: {e}", path.display());
        }
        // Probe: controller endpoint yields 0x09 reports; mouse/kbd interfaces do not.
        let mut probe = [0u8; 64];
        let mut ok = false;
        for _ in 0..8 {
            match (&file).read(&mut probe) {
                Ok(n) if n >= 3 && probe[0] == 1 && probe[1] == 0 && probe[2] == ID_CONTROLLER_DECK_STATE => {
                    ok = true;
                    break;
                }
                Ok(_) => continue,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
        // Even without an immediate report, try lizard disable — Steam may own the bus.
        let lizard = lizard::open_and_disable_on(&path);
        if lizard.is_none() && !ok {
            warn!("deck hidraw {} not usable (no reports / lizard)", path.display());
            continue;
        }
        set_kernel_lizard_mode(false);
        info!(
            "deck hidraw input via {} (probe_ok={ok}, lizard={})",
            path.display(),
            lizard.is_some()
        );
        return Some(HidrawDeck {
            file,
            path,
            lizard,
            pad_last: (None, None),
        });
    }
    None
}

impl HidrawDeck {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn feed_lizard(&mut self) {
        if let Some(g) = &self.lizard {
            g.feed_watchdog();
        } else {
            self.lizard = lizard::open_and_disable_on(&self.path);
            if let Some(g) = &self.lizard {
                g.feed_watchdog();
            }
        }
        set_kernel_lizard_mode(false);
    }

    /// Read latest deck state. Returns true if a fresh report was applied.
    pub fn poll(&mut self, state: &mut ControllerState) -> bool {
        let mut buf = [0u8; 64];
        let mut got = false;
        // Drain the queue; keep the newest deck report.
        loop {
            match self.file.read(&mut buf) {
                Ok(n) if n >= 56 && buf[0] == 1 && buf[1] == 0 && buf[2] == ID_CONTROLLER_DECK_STATE => {
                    apply_deck_report(&buf, state, &mut self.pad_last);
                    got = true;
                }
                Ok(_) => continue,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => {
                    warn!("deck hidraw read: {e}");
                    break;
                }
            }
        }
        got
    }
}

fn apply_deck_report(
    data: &[u8],
    state: &mut ControllerState,
    pad_last: &mut (Option<(i32, i32, u8)>, Option<(i32, i32, u8)>),
) {
    // Buttons are a little-endian u64 at offset 8 (SDL / hid-steam layout).
    let buttons_raw = u64::from_le_bytes([
        data.get(8).copied().unwrap_or(0),
        data.get(9).copied().unwrap_or(0),
        data.get(10).copied().unwrap_or(0),
        data.get(11).copied().unwrap_or(0),
        data.get(12).copied().unwrap_or(0),
        data.get(13).copied().unwrap_or(0),
        data.get(14).copied().unwrap_or(0),
        data.get(15).copied().unwrap_or(0),
    ]);

    // Low-word bits (SDL STEAMDECK_LBUTTON_*)
    const A: u64 = 1 << 7;
    const B: u64 = 1 << 5;
    const X: u64 = 1 << 6;
    const Y: u64 = 1 << 4;
    const LB: u64 = 1 << 3;
    const RB: u64 = 1 << 2;
    const LT_FULL: u64 = 1 << 1;
    const RT_FULL: u64 = 1 << 0;
    const DPAD_UP: u64 = 1 << 8;
    const DPAD_RIGHT: u64 = 1 << 9;
    const DPAD_LEFT: u64 = 1 << 10;
    const DPAD_DOWN: u64 = 1 << 11;
    const VIEW: u64 = 1 << 12;
    const STEAM: u64 = 1 << 13;
    const MENU: u64 = 1 << 14;
    const L5: u64 = 1 << 15;
    const R5: u64 = 1 << 16;
    const LPAD_CLICK: u64 = 1 << 17;
    const RPAD_CLICK: u64 = 1 << 18;
    const LPAD_TOUCH: u64 = 1 << 19;
    const RPAD_TOUCH: u64 = 1 << 20;
    const L3: u64 = 1 << 22;
    const R3: u64 = 1 << 26;
    const L4: u64 = 1 << (32 + 9);
    const R4: u64 = 1 << (32 + 10);
    const QAM: u64 = 1 << (32 + 18);

    let mut buttons = GamepadButtons::empty();
    if buttons_raw & A != 0 {
        buttons |= GamepadButtons::A;
    }
    if buttons_raw & B != 0 {
        buttons |= GamepadButtons::B;
    }
    if buttons_raw & X != 0 {
        buttons |= GamepadButtons::X;
    }
    if buttons_raw & Y != 0 {
        buttons |= GamepadButtons::Y;
    }
    if buttons_raw & LB != 0 {
        buttons |= GamepadButtons::L1;
    }
    if buttons_raw & RB != 0 {
        buttons |= GamepadButtons::R1;
    }
    if buttons_raw & VIEW != 0 {
        buttons |= GamepadButtons::SELECT;
    }
    if buttons_raw & MENU != 0 {
        buttons |= GamepadButtons::START;
    }
    if buttons_raw & (STEAM | QAM) != 0 {
        buttons |= GamepadButtons::GUIDE;
    }
    if buttons_raw & L3 != 0 {
        buttons |= GamepadButtons::L3;
    }
    if buttons_raw & R3 != 0 {
        buttons |= GamepadButtons::R3;
    }
    if buttons_raw & L4 != 0 {
        buttons |= GamepadButtons::L4;
    }
    if buttons_raw & R4 != 0 {
        buttons |= GamepadButtons::R4;
    }
    if buttons_raw & L5 != 0 {
        buttons |= GamepadButtons::L5;
    }
    if buttons_raw & R5 != 0 {
        buttons |= GamepadButtons::R5;
    }
    state.buttons = buttons;

    state.dpad_up = buttons_raw & DPAD_UP != 0;
    state.dpad_right = buttons_raw & DPAD_RIGHT != 0;
    state.dpad_left = buttons_raw & DPAD_LEFT != 0;
    state.dpad_down = buttons_raw & DPAD_DOWN != 0;

    state.lx = norm_stick(i16_le(data, 48));
    state.ly = norm_stick(-i16_le(data, 50));
    state.rx = norm_stick(i16_le(data, 52));
    state.ry = norm_stick(-i16_le(data, 54));

    state.lt = norm_trigger(u16_le(data, 44));
    state.rt = norm_trigger(u16_le(data, 46));
    if buttons_raw & LT_FULL != 0 {
        state.lt = state.lt.max(1.0);
    }
    if buttons_raw & RT_FULL != 0 {
        state.rt = state.rt.max(1.0);
    }

    state.lpad_touch = buttons_raw & LPAD_TOUCH != 0;
    state.rpad_touch = buttons_raw & RPAD_TOUCH != 0;
    state.lpad_click = buttons_raw & LPAD_CLICK != 0;
    state.rpad_click = buttons_raw & RPAD_CLICK != 0;

    state.lpad_dx = 0.0;
    state.lpad_dy = 0.0;
    state.rpad_dx = 0.0;
    state.rpad_dy = 0.0;

    let lx = i16_le(data, 16) as i32;
    let ly = i16_le(data, 18) as i32;
    let rx = i16_le(data, 20) as i32;
    let ry = i16_le(data, 22) as i32;

    // Pads are signed absolute ±32767 (center origin). Log-backed: scale 0.06 +
    // clamp ±20 slammed the host cursor; touch-down + hidraw drain produced huge
    // first deltas. Settle several reports, scale for full-range abs, tight clamp.
    const SETTLE: u8 = 5;
    const MAX_RAW: f32 = 2000.0;
    const SCALE: f32 = 0.012;
    const MAX_OUT: f32 = 6.0;

    fn pad_delta(
        touched: bool,
        x: i32,
        y: i32,
        last: &mut Option<(i32, i32, u8)>,
        which: char,
    ) -> (f32, f32) {
        if !touched {
            *last = None;
            return (0.0, 0.0);
        }
        match *last {
            None => {
                *last = Some((x, y, SETTLE));
                if diag_on() {
                    info!("DIAG pad {which} TOUCH_DOWN raw=({x},{y}) settle={SETTLE}");
                }
                (0.0, 0.0)
            }
            Some((_px, _py, settle)) if settle > 0 => {
                *last = Some((x, y, settle - 1));
                (0.0, 0.0)
            }
            Some((px, py, 0)) => {
                let dx = (x - px) as f32;
                let dy = (py - y) as f32; // finger up → cursor up
                *last = Some((x, y, 0));
                if dx.abs() > MAX_RAW || dy.abs() > MAX_RAW {
                    if diag_on() && diag_throttle(0, 20) {
                        info!(
                            "DIAG pad {which} DROP raw_d=({dx:.0},{dy:.0}) abs=({x},{y}) prev=({px},{py})"
                        );
                    }
                    return (0.0, 0.0);
                }
                let ox = (dx * SCALE).clamp(-MAX_OUT, MAX_OUT);
                let oy = (dy * SCALE).clamp(-MAX_OUT, MAX_OUT);
                if diag_on() && (ox != 0.0 || oy != 0.0) && diag_throttle(1, 40) {
                    info!(
                        "DIAG pad {which} abs=({x},{y}) raw_d=({dx:.0},{dy:.0}) out=({ox:.2},{oy:.2})"
                    );
                }
                (ox, oy)
            }
            _ => (0.0, 0.0),
        }
    }

    let (ldx, ldy) = pad_delta(state.lpad_touch, lx, ly, &mut pad_last.0, 'L');
    state.lpad_dx = ldx;
    state.lpad_dy = ldy;
    let (rdx, rdy) = pad_delta(state.rpad_touch, rx, ry, &mut pad_last.1, 'R');
    state.rpad_dx = rdx;
    state.rpad_dy = rdy;

    if diag_on()
        && !state.lpad_touch
        && !state.rpad_touch
        && (state.rx.abs() > 0.05 || state.ry.abs() > 0.05)
        && diag_throttle(2, 5000)
    {
        info!(
            "DIAG stick rest rx={:.3} ry={:.3} (not mapped to mouse)",
            state.rx, state.ry
        );
    }
}
