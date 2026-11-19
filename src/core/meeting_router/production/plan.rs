use super::super::fallback::{Interval, Reason, RoutingFallback};
use super::*;
use crate::core::lid::{self, Decision};
use std::collections::BTreeMap;

fn pcm(path: &Path, span: vad::Span) -> Result<Vec<f32>> {
    let mut reader = hound::WavReader::open(path)?;
    if span.0 >= span.1 || span.1 > u64::from(reader.duration()) {
        return Err("PCM interval exceeds pinned audio".into());
    }
    reader.seek(u32::try_from(span.0)?)?;
    Ok(reader
        .samples::<i16>()
        .take(usize::try_from(span.1 - span.0)?)
        .map(|v| v.map(|s| f32::from(s) / 32768.0))
        .collect::<std::result::Result<_, _>>()?)
}

fn own(mut range: ranges::AsrRange, piece: vad::Span) -> ranges::AsrRange {
    // Silence padding is disjoint. Own exactly the bytes recognized, bounded at seams.
    range.context = (range.context.0.max(piece.0), range.context.1.min(piece.1));
    range.members = vec![(0, range.context)];
    range
}

#[cfg(windows)]
pub(crate) fn produce(
    folder: &Path,
    work: &Path,
    config: &Config,
    origins: &BTreeMap<String, i64>,
    input_revision: &str,
    clocks: &mut BTreeMap<String, f64>,
    deadline: Option<Instant>,
) -> Result<(Plan, pieces::PieceManifest, Option<fs::File>)> {
    produce_plan(
        folder,
        work,
        config,
        origins,
        input_revision,
        clocks,
        PlanScope {
            owned: None,
            deadline,
        },
    )
}

#[cfg(target_os = "linux")]
pub(crate) fn produce_owned(
    folder: &Path,
    work: &Path,
    config: &Config,
    origins: &BTreeMap<String, i64>,
    input_revision: &str,
    clocks: &mut BTreeMap<String, f64>,
    owned: Option<vad::Span>,
) -> Result<(Plan, pieces::PieceManifest, Option<fs::File>)> {
    produce_plan(
        folder,
        work,
        config,
        origins,
        input_revision,
        clocks,
        PlanScope {
            owned,
            deadline: None,
        },
    )
}

struct PlanScope {
    owned: Option<vad::Span>,
    deadline: Option<Instant>,
}

fn produce_plan(
    folder: &Path,
    work: &Path,
    config: &Config,
    origins: &BTreeMap<String, i64>,
    input_revision: &str,
    clocks: &mut BTreeMap<String, f64>,
    PlanScope { owned, deadline }: PlanScope,
) -> Result<(Plan, pieces::PieceManifest, Option<fs::File>)> {
    remaining(deadline)?;
    let started = Instant::now();
    let mut manifest = pieces::plan(folder, origins, &config.pipeline)?;
    // Staging/normalization changes file identities. Cache against the parent's pinned input.
    manifest.source_revision = input_revision.to_owned();
    for track in &mut manifest.tracks {
        for piece in &mut track.pieces {
            piece.id = cache::key(
                &[],
                input_revision,
                &manifest.config_checksum,
                &serde_json::to_string(&(
                    &track.track,
                    track.origin_sample,
                    piece.start_sample,
                    piece.end_sample,
                ))?,
            );
        }
    }
    store_io::write(&work.join("pieces.json"), &manifest)?;
    clocks.insert("pieces".into(), started.elapsed().as_secs_f64());
    let store = model_catalog::ModelStore::new(config.pipeline.model_state_dir.clone());
    let started = Instant::now();
    remaining(deadline)?;
    let mut vad =
        vad::Silero::load(&store, &config.pipeline.runtime_dll).map_err(|_| "VAD unavailable")?;
    let mut tracks = Vec::new();
    for track in &manifest.tracks {
        let path = folder.join(format!("{}.wav", track.track));
        let mut state = vad::Checkpoint::default();
        let mut spans = Vec::new();
        for piece in &track.pieces {
            remaining(deadline)?;
            let audio = pcm(&path, (piece.start_sample, piece.end_sample))?;
            let mut start = piece.start_sample;
            for chunk in audio.chunks(16000) {
                remaining(deadline)?;
                let end = start + chunk.len() as u64;
                spans.extend(
                    vad.push(
                        &mut state,
                        (&manifest.source_revision, &track.track),
                        start,
                        16000,
                        chunk,
                        end == track.sample_count,
                    )
                    .map_err(|_| "VAD inference failed")?,
                );
                start = end;
            }
        }
        tracks.push((path, spans));
    }
    drop(vad);
    clocks.insert("vad".into(), started.elapsed().as_secs_f64());
    let started = Instant::now();
    let declarations = config
        .pipeline
        .after_stop_languages
        .as_ref()
        .ok_or("Missing explicit language routes")?;
    declarations.validate()?;
    let languages: Vec<_> = std::iter::once(&declarations.primary)
        .chain(&declarations.supporting)
        .cloned()
        .collect();
    let bypass = config
        .pipeline
        .language_mode
        .strip_prefix("forced:")
        .map(str::to_owned)
        .or_else(|| {
            (languages.len() == 1 || config.pipeline.language_mode == "off")
                .then(|| declarations.primary.clone())
        });
    let mut settings = config.labs.language_detection_settings.clone();
    settings.runtime_dll = config.pipeline.runtime_dll.clone();
    settings.validate()?;
    remaining(deadline)?;
    let mut detector = if bypass.is_none() {
        settings.model = store
            .installed_paths("anuvani-lid-int8")?
            .ok_or("Language pack unavailable")?[0]
            .clone();
        Some(lid::anuvani::load(&settings)?)
    } else {
        None
    };
    let mut output = Vec::new();
    let mut coverage = Vec::new();
    let mut uncertain: Vec<RoutingFallback> = Vec::new();
    #[cfg(windows)]
    let mut english_identity = None;
    #[cfg(target_os = "linux")]
    let english_identity: Option<(String, fs::File)> = None;
    #[cfg(windows)]
    let mut pin_seconds = 0.0;
    #[cfg(target_os = "linux")]
    let pin_seconds = 0.0;
    let mut detection_seconds = started.elapsed().as_secs_f64();
    let mut range_seconds = 0.0;
    for (track, (path, spans)) in manifest.tracks.iter().zip(tracks) {
        for piece in &track.pieces {
            let ownership = owned.map_or((piece.start_sample, piece.end_sample), |(a, b)| {
                (a.max(piece.start_sample), b.min(piece.end_sample))
            });
            if ownership.0 >= ownership.1 {
                continue;
            }
            // Split long speech before LID so padded inference remains bounded to 24 seconds.
            let mut intervals = Vec::new();
            for &(start, end) in &spans {
                let mut start = start.max(ownership.0);
                let end = end.min(ownership.1);
                while start < end {
                    let stop = (start + 23 * 16000).min(end);
                    intervals.push((start, stop));
                    start = stop;
                }
            }
            let contexts = ranges::padded((0, track.sample_count), ownership, &intervals)?;
            let mut decisions = Vec::new();
            for &context in &contexts {
                remaining(deadline)?;
                #[cfg(target_os = "linux")]
                if crate::core::speech_jobs::STOP.load(std::sync::atomic::Ordering::Acquire) {
                    return Err("CANCELLED".into());
                }
                let started = Instant::now();
                let decision = if let Some(language) = &bypass {
                    if !languages.contains(language) {
                        return Err("Forced language is undeclared".into());
                    }
                    Decision::Accepted {
                        language: language.clone(),
                        confidence: 1.0,
                        margin: 1.0,
                        calibration: lid::CalibrationKey {
                            model_sha256: String::new(),
                            declared_set: languages.join("+"),
                            version: "declared".into(),
                        },
                    }
                } else {
                    let logits = detector
                        .as_mut()
                        .ok_or("Missing detector")?
                        .logits(&pcm(&path, context)?)?;
                    lid::decide_logits(
                        &logits,
                        &languages,
                        lid::MODEL_SHA256,
                        lid::CALIBRATION_VERSION,
                        &settings,
                    )
                };
                decisions.push(decision);
                if started.elapsed().as_secs_f64()
                    > f64::from(config.engine_settings.stages.lid_request_deadline_seconds)
                {
                    return Err("Language request deadline expired".into());
                }
                detection_seconds += started.elapsed().as_secs_f64();
            }
            let started = Instant::now();
            let pin_before = pin_seconds;
            let speech: Vec<_> = intervals
                .iter()
                .zip(&decisions)
                .map(|(&interval, decision)| ranges::Speech {
                    interval,
                    decision,
                    speaker_turn: None,
                })
                .collect();
            for range in ranges::plan((0, track.sample_count), ownership, &speech)? {
                remaining(deadline)?;
                let rejected: Vec<_> = range
                    .members
                    .iter()
                    .filter_map(|(index, _)| match decisions.get(*index)? {
                        Decision::Accepted { .. } => None,
                        Decision::Unsure(reason) => Some(match reason {
                            lid::Unsure::GuardRejected => Reason::GuardRejected,
                            lid::Unsure::Uncalibrated => Reason::Uncalibrated,
                            lid::Unsure::InvalidScores => Reason::InvalidScores,
                        }),
                    })
                    .collect();
                let range = own(range, ownership);
                coverage.push(Chunk {
                    id: format!("{}-{}", piece.id, range.context.0),
                    start: (track.origin_sample as f64 + range.context.0 as f64) / 16000.0,
                    end: (track.origin_sample as f64 + range.context.1 as f64) / 16000.0,
                    track: track.track.clone(),
                    speaker: None,
                    turn: None,
                    english: String::new(),
                    audio_start: None,
                    words: Vec::new(),
                });
                let Some(language) = &range.language else {
                    let c = coverage.last().ok_or("Missing uncertain interval")?;
                    for reason in rejected {
                        uncertain.push(RoutingFallback {
                            reason,
                            interval: Some(Interval {
                                track: c.track.clone(),
                                start: c.start,
                                end: c.end,
                            }),
                        });
                    }
                    continue;
                };
                let model_id = config
                    .engine_model(language)
                    .ok_or("Missing configured model")?
                    .to_owned();
                let model_sha256 = if language == "en" && model_id == "parakeet_unified_en" {
                    #[cfg(target_os = "linux")]
                    {
                        crate::core::speech_sessions::digest(b"host-english-deferred-v1")
                    }
                    #[cfg(windows)]
                    {
                        if english_identity.is_none() {
                            let pin_started = Instant::now();
                            english_identity = Some(super::pipeline::english_pin(config)?);
                            pin_seconds += pin_started.elapsed().as_secs_f64();
                        }
                        english_identity
                            .as_ref()
                            .ok_or("Missing English identity")?
                            .0
                            .clone()
                    }
                } else {
                    model_catalog::model(&model_id)?.files[0].sha256.to_owned()
                };
                let id = cache::key(&[], &piece.id, &serde_json::to_string(&range)?, &model_id);
                let wav = work.join(format!("{id}.wav"));
                // Derived PCM is disposable: never bless cached damage into a new plan.
                let slice = tempfile::NamedTempFile::new_in(work)?.into_temp_path();
                asr::write_wav(&slice, &pcm(&path, range.context)?)?;
                #[cfg(all(test, target_os = "linux"))]
                crate::service::speech::proofs::crash(&slice, "cut");
                store_io::atomic_write(&wav, &inputs::Snapshot::read_file(&slice, 1 << 20)?)?;
                output.push(Range {
                    track: track.track.clone(),
                    piece: piece.id.clone(),
                    wav,
                    range,
                    model_id,
                    model_sha256,
                });
            }
            range_seconds += started.elapsed().as_secs_f64() - (pin_seconds - pin_before);
        }
    }
    drop(detector);
    clocks.insert("detection".into(), detection_seconds);
    clocks.insert("ranges".into(), range_seconds);
    clocks.insert("model_pin".into(), pin_seconds);
    store_io::write(&work.join("coverage.json"), &coverage)?;
    store_io::write(&work.join("uncertainty.json"), &uncertain)?;
    let started = Instant::now();
    let plan = spool(work, output)?;
    remaining(deadline)?;
    clocks.insert("spool".into(), started.elapsed().as_secs_f64());
    Ok((plan, manifest, english_identity.map(|(_, file)| file)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn untimed_inputs_are_disjoint_and_never_claim_a_smaller_interval() {
        let decision = Decision::Accepted {
            language: "hi".into(),
            confidence: 1.0,
            margin: 1.0,
            calibration: lid::CalibrationKey {
                model_sha256: String::new(),
                declared_set: "hi".into(),
                version: "declared".into(),
            },
        };
        let speech: Vec<_> = [(1000, 8000), (9000, 16000), (17000, 40000)]
            .into_iter()
            .map(|interval| ranges::Speech {
                interval,
                decision: &decision,
                speaker_turn: None,
            })
            .collect();
        let mut end = 0;
        for r in ranges::plan((0, 100000), (0, 40000), &speech).unwrap() {
            let r = own(r, (0, 40000));
            assert!(r.context.0 >= end);
            assert!(r.context.1 <= 40000);
            assert_eq!(r.members, vec![(0, r.context)]);
            end = r.context.1;
        }
        assert_eq!(end, 40000);
    }
    #[test]
    fn pcm_does_not_accept_truncated_or_reversed_requests() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("public.wav");
        asr::write_wav(&path, &[0.0; 1600]).unwrap();
        assert_eq!(pcm(&path, (1599, 1600)).unwrap(), vec![0.0]);
        assert!(pcm(&path, (0, 1601)).is_err());
        assert!(pcm(&path, (100, 99)).is_err());
    }
}
