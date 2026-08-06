//! Ensure Windows Firewall allows DeckLink Host UDP inbound.
//!
//! Windows often creates **Block** rules when the first firewall prompt is dismissed.
//! Those stick to a specific exe path and silently kill Deck discovery. We:
//!   1. Remove Block rules for any `decklink-host.exe`
//!   2. Allow UDP 31415 for **any** program (path-independent)
//!   3. Allow UDP 31415 for this exact exe

use std::path::PathBuf;
use std::process::Command;

use tracing::{info, warn};

use decklink_net::DEFAULT_PORT;

const RULE_PORT: &str = "DeckLink Host UDP Port";
const RULE_EXE: &str = "DeckLink Host UDP";
const MARKER_VER: &str = "v2";

/// Best-effort: clear DeckLink blocks and allow inbound UDP 31415.
pub fn ensure_firewall_rule() {
    let Some(exe) = std::env::current_exe().ok() else {
        warn!("cannot resolve exe path for firewall rule");
        return;
    };
    let exe_s = exe.display().to_string();

    if marker_matches(&exe_s) && !has_block_rules() {
        info!("firewall marker {MARKER_VER} ok — skip");
        return;
    }

    let exe_escaped = exe_s.replace('\'', "''");
    let ps = format!(
        "$ErrorActionPreference='SilentlyContinue'; \
         Get-NetFirewallApplicationFilter | Where-Object {{ $_.Program -like '*decklink-host.exe' }} | ForEach-Object {{ \
           $r = $_ | Get-NetFirewallRule; \
           if ($r -and $r.Action -eq 'Block') {{ Remove-NetFirewallRule -Name $r.Name | Out-Null }} \
         }}; \
         if (-not (Get-NetFirewallRule -DisplayName '{RULE_PORT}' -ErrorAction SilentlyContinue)) {{ \
           New-NetFirewallRule -DisplayName '{RULE_PORT}' -Direction Inbound -Action Allow \
             -Protocol UDP -LocalPort {DEFAULT_PORT} -Profile Any | Out-Null \
         }}; \
         $exe = '{exe_escaped}'; \
         Get-NetFirewallRule -DisplayName '{RULE_EXE}' -ErrorAction SilentlyContinue | Remove-NetFirewallRule; \
         New-NetFirewallRule -DisplayName '{RULE_EXE}' -Direction Inbound -Action Allow \
           -Protocol UDP -LocalPort {DEFAULT_PORT} -Program $exe -Profile Any | Out-Null; \
         exit 0"
    );

    if run_ps(&ps, false) || run_ps(&ps, true) {
        write_marker(&exe_s);
        info!("firewall: UDP {DEFAULT_PORT} allowed (blocks cleared)");
    } else {
        warn!(
            "could not update firewall — allow UDP {DEFAULT_PORT} inbound, \
             and remove any Block rules for decklink-host.exe"
        );
    }
}

fn has_block_rules() -> bool {
    let status = Command::new("powershell")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            "if (Get-NetFirewallApplicationFilter | Where-Object { $_.Program -like '*decklink-host.exe' } | ForEach-Object { $_ | Get-NetFirewallRule } | Where-Object { $_.Action -eq 'Block' -and $_.Enabled }) { exit 0 } else { exit 1 }",
        ])
        .status();
    matches!(status, Ok(s) if s.success())
}

fn marker_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("DeckLink")
        .join(format!("firewall_rule_{MARKER_VER}.txt"))
}

fn marker_matches(exe: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(marker_path()) else {
        return false;
    };
    text.trim() == exe.trim()
}

fn write_marker(exe: &str) {
    let path = marker_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, exe);
}

fn run_ps(script: &str, elevate: bool) -> bool {
    if elevate {
        let dir = std::env::temp_dir();
        let path = dir.join("decklink_fw.ps1");
        if std::fs::write(&path, script).is_err() {
            return false;
        }
        let path_s = path.display().to_string().replace('\'', "''");
        let launcher = format!(
            "Start-Process powershell -Verb RunAs -Wait -ArgumentList \
             '-NoProfile','-ExecutionPolicy','Bypass','-File','{path_s}'"
        );
        let status = Command::new("powershell")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &launcher])
            .status();
        let _ = std::fs::remove_file(&path);
        matches!(status, Ok(s) if s.success())
    } else {
        let status = Command::new("powershell")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script])
            .status();
        matches!(status, Ok(s) if s.success())
    }
}
