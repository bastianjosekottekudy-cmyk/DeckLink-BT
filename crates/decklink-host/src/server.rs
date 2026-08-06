//! UDP host loop: discovery + HID → ViGEm / SendInput.

use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tracing::{info, warn};

use decklink_hid::{
    GamepadReport, KeyModifiers, KeyboardReport, MouseButtons, MouseReport, GAMEPAD_REPORT_ID,
    KEYBOARD_REPORT_ID, MOUSE_REPORT_ID,
};
use decklink_net::{
    decode, decode_hid, encode, MsgKind, DEFAULT_PORT, MAX_PACKET, MULTICAST_ADDR,
};

use crate::inject::Injector;
use crate::pad::VirtualPad;

#[derive(Debug, Clone, Default)]
pub struct HostStatus {
    pub listening: bool,
    pub bind: String,
    pub peer: Option<String>,
    pub peer_name: Option<String>,
    pub last_error: Option<String>,
    pub vigem_ok: bool,
}

#[derive(Clone)]
pub struct HostHandle {
    pub status: Arc<Mutex<HostStatus>>,
    /// Bumped whenever UI-visible status fields change (wakes idle UI cheaply).
    pub status_gen: Arc<AtomicU64>,
    pub stop: Arc<AtomicBool>,
}

impl HostHandle {
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn bump(gen: &AtomicU64) {
    gen.fetch_add(1, Ordering::Relaxed);
}

pub fn spawn_host(bind: String, name: String) -> Result<HostHandle> {
    let status = Arc::new(Mutex::new(HostStatus {
        bind: bind.clone(),
        ..Default::default()
    }));
    let status_gen = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let status_bg = status.clone();
    let gen_bg = status_gen.clone();
    let stop_bg = stop.clone();

    std::thread::Builder::new()
        .name("decklink-udp".into())
        .spawn(move || {
            if let Err(e) = run_loop(bind, name, status_bg.clone(), gen_bg.clone(), stop_bg) {
                let mut s = status_bg.lock().unwrap();
                s.last_error = Some(e.to_string());
                s.listening = false;
                bump(&gen_bg);
                warn!("host loop ended: {e}");
            }
        })?;

    Ok(HostHandle {
        status,
        status_gen,
        stop,
    })
}

fn run_loop(
    bind: String,
    name: String,
    status: Arc<Mutex<HostStatus>>,
    status_gen: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
) -> Result<()> {
    info!("binding UDP {bind}");
    let sock = UdpSocket::bind(&bind).with_context(|| format!("bind {bind}"))?;
    sock.set_broadcast(true)?;
    // Idle: longer timeout (fewer wakeups). Fine for HID — Deck sends often when linked.
    sock.set_read_timeout(Some(Duration::from_millis(200)))?;
    match sock.join_multicast_v4(&MULTICAST_ADDR, &Ipv4Addr::UNSPECIFIED) {
        Ok(()) => info!("joined multicast {MULTICAST_ADDR}:{DEFAULT_PORT}"),
        Err(e) => warn!("multicast join failed ({e}) — broadcast-only discovery"),
    }
    let _ = sock.set_multicast_loop_v4(false);

    let mut pad = match VirtualPad::new() {
        Ok(p) => {
            status.lock().unwrap().vigem_ok = true;
            bump(&status_gen);
            info!("ViGEm Xbox pad ready");
            Some(p)
        }
        Err(e) => {
            let mut s = status.lock().unwrap();
            s.vigem_ok = false;
            s.last_error = Some(format!(
                "ViGEmBus starting… ({e}). Xbox mode waits; mouse/keyboard work."
            ));
            bump(&status_gen);
            warn!("ViGEm unavailable at bind — will retry: {e}");
            None
        }
    };
    let mut inject = Injector::new();
    let mut last_vigem_retry = Instant::now()
        .checked_sub(Duration::from_secs(10))
        .unwrap_or_else(Instant::now);

    {
        let mut s = status.lock().unwrap();
        s.listening = true;
        bump(&status_gen);
    }
    info!("listening — Deck Connect will find this PC automatically");

    let mut buf = [0u8; MAX_PACKET];
    let mut peer: Option<(SocketAddr, Instant)> = None;
    let mut seq = 1u32;
    let mut last_keys: HashSet<u8> = HashSet::new();
    let mut last_mods = KeyModifiers::empty();
    let mut last_mouse_btns = MouseButtons::empty();
    let mut last_announce = Instant::now()
        .checked_sub(Duration::from_secs(10))
        .unwrap_or_else(Instant::now);

    while !stop.load(Ordering::SeqCst) {
        if pad.is_none() && last_vigem_retry.elapsed() > Duration::from_secs(2) {
            last_vigem_retry = Instant::now();
            match VirtualPad::new() {
                Ok(p) => {
                    info!("ViGEm Xbox pad ready (delayed)");
                    let mut s = status.lock().unwrap();
                    s.vigem_ok = true;
                    if s.last_error
                        .as_deref()
                        .is_some_and(|e| e.contains("ViGEm"))
                    {
                        s.last_error = None;
                    }
                    bump(&status_gen);
                    pad = Some(p);
                }
                Err(_) => {}
            }
        }

        let announce_every = if peer.is_some() {
            Duration::from_secs(5)
        } else {
            Duration::from_secs(3)
        };
        if last_announce.elapsed() > announce_every {
            let _ = broadcast_announce(&sock, &name, &mut seq);
            last_announce = Instant::now();
        }

        if let Some((addr, t)) = peer {
            if t.elapsed() > Duration::from_secs(5) {
                warn!("Deck {addr} timed out");
                drop_peer(
                    &mut peer,
                    &mut pad,
                    &mut inject,
                    &mut last_keys,
                    &mut last_mods,
                    &mut last_mouse_btns,
                    &status,
                    &status_gen,
                );
            }
        }

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
                    MsgKind::Discover => {
                        let ack = encode(MsgKind::Announce, seq, name.as_bytes())?;
                        seq = seq.wrapping_add(1);
                        let _ = sock.send_to(&ack, addr);
                        info!("answered Discover from {addr}");
                    }
                    MsgKind::Hello => {
                        let deck_name = String::from_utf8_lossy(&env.payload).into_owned();
                        info!("Deck hello from {addr} ({deck_name})");
                        let ack = encode(MsgKind::HelloAck, seq, name.as_bytes())?;
                        seq = seq.wrapping_add(1);
                        sock.send_to(&ack, addr)?;
                        drop_peer(
                            &mut peer,
                            &mut pad,
                            &mut inject,
                            &mut last_keys,
                            &mut last_mods,
                            &mut last_mouse_btns,
                            &status,
                            &status_gen,
                        );
                        peer = Some((addr, Instant::now()));
                        let mut s = status.lock().unwrap();
                        s.peer = Some(addr.to_string());
                        s.peer_name = Some(deck_name);
                        bump(&status_gen);
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
                            drop_peer(
                                &mut peer,
                                &mut pad,
                                &mut inject,
                                &mut last_keys,
                                &mut last_mods,
                                &mut last_mouse_btns,
                                &status,
                                &status_gen,
                            );
                        }
                    }
                    MsgKind::Hid => {
                        if let Some((p, t)) = peer.as_mut() {
                            if *p != addr {
                                continue;
                            }
                            *t = Instant::now();
                        } else {
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
                                    if let Some(pad) = pad.as_mut() {
                                        pad.update(&r);
                                    }
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
                    MsgKind::HelloAck | MsgKind::Announce => {}
                }
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => return Err(e.into()),
        }
    }

    drop_peer(
        &mut peer,
        &mut pad,
        &mut inject,
        &mut last_keys,
        &mut last_mods,
        &mut last_mouse_btns,
        &status,
        &status_gen,
    );
    status.lock().unwrap().listening = false;
    bump(&status_gen);
    Ok(())
}

fn broadcast_announce(sock: &UdpSocket, name: &str, seq: &mut u32) -> Result<()> {
    let pkt = encode(MsgKind::Announce, *seq, name.as_bytes())?;
    *seq = seq.wrapping_add(1);
    let multi = SocketAddr::V4(SocketAddrV4::new(MULTICAST_ADDR, DEFAULT_PORT));
    let _ = sock.send_to(&pkt, multi);
    let bcast = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::BROADCAST, DEFAULT_PORT));
    let _ = sock.send_to(&pkt, bcast);
    Ok(())
}

fn drop_peer(
    peer: &mut Option<(SocketAddr, Instant)>,
    pad: &mut Option<VirtualPad>,
    inject: &mut Injector,
    last_keys: &mut HashSet<u8>,
    last_mods: &mut KeyModifiers,
    last_mouse_btns: &mut MouseButtons,
    status: &Arc<Mutex<HostStatus>>,
    status_gen: &AtomicU64,
) {
    *peer = None;
    if let Some(pad) = pad.as_mut() {
        pad.reset();
    }
    inject.reset(last_keys, last_mods, last_mouse_btns);
    let mut s = status.lock().unwrap();
    s.peer = None;
    s.peer_name = None;
    bump(status_gen);
}
