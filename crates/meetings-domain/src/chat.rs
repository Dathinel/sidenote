use crate::{
    entity::{BlobId, Device, Meeting, Project, entity},
    ids::Id,
};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

pub const MAX_INLINE_TEXT_BYTES: usize = 32 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextTooLarge;
impl fmt::Display for TextTooLarge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Text over 32 KiB must be stored as a content blob")
    }
}
impl std::error::Error for TextTooLarge {}

/// Text is inline up to 32 KiB. Larger bodies are written by the Store as blobs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "TextWire", into = "TextWire")]
pub struct TextContent(TextWire);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum TextWire {
    Inline(String),
    Blob(BlobId),
}

impl TextContent {
    pub fn inline(text: String) -> Result<Self, TextTooLarge> {
        Self::try_from(TextWire::Inline(text))
    }
    pub fn blob(id: BlobId) -> Self {
        Self(TextWire::Blob(id))
    }
    pub fn as_inline(&self) -> Option<&str> {
        match &self.0 {
            TextWire::Inline(text) => Some(text),
            TextWire::Blob(_) => None,
        }
    }
    pub fn as_blob(&self) -> Option<BlobId> {
        match self.0 {
            TextWire::Blob(id) => Some(id),
            TextWire::Inline(_) => None,
        }
    }
}
impl TryFrom<TextWire> for TextContent {
    type Error = TextTooLarge;
    fn try_from(value: TextWire) -> Result<Self, Self::Error> {
        if matches!(&value, TextWire::Inline(text) if text.len() > MAX_INLINE_TEXT_BYTES) {
            return Err(TextTooLarge);
        }
        Ok(Self(value))
    }
}
impl From<TextContent> for TextWire {
    fn from(value: TextContent) -> Self {
        value.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TitleSource {
    User,
    Auto,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum ChatScope {
    Meeting(Id<Meeting>),
    AllMeetings,
    Project(Id<Project>),
    None,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatThread {
    pub title: String,
    pub title_source: TitleSource,
    pub scope: ChatScope,
    pub pinned: bool,
    pub archived: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    User,
    Assistant,
    ExternalSummary,
    Note,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageStatus {
    Complete,
    Failed,
    Stopped,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Citation {
    pub meeting_id: Id<Meeting>,
    pub at_ms: Option<u64>,
}

/// The envelope carries timestamps, tombstones and the full model/harness provenance.
/// Completed messages are immutable except status; the Store must enforce that rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub thread_id: Id<ChatThread>,
    pub role: ChatRole,
    pub text: TextContent,
    pub device_id: Id<Device>,
    pub status: MessageStatus,
    pub citations: Vec<Citation>,
}

entity!(ChatThread, "chat_thread", crate::ids::NS_THREAD);
entity!(
    ChatMessage,
    "chat_message",
    Uuid::from_u128(0xa1834656f2b756258efdee8afcb202af)
);
