//! Ensure Windows Firewall allows DeckLink Host UDP inbound.

use std::process::Command;

use tracing::{info, warn};

use decklink_net::DEFAULT_PORT;

const RULE_NAME: &str = "DeckLink Host UDP";

/// Best-effort: allow inbound UDP on the DeckLink port for this exe.
pub fn ensure_firewall_rule() {
    if rule_exists() {
        info!("firewall rule already present ({RULE_NAME})");
        return;
    }
    let Some(exe) = std::env::current_exe().ok() else {
        warn!("cannot resolve exe path for firewall rule");
        return;
    };
    let exe_s = exe.display().to_string();

    // netsh is simpler to elevate than nested PowerShell quoting.
    let args = format!(
        "advfirewall firewall add rule name=\"{RULE_NAME}\" dir=in action=allow \
         protocol=UDP localport={DEFAULT_PORT} program=\"{exe_s}\" profile=any enable=yes"
    );

    if run_netsh(&args, false) {
        info!("firewall rule added (UDP {DEFAULT_PORT})");
        return;
    }
    warn!("firewall rule needs elevation — prompting UAC…");
    if run_netsh(&args, true) {
        info!("firewall rule added with elevation (UDP {DEFAULT_PORT})");
    } else {
        warn!(
            "could not add firewall rule — allow UDP {DEFAULT_PORT} inbound for DeckLink Host \
             in Windows Defender Firewall, or discovery will fail"
        );
    }
}

fn rule_exists() -> bool {
    let out = Command::new("netsh")
        .args(["advfirewall", "firewall", "show", "rule", &format!("name={RULE_NAME}")])
        .output();
    match out {
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout);
            o.status.success() && text.contains(RULE_NAME) && !text.contains("No rules match")
        }
        Err(_) => false,
    }
}

fn run_netsh(args: &str, elevate: bool) -> bool {
    if elevate {
        let ps = format!(
            "Start-Process -FilePath netsh.exe -ArgumentList '{}' -Verb RunAs -Wait",
            args.replace('\'', "''")
        );
        let status = Command::new("powershell")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &ps])
            .status();
        matches!(status, Ok(s) if s.success())
    } else {
        // Split carefully: netsh wants the full argument string as separate tokens is hard;
        // invoke via cmd /c for the non-elevated attempt.
        let status = Command::new("cmd")
            .args(["/C", &format!("netsh {args}")])
            .status();
        matches!(status, Ok(s) if s.success())
    }
}
