//! List local IPv4 addresses for the host UI (Deck Connect / manual IP).

use std::net::Ipv4Addr;

/// Returns `(interface_name, ipv4)` — non-loopback first, link-local last.
pub fn list_lan_ipv4() -> Vec<(String, Ipv4Addr)> {
    let Ok(ifaces) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    let mut primary = Vec::new();
    let mut link_local = Vec::new();
    for iface in ifaces {
        if iface.is_loopback() {
            continue;
        }
        let if_addrs::IfAddr::V4(v4) = iface.addr else {
            continue;
        };
        let ip = v4.ip;
        let row = (iface.name, ip);
        if ip.is_link_local() {
            link_local.push(row);
        } else {
            primary.push(row);
        }
    }
    primary.extend(link_local);
    primary
}

pub fn format_lan_ips() -> Vec<String> {
    list_lan_ipv4()
        .into_iter()
        .map(|(name, ip)| format!("{name}: {ip}"))
        .collect()
}
