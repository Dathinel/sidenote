#[cfg(feature = "asr-runtime")]
use super::super::fallback::RoutingFallback;
use super::*;
#[cfg(feature = "asr-runtime")]
use std::collections::BTreeMap;

#[cfg(all(test, feature = "asr-runtime"))]
#[test]
fn production_input_failure_retains_partial_clocks() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("public-missing-input");
    let work = root.path().join("work");
    fs::create_dir_all(&folder).unwrap();
    fs::create_dir_all(&work).unwrap();
    let error = run(&folder, &[], &[], &Config::default(), &work, "public", 1)
        .unwrap_err()
        .to_string();
    let value: Value = serde_json::from_slice(
        &fs::read(work.join("clocks.json")).expect("failed production run must retain its clocks"),
    )
    .unwrap();
    assert_eq!(value["partial"], true);
    assert!(value["worker_total"].as_f64().unwrap() >= 0.0);
    assert!(!work.join("output.json").exists(), "{error}");
}

#[cfg(all(test, feature = "asr-runtime"))]
#[test]
#[ignore = "requires the public assembly-repro directory in SIDENOTE_ASSEMBLY_REPRO"]
fn saved_public_assembly_failure_retains_phase_clocks() {
    let saved = PathBuf::from(std::env::var_os("SIDENOTE_ASSEMBLY_REPRO").unwrap());
    let temp = tempfile::tempdir().unwrap();
    let work = temp.path();
    for entry in fs::read_dir(saved).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), work.join(entry.file_name())).unwrap();
    }
    let read = |name: &str| fs::read(work.join(name)).unwrap();
    let plan: Plan = serde_json::from_slice(&read("batch-plan.json")).unwrap();
    let pieces = serde_json::from_slice(&read("pieces.json")).unwrap();
    let baseline: Vec<Chunk> = serde_json::from_slice(&read("baseline.json")).unwrap();
    fs::write(
        work.join(format!("{}.plan.json", plan.revision)),
        serde_json::to_vec(&plan).unwrap(),
    )
    .unwrap();
    fs::remove_file(work.join(format!(
        "{}-38d0482dac119209983b83022fd4c7d3f68eb8b4bb8a3aa14cbcb456b275c7ff.receipt",
        plan.revision
    )))
    .unwrap();
    let phases = [
        "pieces",
        "vad",
        "detection",
        "ranges",
        "model_pin",
        "spool",
        "batches",
    ];
    let error = with_clocks(work, |clocks, _| {
        // Distinct sentinels stand for already completed phases, not measured times.
        clocks.extend(
            phases
                .iter()
                .enumerate()
                .map(|(i, p)| ((*p).into(), i as f64)),
        );
        revision::assemble(work, &plan, &pieces, &baseline, 1).map(|_| ())
    })
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("Owned interval lacks routed or baseline coverage"),
        "{error}"
    );
    let clocks: Value = serde_json::from_slice(&read("clocks.json")).unwrap();
    assert_eq!(clocks["partial"], true);
    for (i, p) in phases.iter().enumerate() {
        assert_eq!(clocks[*p], i as f64);
    }
    assert!(!work.join("output.json").exists());
}

/// Called only by the Labs job. The parent rechecks the captured input before publication.
pub fn run(
    folder: &Path,
    paths: &[(String, PathBuf)],
    baseline: &[Chunk],
    config: &Config,
    work: &Path,
    input_revision: &str,
    generation: u128,
) -> Result<()> {
    #[cfg(feature = "asr-runtime")]
    {
        with_clocks(work, |clocks, started| {
            if !config.validate_engine_settings().is_empty() {
                return Err("Invalid engine settings".into());
            }
            let normalized = work.join("normalized");
            fs::create_dir_all(&normalized)?;
            let input = inputs::Snapshot::capture(folder)?;
            store_io::atomic_write(&normalized.join("manifest.json"), &input.manifest)?;
            store_io::atomic_write(&normalized.join("transcript.ndjson"), &input.transcript)?;
            let manifest: Value = serde_json::from_slice(&input.manifest)?;
            let mut origins = BTreeMap::new();
            for (track, path) in paths {
                fs::copy(path, normalized.join(format!("{track}.wav")))?;
                origins.insert(track.clone(), origin(&manifest[track]["first_arrival_us"])?);
            }
            let (plan, pieces, _english_pin) =
                plan::produce(&normalized, work, config, &origins, input_revision, clocks)?;
            store_io::write(&work.join("batch-plan.json"), &plan)?;
            let mut bridge = Reuse {
                bridge: english_http::bridge(
                    hex_speech(),
                    work,
                    Duration::from_secs(u64::from(
                        config.engine_settings.stages.asr_request_deadline_seconds,
                    )),
                ),
                baseline,
                origins: &origins,
            };
            let stages = &config.engine_settings.stages;
            let audio_seconds: f64 = plan
                .batches
                .values()
                .flat_map(|b| b.values())
                .map(|r| (r.range.context.1 - r.range.context.0) as f64 / 16000.0)
                .sum();
            let seconds = config
                .engine_settings
                .stage_deadline_seconds
                .filter(|s| *s > 0)
                .map(f64::from)
                .unwrap_or(
                    (audio_seconds
                        * stages.asr_seconds_per_audio_second_per_pass
                        * stages.asr_deadline_headroom
                        + 15.0)
                        .clamp(30.0, 7200.0),
                );
            clocks.insert("asr_budget_seconds".into(), seconds);
            run_batches(
                work,
                &plan,
                &mut Native(config),
                &mut bridge,
                Duration::from_secs_f64(seconds),
                clocks,
                started,
            )?;
            let assemble_started = Instant::now();
            let assembled = assemble_saved(work, &plan, &pieces, baseline, generation);
            clocks.insert("assembly".into(), assemble_started.elapsed().as_secs_f64());
            let revision = assembled?;
            let fallbacks = super::super::fallback::read(&revision)?;
            store_io::write(
                &work.join("output.json"),
                &jobs::BatchOutput {
                    generation: generation.to_string(),
                    source_revision: input_revision.to_owned(),
                    plan,
                    manifest: pieces,
                    fallbacks,
                },
            )?;
            Ok(())
        })
    }
    #[cfg(not(feature = "asr-runtime"))]
    {
        let _ = (
            folder,
            paths,
            baseline,
            config,
            work,
            input_revision,
            generation,
        );
        Err("Production routing requires asr-runtime".into())
    }
}

/// The same assembly and full-speech guard used after live recognition; opens no audio.
#[cfg(feature = "asr-runtime")]
pub fn assemble_saved(
    work: &Path,
    plan: &Plan,
    pieces: &pieces::PieceManifest,
    baseline: &[Chunk],
    generation: u128,
) -> Result<revision::Revision> {
    let mut revision = revision::assemble(work, plan, pieces, baseline, generation)?;
    revision.provenance["english_transport"] = serde_json::from_slice(&store_io::read_bounded(
        &work.join("english-transport.json"),
        65536,
    )?)?;
    let fallbacks: Vec<RoutingFallback> = serde_json::from_slice(&inputs::Snapshot::read_file(
        &work.join("uncertainty.json"),
        16 << 20,
    )?)?;
    super::super::fallback::attach(&mut revision, &fallbacks)?;
    let coverage: Vec<Chunk> = serde_json::from_slice(&store_io::read_bounded(
        &work.join("coverage.json"),
        16 << 20,
    )?)?;
    if let Some(c) = coverage
        .iter()
        .find(|c| !super::super::baseline::source_covered(c, &revision.transcript.segments))
    {
        return Err(format!(
            "Speech lacks routed or explicit baseline coverage: id={} track={} seconds=[{},{}]",
            c.id, c.track, c.start, c.end
        )
        .into());
    }
    Ok(revision)
}

#[cfg(feature = "asr-runtime")]
fn with_clocks(
    work: &Path,
    run: impl FnOnce(&mut BTreeMap<String, f64>, Instant) -> Result<()>,
) -> Result<()> {
    let started = Instant::now();
    let mut clocks = BTreeMap::new();
    let result = run(&mut clocks, started);
    clocks.insert("worker_total".into(), started.elapsed().as_secs_f64());
    let mut value = serde_json::to_value(clocks)?;
    if result.is_err() {
        value["partial"] = true.into();
    }
    match (result, store_io::write(&work.join("clocks.json"), &value)) {
        (Err(error), Err(clock_error)) => {
            Err(format!("{error}; clocks could not be written: {clock_error}").into())
        }
        (Err(error), Ok(())) => Err(error),
        (Ok(()), saved) => Ok(saved?),
    }
}

#[cfg(feature = "asr-runtime")]
pub(super) fn run_batches(
    work: &Path,
    plan: &Plan,
    native: &mut impl NativeLoader,
    bridge: &mut impl EnglishBridge,
    timeout: Duration,
    clocks: &mut BTreeMap<String, f64>,
    started: Instant,
) -> Result<()> {
    let batches_started = Instant::now();
    let result = batches::run_until(work, plan, native, bridge, timeout);
    let cleanup = bridge.finish();
    let result = result.and(cleanup);
    clocks.insert("batches".into(), batches_started.elapsed().as_secs_f64());
    if let Err(error) = result {
        clocks.insert("worker_total".into(), started.elapsed().as_secs_f64());
        let mut partial = serde_json::to_value(clocks)?;
        partial["partial"] = true.into();
        if let Err(clock_error) = store_io::write(&work.join("clocks.json"), &partial) {
            return Err(
                format!("{error}; partial clocks could not be written: {clock_error}").into(),
            );
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(feature = "asr-runtime")]
fn origin(value: &Value) -> Result<i64> {
    // Match Hex's transcript contract, including empty/legacy tracks with no arrival.
    let us = if value.is_null() {
        0
    } else {
        value.as_u64().ok_or("Invalid capture track origin")?
    };
    Ok(i64::try_from(
        (us / 1000).checked_mul(16).ok_or("Origin overflow")?,
    )?)
}

#[cfg(feature = "asr-runtime")]
pub(super) fn english_pin(config: &Config) -> Result<(String, fs::File)> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let model = config.engine_model("en").ok_or("Missing English model")?;
    let status = hex_speech().run_for(
        &["model", "status", model, "--language", "en"],
        None,
        Duration::from_secs(2),
    )?;
    let fields: Vec<_> = status.trim().splitn(4, '\t').collect();
    if fields.len() != 4 || fields[0] != "installed" {
        return Err("English model identity unavailable".into());
    }
    let path = Path::new(fields[3]);
    if !path.is_absolute() {
        return Err("Invalid English model path".into());
    }
    let mut file = inputs::Snapshot::open_file(path)?;
    if !(1..=(2u64 << 30)).contains(&file.metadata()?.len()) {
        return Err("Invalid English model size".into());
    }
    let mut digest = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    Ok((::hex::encode(digest.finalize()), file))
}

#[cfg(feature = "asr-runtime")]
fn hex_speech() -> HexSpeech {
    let args: Vec<_> = std::env::args_os().collect();
    args.windows(2)
        .find(|a| a[0] == "--hex-cli")
        .map(|a| HexSpeech {
            executable: PathBuf::from(&a[1]),
        })
        .unwrap_or_default()
}

#[cfg(feature = "asr-runtime")]
struct Reuse<'a> {
    bridge: English<english_http::RunHex>,
    baseline: &'a [Chunk],
    origins: &'a BTreeMap<String, i64>,
}
#[cfg(feature = "asr-runtime")]
impl EnglishBridge for Reuse<'_> {
    fn finish(&mut self) -> Result<()> {
        self.bridge.finish()
    }
    fn limit(&mut self, remaining: Duration) {
        self.bridge.limit(remaining);
    }
    fn transcribe(&mut self, range: &Range) -> Result<Value> {
        let origin = self.origins[&range.track] as f64 / 16000.0;
        let start = origin + range.range.context.0 as f64 / 16000.0;
        let end = origin + range.range.context.1 as f64 / 16000.0;
        let mut chunks: Vec<_> = self
            .baseline
            .iter()
            .filter(|b| {
                b.track == range.track
                    && b.start >= start
                    && b.end <= end
                    && !b.english.trim().is_empty()
            })
            .collect();
        chunks.sort_by(|a, b| a.start.total_cmp(&b.start));
        let mut cursor = start;
        for chunk in &chunks {
            if chunk.start > cursor {
                break;
            }
            cursor = cursor.max(chunk.end);
        }
        if cursor >= end {
            return Ok(
                json!({"text":chunks.iter().map(|b| b.english.as_str()).collect::<Vec<_>>().join(" "),
                "timing":"range", "reused_baseline":true, "start_sample":range.range.context.0,"end_sample":range.range.context.1}),
            );
        }
        self.bridge.transcribe(range)
    }
}

#[cfg(all(test, feature = "asr-runtime"))]
mod tests {
    use super::*;
    #[test]
    fn production_failed_batch_persists_partial_clocks() {
        use super::super::no_speech_tests::{Fixture, NativeFake};
        let mut f = Fixture::new(b"", 7, b"recognizer allocation failed");
        let mut native = NativeFake::default();
        let previous = BTreeMap::from([
            ("pieces".into(), 0.1),
            ("vad".into(), 0.2),
            ("detection".into(), 0.3),
            ("asr_budget_seconds".into(), 120.0),
        ]);
        let mut clocks = previous.clone();
        let error = run_batches(
            &f.work,
            &f.plan,
            &mut native,
            &mut f.bridge,
            Duration::from_secs(120),
            &mut clocks,
            Instant::now(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("exit=7"));
        assert!(native.0.is_empty());
        let saved: Value = serde_json::from_slice(
            &store_io::read_bounded(&f.work.join("clocks.json"), 65536)
                .expect("failed run must persist partial clocks"),
        )
        .unwrap();
        assert_eq!(saved["partial"], true);
        for (name, seconds) in previous {
            assert_eq!(saved[&name], seconds);
        }
        assert!(saved["batches"].as_f64().unwrap() >= 0.0);
        assert!(saved["worker_total"].as_f64().unwrap() >= saved["batches"].as_f64().unwrap());
        assert!(saved.get("assembly").is_none());
        assert!(!f.work.join("output.json").exists());
    }
    #[test]
    fn empty_track_origin_matches_capture_contract_without_inferring_from_length() {
        assert_eq!(origin(&Value::Null).unwrap(), 0);
        assert_eq!(origin(&json!(1_234_567)).unwrap(), 19_744);
        assert!(origin(&json!(-1)).is_err());
        assert!(origin(&json!("unknown")).is_err());
    }
    #[test]
    fn english_reuses_only_complete_matching_track_intervals() {
        let range: Range =
            serde_json::from_value(json!({"track":"system","piece":"p","wav":"missing",
            "range":{"context":[16000,48000],"members":[[0,[16000,48000]]],"language":"en"},
            "model_id":"parakeet_unified_en","model_sha256":"0".repeat(64)}))
            .unwrap();
        let chunk = |start, end, track: &str, text: &str| Chunk {
            id: text.into(),
            start,
            end,
            track: track.into(),
            speaker: None,
            turn: None,
            english: text.into(),
            audio_start: None,
        };
        let baseline = [
            chunk(1.5, 2.5, "system", "public one"),
            chunk(2.5, 3.5, "system", "public two"),
            chunk(0.0, 10.0, "microphone", "wrong track"),
        ];
        let origins = BTreeMap::from([("system".into(), 8000)]);
        let transport = tempfile::tempdir().unwrap();
        let mut bridge = Reuse {
            bridge: english_http::bridge(
                HexSpeech {
                    executable: PathBuf::from("never-call"),
                },
                transport.path(),
                Duration::from_secs(1),
            ),
            baseline: &baseline,
            origins: &origins,
        };
        let value = bridge.transcribe(&range).unwrap();
        assert_eq!(value["text"], "public one public two");
        assert_eq!(value["reused_baseline"], true);
        bridge.baseline = &baseline[1..];
        assert!(bridge.transcribe(&range).is_err());
    }
}
