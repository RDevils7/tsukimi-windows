#[cfg(target_os = "linux")]
pub mod mpris;
#[cfg(not(target_os = "linux"))]
pub mod mpris_stub;
pub mod player;
