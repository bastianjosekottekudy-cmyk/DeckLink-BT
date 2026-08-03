//! BlueZ HOGP GATT peripheral (Linux) + stub (elsewhere).

mod types;
pub use types::{BtError, BtEvent, BtStatus, HogpServer};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::start_hogp;

#[cfg(not(target_os = "linux"))]
mod stub;
#[cfg(not(target_os = "linux"))]
pub use stub::start_hogp;
