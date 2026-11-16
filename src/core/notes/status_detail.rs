use super::{Helper, JobState, Request};
use crate::core::{
    file_reads,
    meeting_router::fallback::{self, RoutingFallback},
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::SystemTime,
};

type Stamp = (u64, Option<SystemTime>, Option<SystemTime>);
fn stamp(meta: &fs::Metadata) -> Stamp {
    (meta.len(), meta.modified().ok(), meta.created().ok())
}
#[derive(Default)]
struct Cache {
    root: PathBuf,
    jobs: BTreeMap<String, (Stamp, StatusDetail)>,
}

pub fn status_records<'a>(
    root: &Path,
    ids: impl IntoIterator<Item = &'a str>,
) -> BTreeMap<String, StatusDetail> {
    let ids: Vec<_> = ids
        .into_iter()
        .filter(|id| super::queue::valid_id(id))
        .collect();
    let mut result = BTreeMap::new();
    if ids.is_empty() {
        return result;
    }
    let directory = root.join("local/notes-jobs");
    if crate::core::store_io::reject_reparse(&directory).is_err() {
        return result;
    }
    let mut available = BTreeMap::new();
    if let Ok(entries) = fs::read_dir(&directory) {
        for entry in entries.take(file_reads::ENTRIES_MAX).flatten() {
            let name = entry.file_name();
            let Some((id, extension)) = name.to_str().and_then(|name| name.rsplit_once('.')) else {
                continue;
            };
            if !extension.eq_ignore_ascii_case("json") {
                continue;
            }
            if let Ok(meta) = entry.metadata()
                && file_reads::regular(&meta)
            {
                available.insert(id.to_owned(), stamp(&meta));
            }
        }
    }
    for id in &ids {
        if !available.contains_key(*id)
            && let Ok(meta) = fs::symlink_metadata(directory.join(format!("{id}.json")))
            && file_reads::regular(&meta)
        {
            available.insert((*id).to_owned(), stamp(&meta));
        }
    }
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| {
            let mut cache = poisoned.into_inner();
            cache.jobs.clear();
            cache
        });
    if cache.root != root {
        cache.root = root.to_owned();
        cache.jobs.clear();
    }
    cache.jobs.retain(|id, _| available.contains_key(id));
    for id in ids {
        let Some(before) = available.get(id) else {
            continue;
        };
        let job = if before.1.is_some()
            && let Some((_, job)) = cache.jobs.get(id).filter(|(saved, _)| saved == before)
        {
            Some(job.clone())
        } else {
            cache.jobs.remove(id);
            let job = status_record(root, id);
            let path = directory.join(format!("{id}.json"));
            if before.1.is_some()
                && let Some(job) = &job
                && fs::symlink_metadata(path)
                    .is_ok_and(|meta| file_reads::regular(&meta) && stamp(&meta) == *before)
            {
                cache.jobs.insert(id.to_owned(), (*before, job.clone()));
            }
            job
        };
        if let Some(job) = job {
            result.insert(id.to_owned(), job);
        }
    }
    result
}

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
