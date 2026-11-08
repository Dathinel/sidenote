use crate::{
    chat::TextContent,
    entity::{Meeting, MemoryKind, MemoryRecord, MemorySource, Person, Project, Task, entity},
    ids::Id,
    version::Version,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    #[default]
    Pending,
    Accepted,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", content = "target", rename_all = "snake_case")]
pub enum MemoryOperation {
    Add,
    Confirm(Id<MemoryRecord>),
    Supersede(Id<MemoryRecord>),
    Retire(Id<MemoryRecord>),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryProposal {
    pub operation: MemoryOperation,
    pub text: TextContent,
    pub kind: MemoryKind,
    pub project_id: Option<Id<Project>>,
    pub people: Vec<Id<Person>>,
    pub sources: Vec<MemorySource>,
    pub rationale: String,
    pub expected_version: Option<Version>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatSummarySource {
    pub client_name: String,
    pub model: Option<String>,
    pub conversation_ref: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatSummaryProposal {
    pub title: String,
    pub summary: TextContent,
    pub key_points: Vec<String>,
    pub decisions: Vec<String>,
    pub follow_ups: Vec<String>,
    pub related_meeting_ids: Vec<Id<Meeting>>,
    pub source: ChatSummarySource,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum ProposalKind {
    Memory(MemoryProposal),
    ChatSummary(ChatSummaryProposal),
    TaskCompletion {
        task_id: Id<Task>,
        expected_version: Option<Version>,
    },
    NewProject {
        name: String,
    },
}

/// This is a review queue entry, never a direct change to its target entity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    pub kind: ProposalKind,
    pub status: ProposalStatus,
    pub decided_at: Option<DateTime<Utc>>,
}
impl Proposal {
    pub fn pending(kind: ProposalKind) -> Self {
        Self {
            kind,
            status: ProposalStatus::Pending,
            decided_at: None,
        }
    }
}

entity!(
    Proposal,
    "proposal",
    Uuid::from_u128(0x581b2a0d6d965b81a44758930e834080)
);
