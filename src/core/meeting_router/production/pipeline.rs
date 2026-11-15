#[cfg(feature = "asr-runtime")]
use super::super::fallback::RoutingFallback;
use super::*;
#[cfg(feature = "asr-runtime")]
use std::collections::BTreeMap;

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
        if !config.validate_engine_settings().is_empty() {
            return Err("Invalid engine settings".into());
        }
        let started = Instant::now();
        let mut clocks = BTreeMap::new();
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
        let (plan, pieces, _english_pin) = plan::produce(
            &normalized,
            work,
            config,
            &origins,
            input_revision,
            &mut clocks,
        )?;
        store_io::write(&work.join("batch-plan.json"), &plan)?;
        let mut bridge = Reuse {
            bridge: English(
                HexBridge::new(hex_speech()).with_timeout(Duration::from_secs(u64::from(
                    config.engine_settings.stages.asr_request_deadline_seconds,
                ))),
            ),
            baseline,
            origins: &origins,
        };
        let batches_started = Instant::now();
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
        batches::run_until(
            work,
            &plan,
            &mut Native(config),
            &mut bridge,
            Duration::from_secs_f64(seconds),
        )?;
        clocks.insert("batches".into(), batches_started.elapsed().as_secs_f64());
        let assemble_started = Instant::now();
        let mut revision = revision::assemble(work, &plan, &pieces, baseline, generation)?;
        let fallbacks: Vec<RoutingFallback> = serde_json::from_slice(
            &inputs::Snapshot::read_file(&work.join("uncertainty.json"), 16 << 20)?,
        )?;
        super::super::fallback::attach(&mut revision, &fallbacks)?;
        let coverage: Vec<Chunk> = serde_json::from_slice(&store_io::read_bounded(
            &work.join("coverage.json"),
            16 << 20,
        )?)?;
        if coverage
            .iter()
            .any(|c| !super::super::baseline::source_covered(c, &revision.transcript.segments))
        {
            return Err("Speech lacks routed or explicit baseline coverage".into());
        }
        clocks.insert("assembly".into(), assemble_started.elapsed().as_secs_f64());
        clocks.insert("worker_total".into(), started.elapsed().as_secs_f64());
        store_io::write(&work.join("clocks.json"), &clocks)?;
        jobs::BatchOutput::publish(
            &work.join("output.json"),
            generation,
            input_revision,
            plan,
            pieces,
            fallbacks,
        )?;
        Ok(())
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
    bridge: English,
    baseline: &'a [Chunk],
    origins: &'a BTreeMap<String, i64>,
}
#[cfg(feature = "asr-runtime")]
impl EnglishBridge for Reuse<'_> {
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
        let mut bridge = Reuse {
            bridge: English(HexBridge::new(HexSpeech {
                executable: PathBuf::from("never-call"),
            })),
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
