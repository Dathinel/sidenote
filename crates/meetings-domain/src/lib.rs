//! Storage-independent domain records (ADR 0001, 0002, 0006 and 0007).
//!
//! Entities are payloads wrapped in [`Envelope`]. Long content lives in blobs;
//! files, sync engines, agent execution and schema migration IO belong to callers.

#![forbid(unsafe_code)]

pub mod chat;
pub mod entity;
pub mod ids;
pub mod proposal;
pub mod provenance;
pub mod version;

pub use chat::*;
pub use entity::*;
pub use ids::{Id, meeting_id};
pub use proposal::*;
pub use provenance::*;
pub use version::{STORE_API_VERSION, SchemaError, Version, Versioned};
