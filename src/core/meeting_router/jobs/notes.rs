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
#[cfg(all(test, windows))]
pub(crate) fn notes_input(
    root: &Path,
    id: &str,
    chosen: Option<&str>,
    baseline: impl FnOnce() -> io::Result<Vec<Chunk>>,
    ended_ms: i64,
    now_ms: i64,
) -> io::Result<Option<(String, Transcript, Vec<RoutingFallback>)>> {
    notes_input_sized(root, id, chosen, baseline, ended_ms, &|| now_ms, None)
}
/// Resolve a durable dependency to immutable text. This function performs no audio work.
pub(crate) fn notes_input_sized(
    root: &Path,
    id: &str,
    chosen: Option<&str>,
    baseline: impl FnOnce() -> io::Result<Vec<Chunk>>,
    ended_ms: i64,
    now: &dyn Fn() -> i64,
    duration_ms: Option<f64>,
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
    // An unknown length keeps the old wait; a known one waits only while short (0 minutes: never).
    let mut stages = config.engine_settings.stages.clone();
    // A saved wait outside its range falls back to its default; notes must still publish.
    let (bad, def) = (stages.errors(), Config::default().engine_settings.stages);
    if bad.contains(&"stages.notes_wait_seconds") {
        stages.notes_wait_seconds = def.notes_wait_seconds;
    }
    if bad.contains(&"stages.notes_wait_max_minutes") {
        stages.notes_wait_max_minutes = def.notes_wait_max_minutes;
    }
    let max_ms = f64::from(stages.notes_wait_max_minutes) * 60_000.0;
    let seconds = match duration_ms {
        None => stages.routing_deadline_seconds,
        Some(ms) if ms <= max_ms && max_ms > 0.0 => stages.notes_wait_seconds,
        Some(_) => 0,
    };
    if seconds > 3600 || (seconds == 0 && duration_ms.is_none()) {
        return Err(store_io::invalid("Invalid routing deadline"));
    }
    // A zero wait never defers, whatever the clock says; the release check below stays separate.
    let english_first = seconds == 0;
    let deadline = if english_first {
        i64::MIN
    } else {
        ended_ms.saturating_add(i64::from(seconds) * 1000)
    };
    let _lock = pending_lock(root, id)?;
    // Sampled under the lock, so a release published while we waited for it is not "from the future".
    let now_ms = now();
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
        && (english_first || released.completed_ms <= deadline)
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
    let mut attempt_failure = None;
    let generation = if let Some(p) = &pending {
        #[derive(Deserialize)]
        struct Attempt {
            generation: u128,
            child: Outcome,
            cpu_models_ready: Option<u8>,
        }
        attempt_failure = super::super::inputs::Snapshot::read_file(
            &root.join("local/router-work").join(id).join("attempt.json"),
            file_reads::JOURNAL_MAX,
        )
        .ok()
        .and_then(|b| serde_json::from_slice::<Attempt>(&b).ok())
        .filter(|a| a.generation == p.generation && !a.child.success)
        .map(|a| match a.child.error.as_deref() {
            Some(CPU_ASSETS_UNAVAILABLE) => match a.cpu_models_ready {
                Some(ready @ 0..=3) => Reason::CpuLanguageModelsMissing { ready },
                _ => Reason::CpuLanguageAssetsUnavailable,
            },
            _ => Reason::RoutingFailed,
        });
        if attempt_failure.is_none() && now_ms < deadline {
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
        let reason = attempt_failure.unwrap_or(if english_first {
            Reason::EnglishFirst
        } else {
            Reason::DeadlineExpired
        });
        reasons.push(RoutingFallback::new(reason));
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

#[cfg(all(test, windows, feature = "asr-runtime"))]
#[test]
fn dev40_setup_required_keeps_the_capture_and_shows_the_count() {
    use crate::core::model_catalog::ModelStore;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let store = ModelStore::new(root.to_owned());
    store_io::write(&root.join("config.json"), &Config::load_in(root).unwrap()).unwrap();
    let capture =
        br#"{"source":"system","text":"Synthetic baseline survives.","start_ms":0,"end_ms":3000}"#;
    let partial = root.join("models/silero-vad/silero-vad.onnx.part");
    std::fs::create_dir_all(partial.parent().unwrap()).unwrap();
    std::fs::write(partial, b"partial download").unwrap();
    let ready = store.incomplete_cpu_language_pack_count();
    assert_eq!(ready, Some(0));
    for (id, count, wanted) in [
        (
            "100-1",
            ready,
            Reason::CpuLanguageModelsMissing { ready: 0 },
        ),
        ("100-2", None, Reason::CpuLanguageAssetsUnavailable),
    ] {
        let pending = Pending {
            id: id.into(),
            generation: 7,
        };
        store_io::write(&queue(root).join(format!("{id}.json")), &pending).unwrap();
        let work = root.join("local/router-work").join(id);
        let failure = CpuAssetsUnavailable(count);
        record_cpu_assets_failure(&work, pending.generation, &failure).unwrap();
        let (_, reading, reasons) = notes_input(
            root,
            id,
            None,
            || read_baseline(capture).map_err(|e| io::Error::other(e.to_string())),
            10_000,
            11_000,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            reading.segments[0].text_original,
            "Synthetic baseline survives."
        );
        assert_eq!(reasons, [RoutingFallback::new(wanted)]);
        assert!(
            reading
                .warnings
                .iter()
                .any(|text| text.as_str() == wanted.message().as_ref())
        );
        if id == "100-1" {
            assert!(reading.warnings.iter().any(|warning| {
                warning.contains("Hindi and Telugu models are not installed yet; download them once in Settings, Models (about 567 MB).")
                    && warning.contains("0 of 4 ready")
            }));
        }
    }
}
