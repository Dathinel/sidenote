use super::*;
use crate::core::{
    asr::Result,
    meeting_router::batches::{self, NativeLoader, Plan, Range},
    speech_sessions::PieceRequest,
};
use std::{
    path::Path,
    time::{Duration, Instant},
};
fn publish(
    store: &Store,
    session: &str,
    piece: &str,
    metadata: &PieceRequest,
    key: &str,
    range: &Range,
    reading: &Value,
) -> Result<()> {
    let mut reading = reading.clone();
    reading["piece"] = piece.into();
    reading["range"] = key.into();
    reading["track"] = metadata.track.as_str().into();
    reading["start_sample"] = (metadata.context_start_sample + range.range.context.0).into();
    reading["end_sample"] = (metadata.context_start_sample + range.range.context.1).into();
    reading["language"] = range.range.language.clone().into();
    reading["route"] = reading["language"].clone();
    reading["lang_conf"] = Value::Null;
    reading["model_sha256"] = range.model_sha256.as_str().into();
    store
        .emit(session, "range.result", reading)
        .map_err(|_| "EVENT_STORAGE")?;
    Ok(())
}
pub(crate) fn execute(
    store: &Store,
    session: &str,
    piece: &str,
    metadata: &PieceRequest,
    work: &Path,
    plan: &Plan,
    native: &mut impl NativeLoader,
) -> Result<()> {
    let started = Instant::now();
    let stages = crate::core::engine_settings::StageSettings::default();
    store
        .emit(
            session,
            "piece.progress",
            json!({"piece":piece,"stage":"asr","done":0,
        "of":plan.batches.values().map(|b| b.len()).sum::<usize>()}),
        )
        .map_err(|_| "EVENT_STORAGE")?;
    let result = batches::run_host(
        work,
        plan,
        native,
        Duration::from_secs_f64(
            (((metadata.end_sample - metadata.start_sample) as f64 / 16000.0)
                * stages.asr_seconds_per_audio_second_per_pass
                * stages.asr_deadline_headroom
                + 15.0)
                .clamp(30.0, 7200.0),
        ),
        &mut |key, range, reading| publish(store, session, piece, metadata, key, range, reading),
    );
    let mut count = 0;
    let mut failed = Vec::new();
    if STOP.load(std::sync::atomic::Ordering::Acquire) {
        return Err("CANCELLED".into());
    }
    let gaps: Vec<crate::core::meeting_router::fallback::RoutingFallback> =
        store_io::read(&work.join("uncertainty.json"))?;
    for gap in gaps {
        let key = crate::core::speech_sessions::digest(&serde_json::to_vec(&gap)?);
        let interval = gap.interval.ok_or("MISSING_GAP_INTERVAL")?;
        let data = json!({"piece":piece,"range":key,
            "code":"LID_UNCERTAIN","reason":gap.reason,"track":interval.track,
            "start_sample":metadata.context_start_sample + (interval.start*16000.0).round() as u64,
            "end_sample":metadata.context_start_sample + (interval.end*16000.0).round() as u64});
        store
            .emit(session, "range.failed", data.clone())
            .map_err(|_| "EVENT_STORAGE")?;
        failed.push(data);
    }
    let rejected = failed.len();
    for (language, batch) in &plan.batches {
        for (key, range) in batch {
            let reading = if language == "en" {
                Some(json!({"text":null,"timing":"range"}))
            } else {
                batches::read_result(work, plan, key)?
            };
            if let Some(reading) = reading {
                publish(store, session, piece, metadata, key, range, &reading)?;
                count += 1;
            } else {
                failed.push(json!({"range":key,"code":"ASR_UNAVAILABLE"}));
            }
        }
    }
    if result.is_err() && count == 0 && rejected == 0 {
        return Err("ASR_UNAVAILABLE".into());
    }
    store.emit(session,"piece.done",json!({"piece":piece,"ranges_total":count+failed.len(),"ranges_ok":count,
        "failed":failed,"audio_seconds":(metadata.end_sample-metadata.start_sample) as f64 / 16000.0,
        "asr_seconds":started.elapsed().as_secs_f64()})).map_err(|_| "EVENT_STORAGE")?;
    Ok(())
}
#[cfg(feature = "asr-runtime")]
pub(crate) fn process(
    store: &Store,
    id: &str,
    piece: &str,
    session: &crate::core::speech_sessions::Session,
    config: &crate::core::Config,
) -> Result<()> {
    use crate::core::meeting_router::{LanguageRoutes, ProviderRoute};
    let metadata = &session.pieces[piece].metadata;
    let work = store.0.join(id).join("jobs").join(piece);
    let input = work.join("input");
    let _job = {
        let _intake = store.load().map_err(|_| "EVENT_STORAGE")?;
        store_io::lock(&work.join("processing.lock"))?
    };
    let audio = store_io::read_bounded(
        &store.0.join(id).join(format!("{piece}.wav")),
        crate::core::speech_sessions::MAX_PIECE_BYTES as u64,
    )?;
    if crate::core::speech_sessions::digest(&audio) != metadata.sha256 {
        return Err("BAD_AUDIO".into());
    }
    store_io::atomic_write(&input.join(format!("{}.wav", metadata.track)), &audio)?;
    store_io::write(&input.join("manifest.json"), &json!({"status":"complete"}))?;
    store_io::atomic_write(&input.join("transcript.ndjson"), b"")?;
    let mut config = config.clone();
    config.pipeline.after_stop_languages = Some(LanguageRoutes {
        primary: session.request.languages[0].clone(),
        supporting: session.request.languages[1..].to_vec(),
        routes: session
            .request
            .languages
            .iter()
            .map(|l| {
                (
                    l.clone(),
                    ProviderRoute {
                        provider: if l == "en" { "hex" } else { "local" }.into(),
                        model: if l == "en" {
                            "parakeet_unified_en".into()
                        } else {
                            format!("indicconformer-{l}-ctc")
                        },
                    },
                )
            })
            .collect(),
    });
    config.pipeline.meeting_languages = session.request.languages.clone();
    config.pipeline.language_mode = session.request.language_mode.clone();
    let (plan, _, _) = crate::core::meeting_router::production::plan::produce_owned(
        &input,
        &work,
        &config,
        // Stage-local origins avoid losing sample precision in meeting-time gap metadata.
        &[(metadata.track.clone(), 0)].into(),
        piece,
        &mut Default::default(),
        Some((
            metadata.start_sample - metadata.context_start_sample,
            metadata.end_sample - metadata.context_start_sample,
        )),
    )?;
    execute(
        store,
        id,
        piece,
        metadata,
        &work,
        &plan,
        &mut crate::core::meeting_router::production::Native(&config),
    )
}

#[cfg(test)]
mod tests;
