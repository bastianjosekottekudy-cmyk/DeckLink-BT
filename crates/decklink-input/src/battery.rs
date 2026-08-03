//! Read Steam Deck battery from sysfs (Linux) or stub.

#[cfg(target_os = "linux")]
pub fn read_battery_percent() -> u8 {
    use std::fs;
    // Steam Deck / typical Linux power supply
    let candidates = [
        "/sys/class/power_supply/BAT1/capacity",
        "/sys/class/power_supply/BAT0/capacity",
        "/sys/class/power_supply/bms/capacity",
    ];
    for path in candidates {
        if let Ok(s) = fs::read_to_string(path) {
            if let Ok(v) = s.trim().parse::<u8>() {
                return v.min(100);
            }
        }
    }
    100
}

#[cfg(not(target_os = "linux"))]
pub fn read_battery_percent() -> u8 {
    100
}
