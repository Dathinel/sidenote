//! A lazy, native listener overlay. No Rune application or GPU device is initialized here.
mod native;
use crate::core::{self, file_reads, store_io};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct Snapshot {
    id: Option<String>,
    recording: bool,
    speaking: bool,
    elapsed: u64,
    seen: bool,
    now_ms: Option<i64>,
}

#[derive(Clone, Deserialize, Serialize)]
struct Pending {
    elapsed: u64,
    #[serde(default)]
    stopped_at_ms: Option<i64>,
    #[serde(default)]
    skipped_at_ms: Option<i64>,
    #[serde(default)]
    notice_at_ms: Option<i64>,
    #[serde(skip)]
    notice_since: Option<Instant>,
    #[serde(skip)]
    stopped_since: Option<Instant>,
    #[serde(skip)]
    skipped_since: Option<Instant>,
}

// Ten minutes allows queued notes/retries without pinning GUI resources when notes never arrive.
const NO_DRAFT_BOUND_MS: i64 = 10 * 60 * 1000;
const SKIPPED_NOTICE_MS: i64 = 3000;
const SUMMARY_NOTICE_MS: i64 = 30_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Notice {
    Writing,
    Later,
    Failed,
}
impl Notice {
    fn text(self) -> &'static str {
        match self {
            Self::Writing => "Writing summary...",
            Self::Later => "Summary will be written later",
            Self::Failed => "Summary failed, open Sidenote",
        }
    }
}
fn notice(id: &str) -> Notice {
    use core::notes::JobState;
    if core::meetings_dir()
        .join(id)
        .join("pipeline-failed.txt")
        .is_file()
        || core::notes::status(id).is_some_and(|s| {
            !matches!(
                s,
                JobState::Queued
                    | JobState::Running
                    | JobState::Done { .. }
                    | JobState::NoHelper
                    | JobState::NeedsClick
            )
        })
    {
        return Notice::Failed;
    }
    let deferred = store_io::read::<String>(
        &core::state_dir()
            .join("local/draft-outcomes")
            .join(format!("{id}.json")),
    )
    .is_ok_and(|s| s == "deferred");
    if deferred
        || matches!(
            core::notes::status(id),
            Some(JobState::NoHelper | JobState::NeedsClick)
        )
    {
        Notice::Later
    } else {
        Notice::Writing
    }
}
fn dismissed_path(id: &str) -> PathBuf {
    core::state_dir()
        .join("local/floating-icon-dismissed")
        .join(format!("{id}.json"))
}
fn dismiss_notice(id: &str) {
    if valid_id(id) {
        let _ = store_io::write(&dismissed_path(id), &chrono::Utc::now().timestamp_millis());
    }
}
fn notice_fresh(at: i64, now: i64, dismissed: Option<i64>) -> bool {
    now.saturating_sub(at) < SUMMARY_NOTICE_MS && dismissed.is_none_or(|d| d < at)
}

/// Render the native indicator without creating windows or starting a listener.
#[doc(hidden)]
pub fn render_synthetic() -> std::io::Result<()> {
    if !core::is_proof_path(&core::state_dir(), "meeting-indicator-proof") {
        return Err(std::io::ErrorKind::PermissionDenied.into());
    }
    std::fs::create_dir_all(core::state_dir().join("local"))?;
    native::render_synthetic()
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 100
        && id.bytes().any(|b| b.is_ascii_digit())
        && id.bytes().all(|b| b.is_ascii_digit() || b == b'-')
}

fn pending_path() -> PathBuf {
    core::state_dir().join("local/floating-meetings.json")
}

fn seen_path(id: &str) -> PathBuf {
    core::state_dir()
        .join("local/floating-icon-seen")
        .join(format!("{id}.json"))
}

/// Called only when a local meeting's actual draft/final content is displayed.
pub fn draft_seen(id: &str, raw: &str) {
    if valid_id(id) && !raw.is_empty() && !was_seen(id, raw) {
        let digest = blake3::hash(raw.as_bytes()).to_hex().to_string();
        if store_io::write(&seen_path(id), &digest).is_err() {
            crate::listener::log("could not acknowledge the floating meeting draft");
        }
    }
}

fn summary(id: &str) -> Option<String> {
    let final_path = core::meetings_dir().join(id).join("english-summary.json");
    let path = if final_path.is_file() {
        final_path
    } else if notice(id) == Notice::Failed {
        return None;
    } else {
        core::state_dir()
            .join("local/draft-summaries")
            .join(format!("{id}.json"))
    };
    file_reads::text(&path, file_reads::SUMMARY_MAX)
        .ok()
        .filter(|s| {
            serde_json::from_str::<core::Summary>(s).is_ok_and(|s| !s.title.trim().is_empty())
        })
}

fn was_seen(id: &str, raw: &str) -> bool {
    store_io::read::<String>(&seen_path(id))
        .is_ok_and(|s| s == blake3::hash(raw.as_bytes()).to_hex().as_str())
}

fn card_text(id: &str, recording: bool, raw: Option<&str>) -> String {
    if let Some(raw) = raw
        && let Ok(s) = serde_json::from_str::<core::Summary>(raw)
    {
        let mut lines = vec![s.title];
        lines.extend(s.context);
        for topic in s.topics {
            lines.push(topic.title);
            lines.extend(topic.points.into_iter().map(|p| format!("  {}", p.text)));
        }
        lines.extend(s.todo.into_iter().map(|t| format!("To do: {}", t.text())));
        lines.extend(s.to_settle.into_iter().map(|q| q.question));
        return lines.join("\r\n\r\n");
    }
    let folder = core::meetings_dir().join(id);
    let text = match core::live::read_rolling(&folder, None) {
        Ok(core::live::Read::Lines(lines, _)) => lines
            .iter()
            .rev()
            .take(12)
            .rev()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\r\n\r\n"),
        _ => String::new(),
    };
    format!(
        "{}\r\n\r\n{text}",
        if recording {
            "Meeting is recording. Live words may be a few seconds behind."
        } else {
            "Recording saved. Waiting for meeting notes."
        }
    )
}

pub(crate) fn start(isolated: bool) {
    let replay = isolated && std::env::args().any(|a| a == "--floating-icon-replay");
    if let Err(e) = std::thread::Builder::new()
        .name("meeting-icon".into())
        .spawn(move || run(isolated, replay))
    {
        crate::listener::log(&format!("floating icon could not start ({})", e.kind()));
    }
}

fn run(isolated: bool, replay: bool) {
    let engine = crate::hex_engine::HexEngine::new();
    let mut pending: BTreeMap<String, Pending> =
        store_io::read(&pending_path()).unwrap_or_default();
    pending.retain(|id, _| valid_id(id));
    let mut host: Option<native::Host> = None;
    let mut current = Snapshot::default();
    let mut stamp = None;
    let mut last_words = 0;
    let mut speech_until = Instant::now();
    let mut next_status = Instant::now();
    let mut transcript_revision = 0u64;
    let mut last_diagnostics = None;
    while !crate::updates::listener_stopping() {
        if replay {
            let status = native::diagnostics();
            if last_diagnostics != Some(status) {
                let _ = store_io::write(
                    &core::state_dir().join("local/floating-icon-workers.json"),
                    &status,
                );
                last_diagnostics = Some(status);
            }
        }
        if replay && core::state_dir().join("local/floating-icon-exit").is_file() {
            // Only the explicitly isolated replay process accepts this bounded shutdown.
            crate::listener::log("exit reason=stop source=floating-icon-replay");
            std::process::exit(0);
        }
        if Instant::now() >= next_status {
            next_status = Instant::now() + Duration::from_millis(if replay { 100 } else { 1500 });
            let update = if replay {
                file_reads::text(
                    &core::state_dir().join("local/floating-icon-replay.json"),
                    4096,
                )
                .ok()
                .and_then(|s| serde_json::from_str::<Snapshot>(&s).ok())
            } else if isolated {
                Some(Snapshot::default())
            } else {
                engine.status().ok().and_then(|status| {
                    if !matches!(
                        status.meeting_state.as_str(),
                        "recording" | "idle" | "stopped" | "complete" | "finished"
                    ) {
                        return None;
                    }
                    let id = core::meeting_watch::recording_id(&status, &core::meetings_dir());
                    if status.meeting_state == "recording" && id.is_none() {
                        return None;
                    }
                    Some(Snapshot {
                        recording: id.is_some(),
                        id,
                        elapsed: status
                            .recording
                            .as_ref()
                            .and_then(|r| r.elapsed_seconds)
                            .unwrap_or(0),
                        speaking: false,
                        ..Snapshot::default()
                    })
                })
            };
            if let Some(mut update) = update {
                update.id = update.id.filter(|id| valid_id(id));
                update.recording &= update.id.is_some();
                if update.id != current.id {
                    stamp = None;
                    last_words = 0;
                    speech_until = Instant::now();
                    transcript_revision = transcript_revision.wrapping_add(1);
                }
                current = update;
            }
            let now = if replay {
                current
                    .now_ms
                    .unwrap_or_else(|| chrono::Utc::now().timestamp_millis())
            } else {
                chrono::Utc::now().timestamp_millis()
            };
            let mut dirty = false;
            if replay
                && current.seen
                && let Some(id) = &current.id
                && let Some(raw) = summary(id)
            {
                draft_seen(id, &raw);
            }
            if current.recording
                && let Some(id) = &current.id
            {
                let fresh = !pending.contains_key(id);
                dirty |= fresh || pending.get(id).is_some_and(|p| p.stopped_at_ms.is_some());
                pending.insert(
                    id.clone(),
                    Pending {
                        elapsed: current.elapsed,
                        stopped_at_ms: None,
                        notice_at_ms: None,
                        notice_since: None,
                        skipped_at_ms: None,
                        stopped_since: None,
                        skipped_since: None,
                    },
                );
                if let Ok(core::live::Read::Lines(lines, new_stamp)) =
                    core::live::read_rolling(&core::meetings_dir().join(id), stamp)
                {
                    let had_baseline = stamp.is_some();
                    stamp = Some(new_stamp);
                    transcript_revision = transcript_revision.wrapping_add(1);
                    let words = lines
                        .iter()
                        .map(|l| l.text.split_whitespace().count())
                        .sum();
                    // Initial history is not evidence of speech happening now.
                    if !replay && had_baseline && words > last_words {
                        speech_until = Instant::now() + Duration::from_secs(2);
                    }
                    last_words = words;
                }
            }
            let before = pending.len();
            pending.retain(|id, p| {
                if current.recording && current.id.as_ref() == Some(id) {
                    return true;
                }
                if p.stopped_at_ms.is_none_or(|t| t > now) {
                    p.stopped_at_ms = Some(now);
                    dirty = true;
                }
                let stopped_since = *p.stopped_since.get_or_insert_with(Instant::now);
                let short = core::recording_guard::eligibility(
                    &core::state_dir(),
                    &core::meetings_dir(),
                    id,
                )
                .is_ok_and(|e| e == core::recording_guard::Eligibility::Short);
                if short {
                    if p.skipped_at_ms.is_none_or(|t| t > now) {
                        p.skipped_at_ms = Some(now);
                        dirty = true;
                    }
                    let skipped_since = *p.skipped_since.get_or_insert_with(Instant::now);
                    return now.saturating_sub(p.skipped_at_ms.unwrap_or(now)) < SKIPPED_NOTICE_MS
                        && skipped_since.elapsed()
                            < Duration::from_millis(SKIPPED_NOTICE_MS as u64);
                }
                if p.skipped_at_ms.take().is_some() {
                    dirty = true;
                }
                p.skipped_since = None;
                if let Some(raw) = summary(id) {
                    return !was_seen(id, &raw);
                }
                let state = notice(id);
                let expired = now.saturating_sub(p.stopped_at_ms.unwrap_or(now))
                    >= NO_DRAFT_BOUND_MS
                    || stopped_since.elapsed() >= Duration::from_millis(NO_DRAFT_BOUND_MS as u64);
                if state != Notice::Writing || expired || p.notice_at_ms.is_some() {
                    let at = *p.notice_at_ms.get_or_insert_with(|| {
                        dirty = true;
                        now
                    });
                    let since = *p.notice_since.get_or_insert_with(Instant::now);
                    let dismissed = store_io::read::<i64>(&dismissed_path(id)).ok();
                    return notice_fresh(at, now, dismissed)
                        && since.elapsed() < Duration::from_millis(SUMMARY_NOTICE_MS as u64);
                }
                true
            });
            if (dirty || pending.len() != before)
                && store_io::write(&pending_path(), &pending).is_err()
            {
                crate::listener::log("could not save floating meeting acknowledgement");
            }
            let item = current
                .id
                .as_ref()
                .filter(|id| current.recording && pending.contains_key(*id))
                .or_else(|| pending.last_key_value().map(|(id, _)| id));
            if let Some(id) = item {
                if host.is_none() && !native::disabled() {
                    host = native::Host::new(replay)
                        .map_err(|_| crate::listener::log("floating icon window unavailable"))
                        .ok();
                }
                if let Some(window) = &host {
                    let recording = current.recording && current.id.as_ref() == Some(id);
                    let raw = summary(id);
                    if !window.update(native::Frame {
                        id: id.clone(),
                        recording,
                        pill: recording && (current.speaking || Instant::now() < speech_until),
                        elapsed: pending[id].elapsed,
                        notice: (!recording
                            && raw.is_none()
                            && pending[id].skipped_at_ms.is_none())
                        .then(|| {
                            let state = notice(id);
                            if state == Notice::Writing && pending[id].notice_at_ms.is_some() {
                                Notice::Later
                            } else {
                                state
                            }
                        }),
                        raw,
                        transcript_revision,
                        skipped: pending[id].skipped_at_ms.is_some(),
                    }) {
                        host = None;
                    }
                }
            } else {
                host = None;
            }
        }
        native::maintenance();
        std::thread::sleep(Duration::from_millis(if replay { 100 } else { 250 }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_notice_is_bounded_and_click_dismisses() {
        assert!(notice_fresh(1000, 30_999, None));
        assert!(!notice_fresh(1000, 31_000, None));
        assert!(!notice_fresh(1000, 1001, Some(1001)));
        assert!(notice_fresh(1000, 1001, Some(999)));
        assert_eq!(Notice::Later.text(), "Summary will be written later");
        assert_eq!(Notice::Failed.text(), "Summary failed, open Sidenote");
    }
}
