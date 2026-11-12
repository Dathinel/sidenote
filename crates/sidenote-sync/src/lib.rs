//! Standalone, opt-in-to-integration device synchronization. No app or global paths.
pub mod capabilities;
pub mod client;
#[cfg(target_os = "linux")]
pub mod containment;
mod crypto;
pub mod identity;
#[cfg(target_os = "linux")]
mod linux_secret;
pub mod model;
pub mod pairing;
mod preflight;
pub mod status;
pub mod store;
pub mod transport;
mod wire;
pub mod worker;
