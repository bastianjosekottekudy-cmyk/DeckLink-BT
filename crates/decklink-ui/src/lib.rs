//! Slint UI bindings for DeckLink.

slint::include_modules!();

use decklink_profiles::{PairedTarget, Profile};

pub fn profile_from_index(idx: i32) -> Profile {
    match idx {
        1 => Profile::Desktop,
        _ => Profile::Gamepad,
    }
}

pub fn index_from_profile(p: Profile) -> i32 {
    match p {
        Profile::Gamepad => 0,
        Profile::Desktop => 1,
    }
}

pub fn format_targets(targets: &[PairedTarget]) -> String {
    if targets.is_empty() {
        return "(none yet — Connect to a PC running decklink-host)".into();
    }
    targets
        .iter()
        .map(|t| format!("• {}  ({})", t.name, t.address))
        .collect::<Vec<_>>()
        .join("\n")
}
