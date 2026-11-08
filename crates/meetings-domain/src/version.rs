use crate::entity::{Entity, Envelope};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const STORE_API_VERSION: u16 = 1;

/// Engine-owned optimistic concurrency token. Callers must not order or increment it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Version(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(bound = "E: Entity")]
pub struct Versioned<E: Entity> {
    pub version: Version,
    pub entity: Envelope<E>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchemaError {
    Newer {
        kind: &'static str,
        found: u16,
        supported: u16,
    },
    Unsupported {
        kind: &'static str,
        found: u16,
    },
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Newer {
                kind,
                found,
                supported,
            } => write!(
                f,
                "Update Meetings on this device ({kind} schema {found}, supported {supported})"
            ),
            Self::Unsupported { kind, found } => {
                write!(f, "Unsupported {kind} schema {found}")
            }
        }
    }
}
impl std::error::Error for SchemaError {}
