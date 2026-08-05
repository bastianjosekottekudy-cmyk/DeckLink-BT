//! Freeze Steam while a HID host is connected so Desktop stick→mouse cannot fight us.

use std::sync::atomic::{AtomicBool, Ordering};

use tracing::info;

static FROZEN: AtomicBool = AtomicBool::new(false);

/// SIGSTOP / SIGCONT Steam processes (SteamOS Desktop Mode).
/// No-ops when the requested state matches the last applied state (avoids spam).
pub fn set_steam_frozen(freeze: bool) {
    if FROZEN.swap(freeze, Ordering::SeqCst) == freeze {
        return;
    }
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
    } else if !freeze {
        // Still record unfrozen even if killall found nothing.
    }
}
