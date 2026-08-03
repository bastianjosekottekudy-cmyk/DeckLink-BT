//! Steam Deck input capture (evdev on Linux, synthetic pump elsewhere).

mod battery;
mod types;

pub use battery::read_battery_percent;
pub use types::{InputError, InputEvent, InputHandle};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::spawn_input_task;

#[cfg(not(target_os = "linux"))]
mod stub;
#[cfg(not(target_os = "linux"))]
pub use stub::spawn_input_task;
