//! Freeze Steam while a HID host is connected so Desktop stick→mouse cannot fight us.

use tracing::info;

/// SIGSTOP / SIGCONT Steam processes (SteamOS Desktop Mode).
/// Call only after HID Connected — freezing during advertise makes pairing feel dead.
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
            "Steam processes {} (stops Desktop stick→mouse while host HID is live)",
            if freeze {
                "frozen (SIGSTOP)"
            } else {
                "resumed (SIGCONT)"
            }
        );
    }
}
