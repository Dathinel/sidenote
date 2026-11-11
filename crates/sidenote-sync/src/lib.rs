//! Standalone, opt-in-to-integration device synchronization. No app or global paths.
pub mod capabilities;
pub mod client;
mod crypto;
pub mod identity;
pub mod model;
pub mod pairing;
mod preflight;
pub mod status;
pub mod store;
pub mod transport;
mod wire;
pub mod worker;
