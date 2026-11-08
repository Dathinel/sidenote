use crate::{ids::Id, provenance::Provenance, version::SchemaError};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned};
use serde_json::Value;
use uuid::Uuid;

/// Schema is independent of the sync engine and Store API version.
pub trait Entity: Clone + std::fmt::Debug + PartialEq + Serialize + DeserializeOwned {
    const KIND: &'static str;
    const NAMESPACE: Uuid;
    const SCHEMA: u16 = 1;

    /// Pure upgrade hook, called before decoding the payload on every record read.
    /// Schema 1 is the first public format; there is no schema 0 migration.
    fn upgrade(schema: u16, data: Value) -> Result<Value, SchemaError> {
        match schema {
            n if n == Self::SCHEMA => Ok(data),
            n if n > Self::SCHEMA => Err(SchemaError::Newer {
                kind: Self::KIND,
                found: n,
                supported: Self::SCHEMA,
            }),
            found => Err(SchemaError::Unsupported {
                kind: Self::KIND,
                found,
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Actor {
    Device(Id<Device>),
    /// Authentication principal, distinct from a meeting participant.
    User(Uuid),
}

/// Shared metadata and a field-mergeable payload, with no engine clocks or paths.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(bound = "E: Entity")]
pub struct Envelope<E: Entity> {
    pub id: Id<E>,
    pub workspace_id: Id<Workspace>,
    schema: u16,
    pub created_at: DateTime<Utc>,
    pub created_by: Actor,
    pub updated_at: DateTime<Utc>,
    pub deleted: bool,
    pub provenance: Option<Provenance>,
    pub data: E,
}

impl<E: Entity> Envelope<E> {
    pub fn new(
        id: Id<E>,
        workspace_id: Id<Workspace>,
        created_at: DateTime<Utc>,
        created_by: Actor,
        data: E,
    ) -> Self {
        Self {
            id,
            workspace_id,
            schema: E::SCHEMA,
            created_at,
            created_by,
            updated_at: created_at,
            deleted: false,
            provenance: None,
            data,
        }
    }

    pub fn schema(&self) -> u16 {
        self.schema
    }
}

impl<'de, E: Entity> Deserialize<'de> for Envelope<E> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Inspect schema before decoding fields a newer writer may have reshaped.
        let mut value = Value::deserialize(deserializer)?;
        let schema = value
            .get("schema")
            .and_then(Value::as_u64)
            .and_then(|n| u16::try_from(n).ok())
            .ok_or_else(|| serde::de::Error::custom("Entity schema must be a u16"))?;
        let data = value
            .get_mut("data")
            .map(Value::take)
            .unwrap_or(Value::Null);
        let upgraded = E::upgrade(schema, data).map_err(serde::de::Error::custom)?;
        let object = value
            .as_object_mut()
            .ok_or_else(|| serde::de::Error::custom("Entity must be an object"))?;
        object.insert("data".into(), upgraded);

        #[derive(Deserialize)]
        struct Wire<E> {
            id: Id<E>,
            workspace_id: Id<Workspace>,
            created_at: DateTime<Utc>,
            created_by: Actor,
            updated_at: DateTime<Utc>,
            deleted: bool,
            provenance: Option<Provenance>,
            data: E,
        }
        let wire: Wire<E> = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        Ok(Self {
            id: wire.id,
            workspace_id: wire.workspace_id,
            schema: E::SCHEMA,
            created_at: wire.created_at,
            created_by: wire.created_by,
            updated_at: wire.updated_at,
            deleted: wire.deleted,
            provenance: wire.provenance,
            data: wire.data,
        })
    }
}

/// Opaque 32-byte content digest. The Store computes it, not the domain crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BlobId(pub [u8; 32]);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceKind {
    Personal,
    Team,
}

/// A workspace's envelope uses its own id as workspace_id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub name: String,
    pub kind: WorkspaceKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meeting {
    pub legacy_key: String,
    pub device_id: Id<Device>,
    pub title: String,
    pub started_at: DateTime<Utc>,
    pub duration_ms: Option<u64>,
    pub project_id: Option<Id<Project>>,
    pub attendee_ids: Vec<Id<Person>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transcript {
    pub meeting_id: Id<Meeting>,
    pub language: Option<String>,
    pub body: BlobId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteKind {
    GeneratedSummary,
    UserNotes,
    Scratchpad,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub meeting_id: Id<Meeting>,
    pub kind: NoteKind,
    pub body: BlobId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub meeting_id: Option<Id<Meeting>>,
    pub project_id: Option<Id<Project>>,
    pub text: String,
    pub owner_id: Option<Id<Person>>,
    pub due: Option<NaiveDate>,
    pub completed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Person,
    Project,
    Decision,
    Commitment,
    Question,
    Story,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    Proposed,
    Active,
    Superseded,
    Retired,
    Forgotten,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MemorySource {
    Meeting {
        meeting_id: Id<Meeting>,
        at_ms: Option<u64>,
    },
    KnowledgeBase {
        path: String,
        revision: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub kind: MemoryKind,
    pub body: BlobId,
    pub status: MemoryStatus,
    pub project_id: Option<Id<Project>>,
    pub people: Vec<Id<Person>>,
    pub sources: Vec<MemorySource>,
    pub supersedes: Option<Id<MemoryRecord>>,
}

/// Identity and contact data only. Audio and embeddings do not belong here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Person {
    pub name: String,
    pub email: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AttendeeEvidence {
    Calendar {
        event_ref: String,
    },
    TranscriptSelfIntroduction {
        transcript_id: Id<Transcript>,
        at_ms: u64,
    },
    Manual,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttendeeSuggestion {
    pub meeting_id: Id<Meeting>,
    pub person_id: Option<Id<Person>>,
    pub name: String,
    pub evidence: AttendeeEvidence,
}

/// Roster metadata only. Device configuration, keys and job state remain local.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub name: String,
    pub legacy_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyReport {
    pub date: NaiveDate,
    pub body: BlobId,
    pub meeting_ids: Vec<Id<Meeting>>,
    pub proposal_ids: Vec<Id<crate::proposal::Proposal>>,
}

macro_rules! entity {
    ($type:ty, $kind:literal, $namespace:expr) => {
        impl $crate::entity::Entity for $type {
            const KIND: &'static str = $kind;
            const NAMESPACE: uuid::Uuid = $namespace;
        }
    };
}
pub(crate) use entity;

entity!(
    Workspace,
    "workspace",
    Uuid::from_u128(0x28a07cc74b00561dbacf100462b49f60)
);
entity!(Meeting, "meeting", crate::ids::NS_MEETING);
entity!(
    Transcript,
    "transcript",
    Uuid::from_u128(0x4fde01f49f455571abe0314260de5c7d)
);
entity!(
    Note,
    "note",
    Uuid::from_u128(0xfdbf6d532f5c516e8dd6922d23602b00)
);
entity!(
    Task,
    "task",
    Uuid::from_u128(0x8a2cd0d5d8955d44843bc5141a2b762c)
);
entity!(
    Project,
    "project",
    Uuid::from_u128(0x59907228564650039b0edea85cb4234f)
);
entity!(
    MemoryRecord,
    "memory_record",
    Uuid::from_u128(0x39b4f3d28a6b59ae92f32da1fe0e22b0)
);
entity!(
    Person,
    "person",
    Uuid::from_u128(0xa040ce17dc5554f089904d8df0f01a51)
);
entity!(
    AttendeeSuggestion,
    "attendee_suggestion",
    Uuid::from_u128(0x3333fd31ed1356668d6997fbebb519b6)
);
entity!(
    Device,
    "device",
    Uuid::from_u128(0x962690a7de93523a878810ff46a720b5)
);
entity!(
    DailyReport,
    "daily_report",
    Uuid::from_u128(0x34555730d45054f5b520316a207f2e94)
);
