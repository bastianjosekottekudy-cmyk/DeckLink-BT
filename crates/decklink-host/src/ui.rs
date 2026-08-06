//! DeckLink Host window + system tray (Windows).
//!
//! egui/eframe research (issues #5229, #7776, discussion #737):
//!   • `ViewportCommand::Visible(false)` / Win32 `SW_HIDE` → event loop often spins a
//!     full core and never runs `update()`, so Show/Quit via egui break.
//!   • `SW_MINIMIZE` leaves a floating title chip and taskbar peeks on hover.
//!   • Working approach: capture HWND; park **off-screen** + `WS_EX_TOOLWINDOW`;
//!     tray **Click** (not hover) restores; Quit exits from the tray thread;
//!     sleep while parked for ~0% CPU.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetWindowLongPtrW, GetWindowRect, IsWindowVisible, SetForegroundWindow,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, GWL_EXSTYLE, SWP_NOACTIVATE, SWP_NOZORDER,
    SW_RESTORE, SW_SHOWDEFAULT, WINDOW_EX_STYLE, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};

use crate::server::HostHandle;

pub const WINDOW_TITLE: &str = "DeckLink Host";
pub const TRAY_RPC_ADDR: &str = "127.0.0.1:31416";

/// Saved client placement before parking off-screen: x, y, w, h.
type SavedPlace = (i32, i32, i32, i32);

fn to_wide(s: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn as_hwnd(raw: isize) -> Option<HWND> {
    if raw == 0 {
        None
    } else {
        Some(HWND(raw as *mut _))
    }
}

fn find_by_title() -> isize {
    let title = to_wide(WINDOW_TITLE);
    unsafe {
        FindWindowW(PCWSTR::null(), PCWSTR(title.as_ptr()))
            .map(|h| h.0 as isize)
            .unwrap_or(0)
    }
}

fn resolve(slot: &AtomicIsize) -> isize {
    let cur = slot.load(Ordering::SeqCst);
    if cur != 0 {
        return cur;
    }
    let found = find_by_title();
    if found != 0 {
        slot.store(found, Ordering::SeqCst);
    }
    found
}

/// Park off-screen + hide from taskbar (no minimize chip, no hover peek).
fn win_to_tray(slot: &AtomicIsize, saved: &Mutex<Option<SavedPlace>>) -> bool {
    let Some(h) = as_hwnd(resolve(slot)) else {
        return false;
    };
    unsafe {
        let mut rc = RECT::default();
        if GetWindowRect(h, &mut rc).is_ok() {
            let w = (rc.right - rc.left).max(1);
            let hgt = (rc.bottom - rc.top).max(1);
            // Only remember on-screen placements (not a previous park).
            if rc.left > -10_000 {
                *saved.lock().unwrap() = Some((rc.left, rc.top, w, hgt));
            }
        }
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
        let mut style = WINDOW_EX_STYLE(ex as u32);
        style |= WS_EX_TOOLWINDOW;
        style &= !WS_EX_APPWINDOW;
        SetWindowLongPtrW(h, GWL_EXSTYLE, style.0 as isize);
        let _ = SetWindowPos(
            h,
            None,
            -32_000,
            -32_000,
            1,
            1,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
    true
}

fn win_from_tray(slot: &AtomicIsize, saved: &Mutex<Option<SavedPlace>>) -> bool {
    let Some(h) = as_hwnd(resolve(slot)) else {
        return false;
    };
    unsafe {
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
        let mut style = WINDOW_EX_STYLE(ex as u32);
        style &= !WS_EX_TOOLWINDOW;
        style |= WS_EX_APPWINDOW;
        SetWindowLongPtrW(h, GWL_EXSTYLE, style.0 as isize);
        if let Some((x, y, w, hgt)) = *saved.lock().unwrap() {
            let _ = SetWindowPos(h, None, x, y, w, hgt, SWP_NOZORDER);
        }
        let _ = ShowWindow(h, SW_SHOWDEFAULT);
        let _ = ShowWindow(h, SW_RESTORE);
        let _ = SetForegroundWindow(h);
        IsWindowVisible(h).as_bool()
    }
}

fn hard_quit(stop: &AtomicBool) -> ! {
    stop.store(true, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(40));
    std::process::exit(0);
}

struct HostApp {
    handle: HostHandle,
    tray: Option<TrayIcon>,
    hwnd: Arc<AtomicIsize>,
    saved_place: Arc<Mutex<Option<SavedPlace>>>,
    in_tray: Arc<AtomicBool>,
    last_tip: String,
    last_gen: u64,
}

impl eframe::App for HostApp {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        egui::Color32::from_rgb(15, 20, 25).to_normalized_gamma_f32()
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.hwnd.load(Ordering::SeqCst) == 0 {
            if let Ok(wh) = frame.window_handle() {
                if let RawWindowHandle::Win32(h) = wh.as_raw() {
                    self.hwnd.store(h.hwnd.get(), Ordering::SeqCst);
                }
            }
        }

        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.in_tray.store(true, Ordering::SeqCst);
            let _ = win_to_tray(&self.hwnd, &self.saved_place);
        }

        if self.in_tray.load(Ordering::SeqCst) {
            // Event loop still runs while minimized — sleep to keep CPU ~0%.
            std::thread::sleep(Duration::from_millis(250));
            ctx.request_repaint_after(Duration::from_millis(250));
            // Still refresh tray tooltip on status changes.
            let st = self.handle.status.lock().unwrap().clone();
            let tip = if let Some(ref p) = st.peer_name {
                format!("DeckLink Host — linked: {p}")
            } else if st.listening {
                "DeckLink Host — waiting for Deck".to_string()
            } else {
                "DeckLink Host — starting…".to_string()
            };
            if tip != self.last_tip {
                if let Some(t) = self.tray.as_ref() {
                    let _ = t.set_tooltip(Some(tip.as_str()));
                }
                self.last_tip = tip;
            }
            return;
        }

        let gen = self.handle.status_gen.load(Ordering::Relaxed);
        let st = self.handle.status.lock().unwrap().clone();
        let tip = if let Some(ref p) = st.peer_name {
            format!("DeckLink Host — linked: {p}")
        } else if st.listening {
            "DeckLink Host — waiting for Deck".to_string()
        } else if let Some(ref e) = st.last_error {
            format!("DeckLink Host — {e}")
        } else {
            "DeckLink Host — starting…".to_string()
        };
        if tip != self.last_tip {
            if let Some(t) = self.tray.as_ref() {
                let _ = t.set_tooltip(Some(tip.as_str()));
            }
            self.last_tip = tip;
        }

        let changed = gen != self.last_gen;
        self.last_gen = gen;

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(8.0);
            ui.heading("DeckLink Host");
            ui.label("Steam Deck finds this PC on Wi‑Fi (UDP 31415).");
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let (label, color) = if st.peer.is_some() {
                    ("LINKED", egui::Color32::from_rgb(61, 214, 140))
                } else if st.listening {
                    ("WAITING", egui::Color32::from_rgb(94, 176, 255))
                } else {
                    ("OFF", egui::Color32::from_rgb(240, 113, 120))
                };
                ui.colored_label(color, label);
                ui.label(format!("listen {}", st.bind));
            });
            ui.add_space(8.0);
            ui.label("This PC on Wi‑Fi / LAN (UDP 31415):");
            if st.lan_ips.is_empty() {
                ui.label("(no IPv4 found yet)");
            } else {
                for line in &st.lan_ips {
                    ui.monospace(line);
                }
            }
            ui.label("On Deck: Connect, or type the Wi‑Fi IP above.");
            ui.add_space(8.0);
            if st.vigem_ok {
                ui.colored_label(egui::Color32::from_rgb(61, 214, 140), "ViGEmBus: ready");
            } else {
                ui.colored_label(egui::Color32::from_rgb(240, 113, 120), "ViGEmBus: not ready");
            }
            ui.add_space(8.0);
            if let Some(ref n) = st.peer_name {
                ui.label(format!("Deck: {n}"));
            } else {
                ui.label("No Deck connected yet.");
            }
            if let Some(ref e) = st.last_error {
                ui.add_space(6.0);
                ui.colored_label(egui::Color32::from_rgb(240, 113, 120), e);
            }
            ui.add_space(14.0);
            ui.label("Close → tray. Right-click tray → Show / Quit.");
            if ui.button("Quit").clicked() {
                hard_quit(&self.handle.stop);
            }
        });

        ctx.request_repaint_after(if changed {
            Duration::from_millis(200)
        } else {
            Duration::from_secs(2)
        });
    }

    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        self.tray.take();
        self.handle.request_stop();
    }
}

fn spawn_rpc(
    hwnd: Arc<AtomicIsize>,
    saved: Arc<Mutex<Option<SavedPlace>>>,
    in_tray: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) {
    std::thread::Builder::new()
        .name("decklink-tray-rpc".into())
        .spawn(move || {
            let Ok(listener) = TcpListener::bind(TRAY_RPC_ADDR) else {
                return;
            };
            for stream in listener.incoming().flatten() {
                let mut line = String::new();
                if BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .is_err()
                {
                    continue;
                }
                let mut s = stream;
                match line.trim().to_ascii_uppercase().as_str() {
                    "SHOW" => {
                        in_tray.store(false, Ordering::SeqCst);
                        let ok = win_from_tray(&hwnd, &saved);
                        let _ = writeln!(s, "{}", if ok { "OK" } else { "ERR" });
                    }
                    "HIDE" => {
                        in_tray.store(true, Ordering::SeqCst);
                        let ok = win_to_tray(&hwnd, &saved);
                        let _ = writeln!(s, "{}", if ok { "OK" } else { "ERR" });
                    }
                    "QUIT" => {
                        let _ = writeln!(s, "OK");
                        let _ = s.flush();
                        hard_quit(&stop);
                    }
                    "PING" => {
                        let _ = writeln!(s, "PONG");
                    }
                    other => {
                        let _ = writeln!(s, "ERR {other}");
                    }
                }
            }
        })
        .ok();
}

pub fn run_ui(handle: HostHandle, tray_rpc: bool) -> Result<()> {
    let menu = Menu::new();
    let show_item = MenuItem::new("Show DeckLink Host", true, None);
    let quit_item = MenuItem::new("Quit", true, None);
    menu.append(&show_item)
        .map_err(|e| anyhow!("tray menu: {e}"))?;
    menu.append(&PredefinedMenuItem::separator())
        .map_err(|e| anyhow!("tray menu: {e}"))?;
    menu.append(&quit_item)
        .map_err(|e| anyhow!("tray menu: {e}"))?;

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("DeckLink Host — waiting for Deck")
        .with_icon(tray_icon_rgba())
        .build()
        .map_err(|e| anyhow!("tray icon: {e}"))?;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([480.0, 380.0])
            .with_title(WINDOW_TITLE)
            .with_resizable(false),
        centered: true,
        ..Default::default()
    };

    let show_id = show_item.id().clone();
    let quit_id = quit_item.id().clone();
    let hwnd = Arc::new(AtomicIsize::new(0));
    let saved_place: Arc<Mutex<Option<SavedPlace>>> = Arc::new(Mutex::new(None));
    let in_tray = Arc::new(AtomicBool::new(false));
    let handle_ui = handle.clone();

    if tray_rpc {
        spawn_rpc(
            hwnd.clone(),
            saved_place.clone(),
            in_tray.clone(),
            handle_ui.stop.clone(),
        );
    }

    let result = eframe::run_native(
        WINDOW_TITLE,
        options,
        Box::new(move |cc| {
            if let Ok(wh) = cc.window_handle() {
                if let RawWindowHandle::Win32(h) = wh.as_raw() {
                    hwnd.store(h.hwnd.get(), Ordering::SeqCst);
                }
            }

            let hwnd_m = hwnd.clone();
            let saved_m = saved_place.clone();
            let in_tray_m = in_tray.clone();
            let stop_m = handle_ui.stop.clone();
            MenuEvent::set_event_handler(Some(move |ev: MenuEvent| {
                if ev.id == quit_id {
                    hard_quit(&stop_m);
                }
                if ev.id == show_id {
                    in_tray_m.store(false, Ordering::SeqCst);
                    let _ = win_from_tray(&hwnd_m, &saved_m);
                }
            }));

            let hwnd_c = hwnd.clone();
            let saved_c = saved_place.clone();
            let in_tray_c = in_tray.clone();
            TrayIconEvent::set_event_handler(Some(move |ev: TrayIconEvent| {
                // Hover Enter/Move must NOT restore the window.
                let show = matches!(
                    ev,
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } | TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    }
                );
                if show {
                    in_tray_c.store(false, Ordering::SeqCst);
                    let _ = win_from_tray(&hwnd_c, &saved_c);
                }
            }));

            Ok(Box::new(HostApp {
                handle: handle_ui,
                tray: Some(tray),
                hwnd,
                saved_place,
                in_tray,
                last_tip: String::new(),
                last_gen: 0,
            }))
        }),
    );

    handle.request_stop();
    result.map_err(|e| anyhow!("UI: {e}"))
}

pub fn run_tray_self_test() -> Result<()> {
    use std::net::TcpStream;
    use std::process::{Command, Stdio};

    let exe = std::env::current_exe()?;
    let mut child = Command::new(&exe)
        .args([
            "--skip-vigem-install",
            "--tray-rpc",
            "--bind",
            "127.0.0.1:31417",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        if TcpStream::connect_timeout(
            &TRAY_RPC_ADDR.parse().unwrap(),
            Duration::from_millis(200),
        )
        .is_ok()
            && find_by_title() != 0
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    if find_by_title() == 0 {
        let _ = child.kill();
        bail!("self-test: window not found");
    }

    fn rpc(cmd: &str) -> Result<String> {
        let mut stream = TcpStream::connect_timeout(
            &TRAY_RPC_ADDR.parse().unwrap(),
            Duration::from_secs(3),
        )?;
        stream.set_read_timeout(Some(Duration::from_secs(3)))?;
        stream.write_all(format!("{cmd}\n").as_bytes())?;
        let mut resp = String::new();
        BufReader::new(stream).read_line(&mut resp)?;
        Ok(resp)
    }

    let r = rpc("HIDE")?;
    if !r.trim().eq_ignore_ascii_case("OK") {
        let _ = child.kill();
        bail!("self-test: HIDE failed: {r:?}");
    }
    std::thread::sleep(Duration::from_millis(500));
    // Off-screen park — still "visible" to Win32, but left < -1000.
    let h = find_by_title();
    if h != 0 {
        unsafe {
            let mut rc = RECT::default();
            if GetWindowRect(HWND(h as *mut _), &mut rc).is_ok() && rc.left > -1000 {
                let _ = child.kill();
                bail!("self-test: window still on-screen after HIDE ({},{})", rc.left, rc.top);
            }
        }
    }

    let r = rpc("SHOW")?;
    if !r.trim().eq_ignore_ascii_case("OK") {
        let _ = child.kill();
        bail!("self-test: SHOW failed: {r:?}");
    }
    std::thread::sleep(Duration::from_millis(500));
    let h = find_by_title();
    if h == 0 || !unsafe { IsWindowVisible(HWND(h as *mut _)).as_bool() } {
        let _ = child.kill();
        bail!("self-test: not visible after SHOW");
    }
    unsafe {
        let mut rc = RECT::default();
        if GetWindowRect(HWND(h as *mut _), &mut rc).is_ok() && rc.left < -1000 {
            let _ = child.kill();
            bail!("self-test: window still off-screen after SHOW");
        }
    }

    let _ = rpc("QUIT");
    let status = child.wait()?;
    if status.code().unwrap_or(1) != 0 {
        bail!("self-test: bad exit {status:?}");
    }
    Ok(())
}

fn tray_icon_rgba() -> tray_icon::Icon {
    let size = 32u32;
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    for i in (0..rgba.len()).step_by(4) {
        rgba[i] = 30;
        rgba[i + 1] = 140;
        rgba[i + 2] = 180;
        rgba[i + 3] = 255;
    }
    tray_icon::Icon::from_rgba(rgba, size, size).expect("icon")
}
