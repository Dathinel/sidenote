use super::super::fallback::{self, Reason, RoutingFallback};
use super::*;
use crate::core::{Config, file_reads};

pub(super) fn enabled(root: &Path) -> io::Result<Option<Config>> {
    let bytes = match crate::core::config::read(root) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        other => other?,
    };
    let config: Config =
        serde_json::from_slice(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes))?;
    Ok((config.labs.after_stop_language_routing && config.pipeline.enabled()).then_some(config))
}
/// Resolve a durable dependency to immutable text. This function performs no audio work.
pub(crate) fn notes_input(
    root: &Path,
    id: &str,
    chosen: Option<&str>,
    baseline: impl FnOnce() -> io::Result<Vec<Chunk>>,
    ended_ms: i64,
    now_ms: i64,
) -> io::Result<Option<(String, Transcript, Vec<RoutingFallback>)>> {
    if !valid_id(id) {
        return Err(store_io::invalid("Invalid meeting id"));
    }
    let dir = destination(root, id).parent().unwrap().to_owned();
    let Some(config) = enabled(root).ok().flatten() else {
        return Ok(None);
    };
    if let Some(chosen) = chosen
        && let Ok(Some((id, r))) = revision::load(&dir, Some(chosen))
        && let Ok(reasons) = fallback::read(&r)
    {
        return Ok(Some((id, r.transcript, reasons)));
    }
    let seconds = config.engine_settings.stages.routing_deadline_seconds;
    if !(1..=3600).contains(&seconds) {
        return Err(store_io::invalid("Invalid routing deadline"));
    }
    let deadline = ended_ms.saturating_add(i64::from(seconds) * 1000);
    let _lock = pending_lock(root, id)?;
    let dependency =
        super::super::inputs::Snapshot::read_file(&queue(root).join(format!("{id}.json")), 4096)
            .and_then(|bytes| serde_json::from_slice::<Pending>(&bytes).map_err(io::Error::from));
    let (pending, pending_unusable) = match dependency {
        Ok(p) if p.id == id && p.generation > 0 => (Some(p), false),
        Err(e) if e.kind() == io::ErrorKind::NotFound => (None, false),
        _ => (None, true),
    };
    let waiting = || {
        io::Error::new(
            io::ErrorKind::WouldBlock,
            store_io::UserMessage("routing_pending".into()),
        )
    };
    // An unavailable pin must not switch to an unrelated routed revision.
    let require_hex = chosen.is_some() || pending_unusable;
    if require_hex && now_ms < deadline {
        return Err(waiting());
    }
    let baseline = baseline()?;
    let same_baseline =
        |t: &Transcript| serde_json::to_vec(&t.baseline).ok() == serde_json::to_vec(&baseline).ok();
    // An unchosen damaged selection is optional input. Preserve its bytes and wait/fall back.
    let loaded = revision::load(&dir, None);
    let latest_unavailable = loaded.is_err();
    let latest = loaded.ok().flatten();
    let released = super::super::inputs::Snapshot::read_file(
        &handoff::work(root, id).join("released.json"),
        4096,
    )
    .ok()
    .and_then(|b| serde_json::from_slice::<handoff::Released>(&b).ok());
    if !require_hex
        && latest.is_some()
        && let Some(released) = released
        && released.completed_ms >= 0
        && released.completed_ms <= deadline
        && released.completed_ms <= now_ms
        && pending
            .as_ref()
            .is_none_or(|p| p.generation.to_string() == released.generation)
        && let Some((chosen, r)) = revision::load(&dir, Some(&released.revision))
            .ok()
            .flatten()
        && r.generation == released.generation
        && r.transcript.input_checksum == released.source_revision
        && same_baseline(&r.transcript)
        && let Ok(reasons) = fallback::read(&r)
    {
        return Ok(Some((chosen, r.transcript, reasons)));
    }
    let mut failed_attempt = false;
    let generation = if let Some(p) = &pending {
        #[derive(Deserialize)]
        struct Attempt {
            generation: u128,
            child: Outcome,
        }
        let failed = super::super::inputs::Snapshot::read_file(
            &root.join("local/router-work").join(id).join("attempt.json"),
            file_reads::JOURNAL_MAX,
        )
        .ok()
        .and_then(|b| serde_json::from_slice::<Attempt>(&b).ok())
        .is_some_and(|a| a.generation == p.generation && !a.child.success);
        failed_attempt = failed;
        if !failed && now_ms < deadline {
            return Err(waiting());
        }
        p.generation
    } else {
        u128::try_from(now_ms).map_err(|_| store_io::invalid("Invalid deadline clock"))? * 1_000_000
    };
    revision::begin(&dir, generation)?;
    let mut t = english_fallback(
        &baseline,
        &config.pipeline,
        "compatibility".into(),
        "routing failed or deadline expired",
    );
    reconcile(&mut t, &baseline);
    let mut reasons = Vec::new();
    if chosen.is_some() {
        reasons.push(RoutingFallback::new(Reason::PinnedRevisionUnavailable));
    }
    if pending_unusable {
        reasons.push(RoutingFallback::new(Reason::PendingRecordInvalid));
    }
    if latest_unavailable && chosen.is_none() {
        reasons.push(RoutingFallback::new(Reason::SelectedRevisionUnavailable));
    }
    if reasons.is_empty() {
        reasons.push(RoutingFallback::new(if failed_attempt {
            Reason::RoutingFailed
        } else {
            Reason::DeadlineExpired
        }));
    }
    let mut data = revision::Revision {
        generation: generation.to_string(),
        plan_revision: "compatibility".into(),
        transcript: t,
        alternatives: vec![],
        evidence: Default::default(),
        provenance: if require_hex {
            serde_json::json!({"unavailable_revision":chosen,"pending_unusable":pending_unusable})
        } else {
            serde_json::Value::Null
        },
    };
    fallback::attach(&mut data, &reasons)?;
    let id = revision::retain(&dir, &data)?;
    Ok(Some((id, data.transcript, reasons)))
}
