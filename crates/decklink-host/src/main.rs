//! DeckLink Windows host — UDP → ViGEm Xbox 360 + SendInput mouse/keyboard.

use std::collections::HashSet;
use std::net::UdpSocket;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use tracing::{error, info, warn};

use decklink_hid::{
    GamepadReport, KeyModifiers, KeyboardReport, MouseButtons, MouseReport, GAMEPAD_REPORT_ID,
    KEYBOARD_REPORT_ID, MOUSE_REPORT_ID,
};
use decklink_net::{decode, decode_hid, encode, MsgKind, MAX_PACKET, DEFAULT_PORT};

#[cfg(windows)]
mod inject;
#[cfg(windows)]
mod pad;

#[derive(Parser, Debug)]
#[command(
    name = "decklink-host",
    version,
    about = "DeckLink Wi-Fi host for Windows (ViGEmBus required)"
)]
struct Cli {
    /// Bind address (UDP)
    #[arg(long, default_value_t = format!("0.0.0.0:{DEFAULT_PORT}"))]
    bind: String,

    /// Name sent in HelloAck
    #[arg(long, default_value = "DeckLink Host")]
    name: String,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "decklink_host=info".into()),
        )
        .init();

    let cli = Cli::parse();

    #[cfg(not(windows))]
    {
        anyhow::bail!("decklink-host is Windows-only (needs ViGEmBus)");
    }

    #[cfg(windows)]
    {
        run_windows(cli)
    }
}

#[cfg(windows)]
fn run_windows(cli: Cli) -> Result<()> {
    info!("binding UDP {}", cli.bind);
    let sock = UdpSocket::bind(&cli.bind).with_context(|| format!("bind {}", cli.bind))?;
    sock.set_read_timeout(Some(Duration::from_millis(50)))?;

    let mut pad = pad::VirtualPad::new().context(
        "ViGEmBus not available — install https://github.com/ViGEm/ViGEmBus/releases then retry",
    )?;
    let mut inject = inject::Injector::new();

    info!(
        "listening on {} — on the Deck, Connect to this PC's LAN IP (port {DEFAULT_PORT})",
        cli.bind
    );

    let mut buf = [0u8; MAX_PACKET];
    let mut peer: Option<(std::net::SocketAddr, Instant)> = None;
    let mut seq = 1u32;
    let mut last_keys: HashSet<u8> = HashSet::new();
    let mut last_mods = KeyModifiers::empty();
    let mut last_mouse_btns = MouseButtons::empty();

    loop {
        match sock.recv_from(&mut buf) {
            Ok((n, addr)) => {
                let env = match decode(&buf[..n]) {
                    Ok(e) => e,
                    Err(e) => {
                        warn!("bad packet from {addr}: {e}");
                        continue;
                    }
                };

                match env.kind {
                    MsgKind::Hello => {
                        let deck_name = String::from_utf8_lossy(&env.payload).into_owned();
                        info!("Deck hello from {addr} ({deck_name})");
                        let ack = encode(MsgKind::HelloAck, seq, cli.name.as_bytes())?;
                        seq = seq.wrapping_add(1);
                        sock.send_to(&ack, addr)?;
                        peer = Some((addr, Instant::now()));
                        last_keys.clear();
                        last_mods = KeyModifiers::empty();
                        last_mouse_btns = MouseButtons::empty();
                        pad.reset();
                        inject.release_all();
                    }
                    MsgKind::Heartbeat => {
                        if let Some((p, t)) = peer.as_mut() {
                            if *p == addr {
                                *t = Instant::now();
                            }
                        }
                        let hb = encode(MsgKind::Heartbeat, seq, &[])?;
                        seq = seq.wrapping_add(1);
                        let _ = sock.send_to(&hb, addr);
                    }
                    MsgKind::Goodbye => {
                        info!("Deck goodbye from {addr}");
                        if peer.map(|(p, _)| p) == Some(addr) {
                            peer = None;
                            pad.reset();
                            inject.release_all();
                            last_keys.clear();
                        }
                    }
                    MsgKind::Hid => {
                        if let Some((p, t)) = peer.as_mut() {
                            if *p != addr {
                                continue;
                            }
                            *t = Instant::now();
                        } else {
                            // Accept HID before hello only if we got traffic — require hello.
                            continue;
                        }
                        let pkt = match decode_hid(&env.payload) {
                            Ok(p) => p,
                            Err(e) => {
                                warn!("hid: {e}");
                                continue;
                            }
                        };
                        match pkt.report_id {
                            GAMEPAD_REPORT_ID => {
                                if let Some(r) = GamepadReport::unpack(&pkt.data) {
                                    pad.update(&r);
                                }
                            }
                            MOUSE_REPORT_ID => {
                                if let Some(r) = MouseReport::unpack(&pkt.data) {
                                    inject.apply_mouse(&r, &mut last_mouse_btns);
                                }
                            }
                            KEYBOARD_REPORT_ID => {
                                if let Some(r) = KeyboardReport::unpack(&pkt.data) {
                                    inject.apply_keyboard(
                                        &r,
                                        &mut last_keys,
                                        &mut last_mods,
                                    );
                                }
                            }
                            other => warn!("ignored report id {other}"),
                        }
                    }
                    MsgKind::HelloAck => {}
                }
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                if let Some((addr, t)) = peer {
                    if t.elapsed() > Duration::from_secs(5) {
                        warn!("Deck {addr} timed out");
                        peer = None;
                        pad.reset();
                        inject.release_all();
                        last_keys.clear();
                    }
                }
            }
            Err(e) => {
                error!("recv: {e}");
                return Err(e.into());
            }
        }
    }
}