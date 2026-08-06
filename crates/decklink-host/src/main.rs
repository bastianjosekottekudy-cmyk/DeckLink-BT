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
}

fn main() -> Result<()> {
    // Allocate a console when launched as windows_subsystem app so --headless logs work.
    #[cfg(windows)]
    {
        if std::env::args().any(|a| a == "--headless") {
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
        if !cli.skip_vigem_install {
            if let Err(e) = vigem_setup::ensure_vigem() {
                error!("{e}");
                // Still start UI so user sees the error; server will also report vigem_ok=false.
            }
        }
        // Discovery needs inbound UDP 31415 — private GUI apps often get silently blocked.
        firewall::ensure_firewall_rule();

        let handle = server::spawn_host(cli.bind.clone(), cli.name.clone())?;
        if cli.headless {
            info!("headless host running — end the process to stop");
            let stop = Arc::new(AtomicBool::new(false));
            while !stop.load(Ordering::SeqCst) && !handle.stop.load(Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            handle.request_stop();
            Ok(())
        } else {
            ui::run_ui(handle, cli.name)
        }
    }
}
