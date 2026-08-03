//! Slint UI bindings for DeckLink BT.

slint::include_modules!();

use decklink_profiles::{PairedTarget, Profile};

pub fn profile_from_index(idx: i32) -> Profile {
    match idx {
        1 => Profile::Desktop,
        2 => Profile::Flight,
        3 => Profile::Racing,
        _ => Profile::Gamepad,
    }
}

pub fn index_from_profile(p: Profile) -> i32 {
    match p {
        Profile::Gamepad => 0,
        Profile::Desktop => 1,
        Profile::Flight => 2,
        Profile::Racing => 3,
    }
}

pub fn format_targets(targets: &[PairedTarget]) -> String {
    if targets.is_empty() {
        return "(none yet — pair from the host Bluetooth menu)".into();
    }
    targets
        .iter()
        .map(|t| format!("• {}  ({})", t.name, t.address))
        .collect::<Vec<_>>()
        .join("\n")
}
