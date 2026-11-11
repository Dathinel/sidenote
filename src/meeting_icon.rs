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
}

#[derive(Clone, Deserialize, Serialize)]
struct Pending {
    elapsed: u64,
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
    while !crate::updates::listener_stopping() {
        if replay && core::state_dir().join("local/floating-icon-exit").is_file() {
            // Only the explicitly isolated replay process accepts this bounded shutdown.
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
                    })
                })
            };
            if let Some(mut update) = update {
                update.id = update.id.filter(|id| valid_id(id));
                update.recording &= update.id.is_some();
                if current.recording
                    && (!update.recording || update.id != current.id)
                    && store_io::write(&pending_path(), &pending).is_err()
                {
                    crate::listener::log("could not save stopped floating meeting");
                }
                if update.id != current.id {
                    stamp = None;
                    last_words = 0;
                    speech_until = Instant::now();
                }
                current = update;
            }
            if current.recording
                && let Some(id) = &current.id
            {
                let fresh = !pending.contains_key(id);
                pending.insert(
                    id.clone(),
                    Pending {
                        elapsed: current.elapsed,
                    },
                );
                if fresh && store_io::write(&pending_path(), &pending).is_err() {
                    crate::listener::log("could not save pending floating meeting");
                }
                if !replay
                    && let Ok(core::live::Read::Lines(lines, new_stamp)) =
                        core::live::read_rolling(&core::meetings_dir().join(id), stamp)
                {
                    let had_baseline = stamp.is_some();
                    stamp = Some(new_stamp);
                    let words = lines
                        .iter()
                        .map(|l| l.text.split_whitespace().count())
                        .sum();
                    // Initial history is not evidence of speech happening now.
                    if had_baseline && words > last_words {
                        speech_until = Instant::now() + Duration::from_secs(2);
                    }
                    last_words = words;
                }
            }
            let before = pending.len();
            pending.retain(|id, _| {
                current.recording && current.id.as_ref() == Some(id)
                    || summary(id).is_none_or(|raw| !was_seen(id, &raw))
            });
            if pending.len() != before && store_io::write(&pending_path(), &pending).is_err() {
                crate::listener::log("could not save floating meeting acknowledgement");
            }
            let item = current
                .id
                .as_ref()
                .filter(|id| current.recording && pending.contains_key(*id))
                .or_else(|| pending.last_key_value().map(|(id, _)| id));
            if let Some(id) = item {
                if host.is_none() {
                    host = native::Host::new(replay)
                        .map_err(|_| crate::listener::log("floating icon window unavailable"))
                        .ok();
                }
                if let Some(host) = &mut host {
                    let recording = current.recording && current.id.as_ref() == Some(id);
                    let raw = summary(id);
                    host.update(
                        id,
                        recording,
                        recording && (current.speaking || Instant::now() < speech_until),
                        pending[id].elapsed,
                        raw,
                    );
                }
            } else {
                host = None;
            }
        }
        if let Some(host) = &mut host {
            host.pump();
        } else {
            std::thread::sleep(Duration::from_millis(if replay { 100 } else { 250 }));
        }
    }
}
