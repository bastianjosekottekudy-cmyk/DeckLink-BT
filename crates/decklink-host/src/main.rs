//! DeckLink Windows host — tray UI + UDP → ViGEm (auto-installs driver).

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use tracing::{error, info};

#[cfg(windows)]
mod firewall;
#[cfg(windows)]
mod inject;
#[cfg(windows)]
mod pad;
#[cfg(windows)]
mod server;
#[cfg(windows)]
mod ui;
#[cfg(windows)]
mod vigem_setup;

use decklink_net::DEFAULT_PORT;

#[derive(Parser, Debug)]
#[command(
    name = "decklink-host",
    version,
    about = "DeckLink Wi-Fi host for Windows (ViGEmBus + tray UI)"
)]
struct Cli {
    /// Bind address (UDP)
    #[arg(long, default_value_t = format!("0.0.0.0:{DEFAULT_PORT}"))]
    bind: String,

    /// Name shown on the Deck when discovered
    #[arg(long, default_value = "DeckLink Host")]
    name: String,

    /// No window/tray — console only
    #[arg(long)]
    headless: bool,

    /// Skip ViGEmBus auto-install (Xbox mode will fail without the driver)
    #[arg(long)]
    skip_vigem_install: bool,

    /// Listen on 127.0.0.1:31416 for SHOW/QUIT (test / automation)
    #[arg(long)]
    tray_rpc: bool,

    /// Run automated tray hide→show→quit test, then exit
    #[arg(long)]
    self_test_tray: bool,
}

fn main() -> Result<()> {
    #[cfg(windows)]
    {
        if std::env::args().any(|a| a == "--headless" || a == "--self-test-tray") {
            unsafe {
                let _ = windows::Win32::System::Console::AllocConsole();
            }
        }
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "decklink_host=info".into()),
        )
        .init();

    let cli = Cli::parse();

    #[cfg(not(windows))]
    {
        let _ = cli;
        anyhow::bail!("decklink-host is Windows-only (needs ViGEmBus)");
    }

    #[cfg(windows)]
    {
        if cli.self_test_tray {
            info!("running tray self-test…");
            ui::run_tray_self_test()?;
            info!("tray self-test OK");
            return Ok(());
        }

        let handle = server::spawn_host(cli.bind.clone(), cli.name.clone())?;

        let skip_vigem = cli.skip_vigem_install;
        let status = handle.status.clone();
        std::thread::Builder::new()
            .name("decklink-setup".into())
            .spawn(move || {
                if !skip_vigem {
                    if let Err(e) = vigem_setup::ensure_vigem() {
                        error!("{e}");
                        let mut s = status.lock().unwrap();
                        s.last_error = Some(e.to_string());
                        s.vigem_ok = vigem_setup::vigem_available();
                    } else if vigem_setup::vigem_available() {
                        status.lock().unwrap().vigem_ok = true;
                    }
                }
                firewall::ensure_firewall_rule();
            })
            .ok();

        if cli.headless {
            info!("headless host running — end the process to stop");
            let stop = Arc::new(AtomicBool::new(false));
            while !stop.load(Ordering::SeqCst) && !handle.stop.load(Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            handle.request_stop();
            Ok(())
        } else {
            ui::run_ui(handle, cli.tray_rpc)
        }
    }
}
