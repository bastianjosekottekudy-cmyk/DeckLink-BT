//! Minimal host window + system tray (close hides to tray; Quit exits).

use std::time::Duration;

use anyhow::{anyhow, Result};
use eframe::egui;
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    TrayIcon, TrayIconBuilder, TrayIconEvent,
};

use crate::server::HostHandle;

struct HostApp {
    handle: HostHandle,
    tray: TrayIcon,
    show_id: tray_icon::menu::MenuId,
    quit_id: tray_icon::menu::MenuId,
    /// Window should be visible (false = in tray only).
    visible: bool,
    /// User chose Quit — allow the window to actually close.
    quitting: bool,
    /// Apply Visible(false) on the next frame (never inside the input lock).
    hide_pending: bool,
    last_tip: String,
}

impl eframe::App for HostApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Color32::from_rgb(15, 20, 25).to_normalized_gamma_f32()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Tray menu / clicks
        if let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == self.show_id {
                self.visible = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            } else if event.id == self.quit_id {
                self.quitting = true;
                self.handle.request_stop();
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        while let Ok(_ev) = TrayIconEvent::receiver().try_recv() {
            self.visible = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        }

        // Close button → tray (must CancelClose or the process exits / crashes).
        let close_requested = ctx.input(|i| i.viewport().close_requested());
        if close_requested {
            if self.quitting {
                // Let the window close and end the event loop.
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.visible = false;
                self.hide_pending = true;
            }
        }

        if self.hide_pending {
            self.hide_pending = false;
            // Minimized + invisible is more reliable on Win32 than Visible alone.
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
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
            let _ = self.tray.set_tooltip(Some(tip.as_str()));
            self.last_tip = tip;
        }

        if !self.visible {
            // Stay in the event loop while hidden — do not tear down GL/window.
            ctx.request_repaint_after(Duration::from_millis(500));
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
                self.quitting = true;
                self.handle.request_stop();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });

        ctx.request_repaint_after(Duration::from_millis(250));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
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

    let app = HostApp {
        handle: handle.clone(),
        tray,
        show_id: item_show.id().clone(),
        quit_id: item_quit.id().clone(),
        visible: true,
        quitting: false,
        hide_pending: false,
        last_tip: String::new(),
    };

    let result = eframe::run_native(
        "DeckLink Host",
        options,
        Box::new(|_cc| Ok(Box::new(app))),
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
            // Simple teal square — no PNG decode at startup.
            rgba[i] = 30;
            rgba[i + 1] = 140;
            rgba[i + 2] = 180;
            rgba[i + 3] = 255;
        }
    }
    tray_icon::Icon::from_rgba(rgba, size, size).expect("icon")
}
