use crate::entity::{Entity, Meeting};
use serde::{Deserialize, Serialize};
use std::{fmt, hash::Hash, marker::PhantomData, str::FromStr};
use uuid::Uuid;

/// A UUID whose Rust type prevents mixing entity kinds. The wire format is a UUID string.
#[derive(Serialize, Deserialize)]
#[serde(transparent, bound = "")]
pub struct Id<T> {
    uuid: Uuid,
    #[serde(skip)]
    marker: PhantomData<fn() -> T>,
}

impl<T> Id<T> {
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self {
            uuid,
            marker: PhantomData,
        }
    }

    /// New, time-ordered identity. Reads the system clock and OS randomness only.
    pub fn new_v7() -> Self {
        Self::from_uuid(Uuid::now_v7())
    }

    pub fn new_v5(namespace: &Uuid, key: &[u8]) -> Self {
        Self::from_uuid(Uuid::new_v5(namespace, key))
    }

    pub const fn as_uuid(&self) -> &Uuid {
        &self.uuid
    }
}

// Manual implementations avoid imposing traits on the entity payload T.
impl<T> Copy for Id<T> {}
impl<T> Clone for Id<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> PartialEq for Id<T> {
    fn eq(&self, other: &Self) -> bool {
        self.uuid == other.uuid
    }
}
impl<T> Eq for Id<T> {}
impl<T> PartialOrd for Id<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<T> Ord for Id<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.uuid.cmp(&other.uuid)
    }
}
impl<T> Hash for Id<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.uuid.hash(state);
    }
}
impl<T> fmt::Display for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.uuid.fmt(f)
    }
}
impl<T> fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Id").field(&self.uuid).finish()
    }
}
impl<T> FromStr for Id<T> {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self::from_uuid)
    }
}

/// Fixed wire-contract namespaces. Never change after records have been written.
/// Derived once as UUIDv5(URL, "https://hex-meetings.org/ns/<kind>").
pub const NS_MEETING: Uuid = Uuid::from_u128(0xfc0741d8aaa8556d943cdcd2b014143e);
pub const NS_THREAD: Uuid = Uuid::from_u128(0xe0817569d3585e5098e387f2c203e3c8);

impl<T: Entity> Id<T> {
    /// Import an external key under the entity kind's permanent namespace.
    pub fn from_key(key: &[u8]) -> Self {
        Self::new_v5(&T::NAMESPACE, key)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidMeetingKey;
impl fmt::Display for InvalidMeetingKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Meeting identity needs a nonempty device key without '/' and a digits-digits Hex folder")
    }
}
impl std::error::Error for InvalidMeetingKey {}

/// Device and folder are the exact legacy keys, without case or whitespace normalization.
pub fn meeting_id(device_id: &str, folder: &str) -> Result<Id<Meeting>, InvalidMeetingKey> {
    let valid_folder = folder.split_once('-').is_some_and(|(a, b)| {
        !a.is_empty() && !b.is_empty() && a.bytes().chain(b.bytes()).all(|c| c.is_ascii_digit())
    });
    if device_id.is_empty() || device_id.contains('/') || !valid_folder {
        return Err(InvalidMeetingKey);
    }
    Ok(Id::new_v5(
        &NS_MEETING,
        format!("hex/{device_id}/{folder}").as_bytes(),
    ))
}
