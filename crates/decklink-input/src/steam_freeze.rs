//! Freeze Steam while DeckLink is advertising so Desktop stick→mouse cannot fight us.

use tracing::info;

/// SIGSTOP / SIGCONT Steam processes (SteamOS Desktop Mode).
pub fn set_steam_frozen(freeze: bool) {
    let arg = if freeze { "-STOP" } else { "-CONT" };
    let mut any = false;
    for name in ["steamwebhelper", "steam"] {
        let ok = std::process::Command::new("killall")
            .args(["-q", arg, name])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            any = true;
        }
    }
    if any {
        info!(
            "Steam processes {} (stops Desktop stick→mouse while DeckLink advertises)",
            if freeze { "frozen (SIGSTOP)" } else { "resumed (SIGCONT)" }
        );
    }
}
