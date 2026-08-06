//! Minimal host window + system tray (close hides to tray).

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use eframe::egui;
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    TrayIcon, TrayIconBuilder, TrayIconEvent,
};

use crate::server::HostHandle;

pub fn run_ui(handle: HostHandle, title: String) -> Result<()> {
    let show = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let quit = Arc::new(std::sync::atomic::AtomicBool::new(false));

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

    let show_id = item_show.id().clone();
    let quit_id = item_quit.id().clone();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([440.0, 300.0])
            .with_title(format!("DeckLink Host — {title}"))
            .with_resizable(false),
        ..Default::default()
    };

    let handle_ui = handle.clone();
    let show_ui = show.clone();
    let quit_ui = quit.clone();

    let ui_result = eframe::run_simple_native("DeckLink Host", options, move |ctx, _frame| {
        if let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == show_id {
                show_ui.store(true, std::sync::atomic::Ordering::SeqCst);
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            } else if event.id == quit_id {
                quit_ui.store(true, std::sync::atomic::Ordering::SeqCst);
                handle_ui.request_stop();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        while TrayIconEvent::receiver().try_recv().is_ok() {
            show_ui.store(true, std::sync::atomic::Ordering::SeqCst);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        }

        if quit_ui.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }

        ctx.input(|i| {
            if i.viewport().close_requested() {
                show_ui.store(false, std::sync::atomic::Ordering::SeqCst);
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
        });

        if !show_ui.load(std::sync::atomic::Ordering::SeqCst) {
            ctx.request_repaint_after(Duration::from_millis(500));
            return;
        }

        let st = handle_ui.status.lock().unwrap().clone();
        let tip = if let Some(ref p) = st.peer_name {
            format!("DeckLink Host — linked: {p}")
        } else if st.listening {
            "DeckLink Host — waiting for Deck".to_string()
        } else {
            "DeckLink Host — starting…".to_string()
        };
        let _ = tray.set_tooltip(Some(tip.as_str()));

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(8.0);
            ui.heading("DeckLink Host");
            ui.label("Steam Deck finds this PC automatically on Wi‑Fi.");
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
                    "ViGEmBus: not ready",
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
                quit_ui.store(true, std::sync::atomic::Ordering::SeqCst);
                handle_ui.request_stop();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });

        ctx.request_repaint_after(Duration::from_millis(250));
    });

    handle.request_stop();
    ui_result.map_err(|e| anyhow!("UI: {e}"))
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
