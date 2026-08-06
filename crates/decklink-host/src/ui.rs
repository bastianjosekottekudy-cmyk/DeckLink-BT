//! Host window + system tray.
//!
//! On Windows, eframe's `ViewportCommand::Visible(false)` is unsafe/broken (event loop
//! stalls or the process dies). We hide with Win32 `ShowWindow(SW_HIDE)` instead and
//! keep the egui window "alive" for the event loop.

use std::time::Duration;

use anyhow::{anyhow, Result};
use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    TrayIcon, TrayIconBuilder, TrayIconEvent,
};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    SetForegroundWindow, ShowWindow, SW_HIDE, SW_RESTORE, SW_SHOW,
};

use crate::server::HostHandle;

struct HostApp {
    handle: HostHandle,
    tray: Option<TrayIcon>,
    show_id: tray_icon::menu::MenuId,
    quit_id: tray_icon::menu::MenuId,
    hwnd: Option<HWND>,
    /// Logical "in tray" state (Win32 window may be SW_HIDE).
    in_tray: bool,
    quitting: bool,
    last_tip: String,
}

impl HostApp {
    fn capture_hwnd(&mut self, frame: &eframe::Frame) {
        if self.hwnd.is_some() {
            return;
        }
        if let Ok(wh) = frame.window_handle() {
            if let RawWindowHandle::Win32(h) = wh.as_raw() {
                self.hwnd = Some(HWND(h.hwnd.get() as *mut _));
            }
        }
    }

    fn win_hide(&self) {
        if let Some(hwnd) = self.hwnd {
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }
    }

    fn win_show(&self) {
        if let Some(hwnd) = self.hwnd {
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = ShowWindow(hwnd, SW_RESTORE);
                let _ = SetForegroundWindow(hwnd);
            }
        }
    }

    fn begin_quit(&mut self, ctx: &egui::Context) {
        self.quitting = true;
        self.handle.request_stop();
        // Drop tray before tearing down the GL window — avoids Win32 tray crashes.
        self.tray.take();
        self.win_show();
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

impl eframe::App for HostApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Color32::from_rgb(15, 20, 25).to_normalized_gamma_f32()
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.capture_hwnd(frame);

        // Tray menu
        if let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == self.show_id {
                self.in_tray = false;
                self.win_show();
            } else if event.id == self.quit_id {
                self.begin_quit(ctx);
                return;
            }
        }

        // Tray icon click → show
        while TrayIconEvent::receiver().try_recv().is_ok() {
            self.in_tray = false;
            self.win_show();
        }

        // X button → hide to tray (never destroy the window).
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.quitting {
                return;
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.in_tray = true;
            self.win_hide();
        }

        if self.quitting {
            return;
        }

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
            if let Some(tray) = self.tray.as_ref() {
                let _ = tray.set_tooltip(Some(tip.as_str()));
            }
            self.last_tip = tip;
        }

        if self.in_tray {
            // Light pulse so we keep polling tray events; Win32 hide keeps winit alive.
            ctx.request_repaint_after(Duration::from_millis(400));
            return;
        }

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
            if st.vigem_ok {
                ui.colored_label(
                    egui::Color32::from_rgb(61, 214, 140),
                    "ViGEmBus: installed (Xbox pad ready)",
                );
            } else {
                ui.colored_label(
                    egui::Color32::from_rgb(240, 113, 120),
                    "ViGEmBus: not ready (installing in background…)",
                );
            }

            ui.add_space(8.0);
            if let Some(ref n) = st.peer_name {
                ui.label(format!("Deck: {n}"));
                if let Some(ref a) = st.peer {
                    ui.label(format!("from {a}"));
                }
            } else {
                ui.label("No Deck connected yet.");
                ui.label("On the Deck: open DeckLink → Connect.");
            }

            if let Some(ref e) = st.last_error {
                ui.add_space(8.0);
                ui.colored_label(egui::Color32::from_rgb(240, 113, 120), e);
            }

            ui.add_space(14.0);
            ui.label("Close this window to keep running in the system tray.");
            if ui.button("Quit").clicked() {
                self.begin_quit(ctx);
            }
        });

        ctx.request_repaint_after(Duration::from_millis(250));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.tray.take();
        self.handle.request_stop();
    }
}

pub fn run_ui(handle: HostHandle, title: String) -> Result<()> {
    let tray_menu = Menu::new();
    let item_show = MenuItem::new("Show DeckLink Host", true, None);
    let item_quit = MenuItem::new("Quit", true, None);
    tray_menu
        .append(&item_show)
        .map_err(|e| anyhow!("tray menu: {e}"))?;
    tray_menu
        .append(&PredefinedMenuItem::separator())
        .map_err(|e| anyhow!("tray menu: {e}"))?;
    tray_menu
        .append(&item_quit)
        .map_err(|e| anyhow!("tray menu: {e}"))?;

    let icon = tray_icon_rgba();
    let tray: TrayIcon = TrayIconBuilder::new()
        .with_menu(Box::new(tray_menu))
        .with_tooltip("DeckLink Host — waiting for Deck")
        .with_icon(icon)
        .build()
        .map_err(|e| anyhow!("tray icon: {e}"))?;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([460.0, 320.0])
            .with_title(format!("DeckLink Host — {title}"))
            .with_resizable(false)
            .with_minimize_button(true)
            .with_close_button(true),
        centered: true,
        ..Default::default()
    };

    let show_id = item_show.id().clone();
    let quit_id = item_quit.id().clone();
    let handle_ui = handle.clone();

    let result = eframe::run_native(
        "DeckLink Host",
        options,
        Box::new(move |cc| {
            // Wake egui when the tray is used while the window is hidden.
            let ctx_menu = cc.egui_ctx.clone();
            MenuEvent::set_event_handler(Some(move |_ev| {
                ctx_menu.request_repaint();
            }));
            let ctx_tray = cc.egui_ctx.clone();
            TrayIconEvent::set_event_handler(Some(move |_ev| {
                ctx_tray.request_repaint();
            }));

            let mut hwnd = None;
            if let Ok(wh) = cc.window_handle() {
                if let RawWindowHandle::Win32(h) = wh.as_raw() {
                    hwnd = Some(HWND(h.hwnd.get() as *mut _));
                }
            }

            Ok(Box::new(HostApp {
                handle: handle_ui,
                tray: Some(tray),
                show_id,
                quit_id,
                hwnd,
                in_tray: false,
                quitting: false,
                last_tip: String::new(),
            }))
        }),
    );

    handle.request_stop();
    result.map_err(|e| anyhow!("UI: {e}"))
}

fn tray_icon_rgba() -> tray_icon::Icon {
    let size = 32u32;
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let i = ((y * size + x) * 4) as usize;
            rgba[i] = 30;
            rgba[i + 1] = 140;
            rgba[i + 2] = 180;
            rgba[i + 3] = 255;
        }
    }
    tray_icon::Icon::from_rgba(rgba, size, size).expect("icon")
}
