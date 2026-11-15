use super::{Helper, JobState, Request};
use crate::core::{
    file_reads,
    meeting_router::fallback::{self, RoutingFallback},
};
use serde::Deserialize;
use std::path::Path;

/// The persisted pin and its routing warning travel together to every notes surface.
#[derive(Clone, Debug, Deserialize)]
pub struct StatusDetail {
    #[serde(flatten)]
    pub state: JobState,
    #[serde(default)]
    pub helper: Option<Helper>,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub request: Option<Request>,
    #[serde(default)]
    pub transcript_revision: Option<String>,
    #[serde(default)]
    pub routing_fallbacks: Vec<RoutingFallback>,
}
impl StatusDetail {
    pub fn retry_request(&self) -> Request {
        self.request.unwrap_or(Request::TryAgain)
    }
    pub fn queued() -> Self {
        Self {
            state: JobState::Queued,
            helper: None,
            updated_at: 0,
            request: None,
            transcript_revision: None,
            routing_fallbacks: Vec::new(),
        }
    }
    pub fn routing_message(&self) -> String {
        if self.transcript_revision.is_some() {
            fallback::message(&self.routing_fallbacks)
        } else {
            String::new()
        }
    }
}
pub fn status_record(root: &Path, id: &str) -> Option<StatusDetail> {
    if !super::queue::valid_id(id) {
        return None;
    }
    let path = root.join("local/notes-jobs").join(format!("{id}.json"));
    serde_json::from_str(&file_reads::text(&path, file_reads::JOURNAL_MAX).ok()?).ok()
}
