//! Operator output is an allowlist of counts, timestamps and fixed states.
use crate::{core::store_io, sync_owner};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{fs, io, path::Path};

#[derive(Default, Serialize)]
struct Queue {
    pending: usize,
    failed: usize,
    blocked: usize,
    unreadable: usize,
}
fn queue(root: &Path, notes: bool) -> Queue {
    let mut counts = Queue::default();
    let path = root.join(if notes {
        "local/notes-jobs"
    } else {
        "local/router-queue"
    });
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return counts,
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {}
        _ => {
            counts.unreadable += 1;
            return counts;
        }
    }
    let Ok(entries) = fs::read_dir(path) else {
        counts.unreadable += 1;
        return counts;
    };
    for (index, entry) in entries.enumerate() {
        if index >= 4096 {
            counts.unreadable += 1;
            break;
        }
        let Ok(entry) = entry else {
            counts.unreadable += 1;
            continue;
        };
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let value = store_io::read_bounded(&path, 64 * 1024)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        let Some(value) = value else {
            counts.unreadable += 1;
            continue;
        };
        if notes {
            match value["state"].as_str() {
                Some("queued" | "running") => counts.pending += 1,
                Some("done") => counts.failed += usize::from(value["tasks_failed"] == true),
                Some("no_helper" | "needs_click" | "helper_missing") => counts.blocked += 1,
                Some(
                    "backend_error" | "timed_out" | "bad_output" | "too_long" | "context_too_long"
                    | "empty",
                ) => counts.failed += 1,
                _ => counts.unreadable += 1,
            }
        } else if value["id"].as_str().is_some() && value["generation"].is_number() {
            counts.pending += 1;
            let work = root
                .join("local/router-work")
                .join(path.file_stem().unwrap_or_default());
            for name in ["failure.json", "attempt.json"] {
                match store_io::read_bounded(&work.join(name), 64 * 1024) {
                    Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                        Ok(value)
                            if name == "failure.json"
                                || value["setup_failed"] == true
                                || value["child"]["success"] == false
                                || value["child"]["timed_out"] == true
                                || value["child"]["error"].is_string()
                                || value["published_fallback"] == true =>
                        {
                            counts.failed += 1;
                            break;
                        }
                        Ok(_) => {}
                        Err(_) => counts.unreadable += 1,
                    },
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(_) => counts.unreadable += 1,
                }
            }
        } else {
            counts.unreadable += 1;
        }
    }
    counts
}
fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
fn relay(state: &str) -> &'static str {
    match state {
        "disabled" => "disabled",
        "offline" => "offline",
        "connecting" => "connecting",
        "synced" => "connected",
        s if s.starts_with("failed:") => "failed",
        _ => "unknown",
    }
}
pub(crate) fn status(root: &Path, state: &sync_owner::Snapshot) -> Value {
    let router = queue(root, false);
    let notes = queue(root, true);
    let age = if state.observed_at_ms == 0 {
        None
    } else {
        Some(now().saturating_sub(state.observed_at_ms).max(0))
    };
    let ready = age.is_some_and(|age| age <= 5000)
        && state.errors == 0
        && (!state.relay_enabled || relay(&state.relay_status) == "connected")
        && router.unreadable + notes.unreadable + router.failed + notes.failed + notes.blocked == 0;
    let (requests, failures) = sync_owner::mcp_metrics();
    json!({"live":true,"ready":ready,"role":crate::service::role().name(),
        "observation_age_ms":age,"errors":state.errors,"meeting_sources":state.meetings.len(),
        "pending_sources":state.meetings.iter().filter(|m|m.pending).count(),
        "conflicted_sources":state.meetings.iter().filter(|m|m.conflict).count(),
        "peers":{"configured":state.peers,"transport":relay(&state.relay_status),
            "last_connected_observation_ms":state.last_connected_observation_ms},
        "queues":{"router":router,"notes":notes},"mcp":{"requests":requests,"failures":failures}})
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Event {
    at_ms: i64,
    event: EventKind,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EventKind {
    ServiceTick,
    NotesChanged,
    WorkerChanged,
}
pub(crate) fn record(root: &Path, event: EventKind) -> io::Result<()> {
    let path = root.join("local/operations/events.json");
    let _lock = store_io::lock(&root.join("local/operations/log.lock"))?;
    let mut events: Vec<Event> = match store_io::read_bounded(&path, 64 * 1024) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e),
    };
    if events.len() >= 128 {
        events.drain(..events.len() - 127);
    }
    events.push(Event {
        at_ms: now(),
        event,
    });
    store_io::write(&path, &events)
}
pub(crate) fn logs(root: &Path) -> io::Result<Value> {
    let path = root.join("local/operations/events.json");
    match store_io::read_bounded(&path, 64 * 1024) {
        Ok(bytes) => Ok(serde_json::to_value(serde_json::from_slice::<Vec<Event>>(
            &bytes,
        )?)?),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(json!([])),
        Err(e) => Err(e),
    }
}
pub fn cli(args: &[std::ffi::OsString]) -> i32 {
    let kind = args
        .windows(2)
        .find(|p| p[0] == "--ops")
        .and_then(|p| p[1].to_str());
    let Some(kind @ ("status" | "logs")) = kind else {
        eprintln!("OPS_INVALID_COMMAND");
        return 2;
    };
    match sync_owner::local_request(&json!({"op":"operations","kind":kind})) {
        Ok(value) => {
            println!("{value}");
            i32::from(kind == "status" && value["ready"] != true)
        }
        Err(_) => {
            eprintln!("OPS_OWNER_UNAVAILABLE");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn router_child_failures_and_published_fallback_degrade_readiness() {
        let root = tempfile::tempdir().unwrap();
        let state = sync_owner::Snapshot {
            observed_at_ms: now(),
            ..Default::default()
        };
        let pending = root.path().join("local/router-queue");
        let work = root.path().join("local/router-work/synthetic");
        fs::create_dir_all(&pending).unwrap();
        fs::create_dir_all(&work).unwrap();
        fs::write(
            pending.join("synthetic.json"),
            r#"{"id":"synthetic","generation":1}"#,
        )
        .unwrap();
        for attempt in [
            json!({"child":{"success":false,"timed_out":false,"error":null},"published_fallback":true}),
            json!({"child":{"success":false,"timed_out":true,"error":"synthetic timeout"},"published_fallback":true}),
            json!({"child":{"success":true,"timed_out":false,"error":null},"published_fallback":true}),
            json!({"child":{"success":true,"timed_out":false,"error":"synthetic failure"},"published_fallback":false}),
        ] {
            fs::write(work.join("attempt.json"), attempt.to_string()).unwrap();
            let health = status(root.path(), &state);
            assert_eq!(health["queues"]["router"]["failed"], 1);
            assert_eq!(health["queues"]["router"]["pending"], 1);
            assert_eq!(health["ready"], false);
        }
        fs::write(work.join("attempt.json"), json!({"child":{"success":true,"timed_out":false,"error":null},"published_fallback":false}).to_string()).unwrap();
        let health = status(root.path(), &state);
        assert_eq!(health["queues"]["router"]["failed"], 0);
        assert_eq!(health["ready"], true);
    }
    #[test]
    fn successful_notes_with_failed_task_sorting_degrade_readiness() {
        let root = tempfile::tempdir().unwrap();
        let state = sync_owner::Snapshot {
            observed_at_ms: now(),
            ..Default::default()
        };
        let dir = root.path().join("local/notes-jobs");
        fs::create_dir_all(&dir).unwrap();
        for tasks_failed in [true, false] {
            fs::write(
                dir.join("synthetic.json"),
                json!({"state":"done","tasks_failed":tasks_failed}).to_string(),
            )
            .unwrap();
            let health = status(root.path(), &state);
            assert_eq!(
                health["queues"]["notes"]["failed"],
                usize::from(tasks_failed)
            );
            assert_eq!(health["queues"]["notes"]["pending"], 0);
            assert_eq!(health["ready"], !tasks_failed);
        }
    }
    #[test]
    fn health_counts_are_redacted_and_fail_closed() {
        let root = tempfile::tempdir().unwrap();
        let state = sync_owner::Snapshot {
            observed_at_ms: now(),
            relay_status: "failed:synthetic-canary".into(),
            identity: "synthetic-canary".into(),
            ..Default::default()
        };
        let dir = root.path().join("local/notes-jobs");
        fs::create_dir_all(&dir).unwrap();
        for (name, value) in [("a", "queued"), ("b", "backend_error"), ("c", "unknown")] {
            fs::write(
                dir.join(format!("{name}.json")),
                json!({"state":value,"message":"synthetic-canary"}).to_string(),
            )
            .unwrap();
        }
        let health = status(root.path(), &state);
        assert_eq!(health["queues"]["notes"]["pending"], 1);
        assert_eq!(health["queues"]["notes"]["failed"], 1);
        assert_eq!(health["queues"]["notes"]["unreadable"], 1);
        assert_eq!(health["ready"], false);
        assert!(!health.to_string().contains("synthetic-canary"));
        assert_eq!(status(root.path(), &Default::default())["ready"], false);
    }
    #[test]
    fn operations_log_is_bounded_and_schema_checked() {
        let root = tempfile::tempdir().unwrap();
        for _ in 0..140 {
            record(root.path(), EventKind::WorkerChanged).unwrap();
        }
        assert_eq!(logs(root.path()).unwrap().as_array().unwrap().len(), 128);
        fs::write(
            root.path().join("local/operations/events.json"),
            br#"[{"at_ms":0,"event":"worker_changed","secret":"synthetic-canary"}]"#,
        )
        .unwrap();
        assert!(logs(root.path()).is_err());
    }
}
