//! DeckLink Wi-Fi UDP protocol: Deck client ↔ PC host.
//!
//! Framing (little-endian):
//! ```text
//! magic u32 = 0x444C4E4B ("DLNK")
//! version u8 = 1
//! kind u8
//! seq u32
//! payload_len u16
//! payload [u8; payload_len]
//! ```

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::time::{Duration, Instant};

use decklink_hid::HidPacket;
use thiserror::Error;
use tracing::{debug, info, warn};

pub const MAGIC: u32 = 0x444C_4E4B; // DLNK
pub const PROTOCOL_VERSION: u8 = 1;
pub const DEFAULT_PORT: u16 = 31415;
pub const HEADER_LEN: usize = 12;
pub const MAX_PAYLOAD: usize = 240;
pub const MAX_PACKET: usize = HEADER_LEN + MAX_PAYLOAD;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgKind {
    Hello = 1,
    HelloAck = 2,
    Heartbeat = 3,
    Hid = 4,
    Goodbye = 5,
    /// LAN broadcast from Deck looking for hosts.
    Discover = 6,
    /// Unicast/broadcast reply from host (payload = display name).
    Announce = 7,
}

impl MsgKind {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Hello),
            2 => Some(Self::HelloAck),
            3 => Some(Self::Heartbeat),
            4 => Some(Self::Hid),
            5 => Some(Self::Goodbye),
            6 => Some(Self::Discover),
            7 => Some(Self::Announce),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Envelope {
    pub kind: MsgKind,
    pub seq: u32,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct DiscoveredHost {
    pub addr: SocketAddr,
    pub name: String,
}

#[derive(Debug, Error)]
pub enum NetError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("bad magic")]
    BadMagic,
    #[error("unsupported protocol version {0}")]
    BadVersion(u8),
    #[error("unknown message kind {0}")]
    BadKind(u8),
    #[error("truncated / short packet")]
    Truncated,
    #[error("payload too large")]
    TooLarge,
    #[error("not connected")]
    NotConnected,
    #[error("hello timeout")]
    HelloTimeout,
    #[error("no DeckLink host found on the LAN — is decklink-host running on the PC?")]
    NoHostsFound,
}

pub fn encode(kind: MsgKind, seq: u32, payload: &[u8]) -> Result<Vec<u8>, NetError> {
    if payload.len() > MAX_PAYLOAD {
        return Err(NetError::TooLarge);
    }
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
    out.extend_from_slice(&MAGIC.to_le_bytes());
    out.push(PROTOCOL_VERSION);
    out.push(kind as u8);
    out.extend_from_slice(&seq.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

pub fn decode(buf: &[u8]) -> Result<Envelope, NetError> {
    if buf.len() < HEADER_LEN {
        return Err(NetError::Truncated);
    }
    let magic = u32::from_le_bytes(buf[0..4].try_into().unwrap());
    if magic != MAGIC {
        return Err(NetError::BadMagic);
    }
    let version = buf[4];
    if version != PROTOCOL_VERSION {
        return Err(NetError::BadVersion(version));
    }
    let kind = MsgKind::from_u8(buf[5]).ok_or(NetError::BadKind(buf[5]))?;
    let seq = u32::from_le_bytes(buf[6..10].try_into().unwrap());
    let plen = u16::from_le_bytes(buf[10..12].try_into().unwrap()) as usize;
    if buf.len() < HEADER_LEN + plen {
        return Err(NetError::Truncated);
    }
    if plen > MAX_PAYLOAD {
        return Err(NetError::TooLarge);
    }
    Ok(Envelope {
        kind,
        seq,
        payload: buf[HEADER_LEN..HEADER_LEN + plen].to_vec(),
    })
}

pub fn encode_hid(seq: u32, pkt: &HidPacket) -> Result<Vec<u8>, NetError> {
    let mut payload = Vec::with_capacity(1 + pkt.data.len());
    payload.push(pkt.report_id);
    payload.extend_from_slice(&pkt.data);
    encode(MsgKind::Hid, seq, &payload)
}

pub fn decode_hid(payload: &[u8]) -> Result<HidPacket, NetError> {
    if payload.is_empty() {
        return Err(NetError::Truncated);
    }
    Ok(HidPacket {
        report_id: payload[0],
        data: payload[1..].to_vec(),
    })
}

pub fn default_bind_addr() -> String {
    format!("0.0.0.0:{DEFAULT_PORT}")
}

pub fn parse_host_addr(host: &str) -> Result<SocketAddr, NetError> {
    let host = host.trim();
    if host.is_empty() {
        return Err(NetError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "empty host",
        )));
    }
    if let Ok(addr) = host.parse::<SocketAddr>() {
        return Ok(addr);
    }
    if host.starts_with('[') {
        if let Some(end) = host.find(']') {
            let ip = &host[1..end];
            let rest = &host[end + 1..];
            if rest.is_empty() {
                let ip: std::net::IpAddr = ip.parse().map_err(|e| {
                    NetError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, e))
                })?;
                return Ok(SocketAddr::new(ip, DEFAULT_PORT));
            }
        }
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return Ok(SocketAddr::new(ip, DEFAULT_PORT));
    }
    format!("{host}:{DEFAULT_PORT}").parse().map_err(|e| {
        NetError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, e))
    })
}

/// Broadcast Discover and collect Announce replies for `timeout`.
pub fn discover_hosts(timeout: Duration) -> Result<Vec<DiscoveredHost>, NetError> {
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.set_broadcast(true)?;
    sock.set_read_timeout(Some(Duration::from_millis(200)))?;

    let discover = encode(MsgKind::Discover, 1, &[])?;
    let bcast = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::BROADCAST, DEFAULT_PORT));
    sock.send_to(&discover, bcast)?;

    let mut found: Vec<DiscoveredHost> = Vec::new();
    let deadline = Instant::now() + timeout;
    let mut buf = [0u8; MAX_PACKET];
    while Instant::now() < deadline {
        match sock.recv_from(&mut buf) {
            Ok((n, addr)) => {
                if let Ok(env) = decode(&buf[..n]) {
                    if env.kind == MsgKind::Announce {
                        let name = String::from_utf8_lossy(&env.payload).into_owned();
                        let name = if name.trim().is_empty() {
                            "DeckLink Host".into()
                        } else {
                            name
                        };
                        // Reply comes from host's data port.
                        let host = SocketAddr::new(addr.ip(), DEFAULT_PORT);
                        if !found.iter().any(|h| h.addr == host) {
                            info!("discovered {name} @ {host}");
                            found.push(DiscoveredHost { addr: host, name });
                        }
                    }
                }
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                // Re-broadcast once mid-wait.
                if Instant::now() + timeout / 2 < deadline {
                    let _ = sock.send_to(&discover, bcast);
                }
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(found)
}

/// Deck-side UDP client: hello → ack, then stream HID frames.
pub struct NetClient {
    sock: UdpSocket,
    peer: SocketAddr,
    seq: u32,
    pub peer_name: String,
    last_rx: Instant,
}

impl NetClient {
    /// Find a host on the LAN and complete Hello.
    pub fn connect_auto(device_name: &str) -> Result<Self, NetError> {
        let hosts = discover_hosts(Duration::from_secs(2))?;
        let Some(host) = hosts.into_iter().next() else {
            return Err(NetError::NoHostsFound);
        };
        info!("auto-connect → {} ({})", host.name, host.addr);
        Self::connect(&host.addr.to_string(), device_name)
    }

    pub fn connect(host: &str, device_name: &str) -> Result<Self, NetError> {
        let peer = parse_host_addr(host)?;

        let sock = UdpSocket::bind("0.0.0.0:0")?;
        sock.set_read_timeout(Some(Duration::from_millis(800)))?;
        sock.connect(peer)?;

        let mut seq = 1u32;
        let hello = encode(MsgKind::Hello, seq, device_name.as_bytes())?;
        sock.send(&hello)?;
        seq = seq.wrapping_add(1);

        let mut buf = [0u8; MAX_PACKET];
        let deadline = Instant::now() + Duration::from_secs(3);
        let peer_name = loop {
            if Instant::now() > deadline {
                return Err(NetError::HelloTimeout);
            }
            match sock.recv(&mut buf) {
                Ok(n) => match decode(&buf[..n]) {
                    Ok(env) if env.kind == MsgKind::HelloAck => {
                        let name = String::from_utf8_lossy(&env.payload).into_owned();
                        info!("Wi-Fi linked to {name} @ {peer}");
                        break name;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        warn!("hello decode noise: {e}");
                    }
                },
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    sock.send(&hello)?;
                    continue;
                }
                Err(e) => return Err(e.into()),
            }
        };

        sock.set_nonblocking(true)?;
        Ok(Self {
            sock,
            peer,
            seq,
            peer_name,
            last_rx: Instant::now(),
        })
    }

    pub fn peer_addr(&self) -> SocketAddr {
        self.peer
    }

    pub fn send_hid(&mut self, pkt: &HidPacket) -> Result<(), NetError> {
        let bytes = encode_hid(self.seq, pkt)?;
        self.seq = self.seq.wrapping_add(1);
        self.sock.send(&bytes)?;
        Ok(())
    }

    pub fn send_heartbeat(&mut self) -> Result<(), NetError> {
        let bytes = encode(MsgKind::Heartbeat, self.seq, &[])?;
        self.seq = self.seq.wrapping_add(1);
        self.sock.send(&bytes)?;
        Ok(())
    }

    pub fn send_goodbye(&mut self) -> Result<(), NetError> {
        let bytes = encode(MsgKind::Goodbye, self.seq, &[])?;
        self.seq = self.seq.wrapping_add(1);
        let _ = self.sock.send(&bytes);
        Ok(())
    }

    /// Drain inbound packets; returns true if still linked.
    pub fn poll(&mut self) -> Result<bool, NetError> {
        let mut buf = [0u8; MAX_PACKET];
        loop {
            match self.sock.recv(&mut buf) {
                Ok(n) => match decode(&buf[..n]) {
                    Ok(env) => {
                        self.last_rx = Instant::now();
                        match env.kind {
                            MsgKind::Goodbye => {
                                warn!("host sent goodbye");
                                return Ok(false);
                            }
                            MsgKind::Heartbeat | MsgKind::HelloAck | MsgKind::Announce => {}
                            other => debug!("client ignored {other:?}"),
                        }
                    }
                    Err(e) => warn!("decode: {e}"),
                },
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e.into()),
            }
        }
        if self.last_rx.elapsed() > Duration::from_secs(5) {
            warn!("host silent >5s");
            return Ok(false);
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_hid() {
        let pkt = HidPacket {
            report_id: 1,
            data: vec![1, 2, 3, 4],
        };
        let bytes = encode_hid(42, &pkt).unwrap();
        let env = decode(&bytes).unwrap();
        assert_eq!(env.kind, MsgKind::Hid);
        assert_eq!(env.seq, 42);
        let got = decode_hid(&env.payload).unwrap();
        assert_eq!(got.report_id, 1);
        assert_eq!(got.data, vec![1, 2, 3, 4]);
    }

    #[test]
    fn parse_host_ipv4_and_port() {
        assert_eq!(
            parse_host_addr("192.168.1.10").unwrap(),
            "192.168.1.10:31415".parse().unwrap()
        );
        assert_eq!(
            parse_host_addr("192.168.1.10:4000").unwrap(),
            "192.168.1.10:4000".parse().unwrap()
        );
    }

    #[test]
    fn parse_host_ipv6() {
        assert_eq!(
            parse_host_addr("::1").unwrap(),
            "[::1]:31415".parse().unwrap()
        );
        assert_eq!(
            parse_host_addr("[::1]:4000").unwrap(),
            "[::1]:4000".parse().unwrap()
        );
    }

    #[test]
    fn discover_announce_roundtrip_kinds() {
        let d = encode(MsgKind::Discover, 1, &[]).unwrap();
        assert_eq!(decode(&d).unwrap().kind, MsgKind::Discover);
        let a = encode(MsgKind::Announce, 2, b"PC").unwrap();
        let env = decode(&a).unwrap();
        assert_eq!(env.kind, MsgKind::Announce);
        assert_eq!(env.payload, b"PC");
    }
}
