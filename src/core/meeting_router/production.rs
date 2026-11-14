//! Adapters for the immutable batch protocol. Children own native model lifetimes.
use super::{batches::*, hex_bridge::*, *};
use crate::core::{Config, model_catalog};
use serde_json::{Value, json};
use std::{
    num::NonZeroUsize,
    process::Command,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

pub struct Native<'a>(pub &'a Config);
impl NativeLoader for Native<'_> {
    fn load(&mut self, language: NativeLanguage, range: &Range) -> Result<Box<dyn Recognizer>> {
        let language = match language {
            NativeLanguage::Hindi => "hi",
            NativeLanguage::Telugu => "te",
        };
        let config = self.0;
        let threads = config.engine_settings.worker_threads;
        if !(1..=256).contains(&threads) {
            return Err("Invalid worker threads".into());
        }
        let paths = model_catalog::ModelStore::new(config.pipeline.model_state_dir.clone())
            .installed_paths(&range.model_id)?
            .ok_or("Native pack unavailable")?;
        let mut command = Command::new(std::env::current_exe()?);
        command
            .args(["--routing-native", language, "--graph"])
            .arg(&paths[0])
            .arg("--tokens")
            .arg(&paths[1])
            .arg("--runtime-dll")
            .arg(&config.pipeline.runtime_dll);
        Ok(Box::new(worker::Worker::spawn_with_threads(
            command,
            Arc::new(AtomicBool::new(false)),
            Duration::from_secs(u64::from(
                config.engine_settings.stages.asr_request_deadline_seconds,
            )),
            NonZeroUsize::new(threads as usize).ok_or("Invalid threads")?,
        )?))
    }
}

pub struct English(pub HexBridge<HexSpeech>);
impl EnglishBridge for English {
    fn transcribe(&mut self, range: &Range) -> Result<Value> {
        let reading = self.0.transcribe(
            &range.wav,
            &HexRequest {
                range_id: range.piece.clone(),
                model_id: range.model_id.clone(),
                language: "en".into(),
            },
        )?;
        english_result(reading, range)
    }
}
fn english_result(reading: HexReading, range: &Range) -> Result<Value> {
    let mut value = json!({"text":reading.text, "start_sample":range.range.context.0,
        "end_sample":range.range.context.1, "timing":"range"});
    if let Some(segments) = reading.json.and_then(|v| v.get("segments").cloned()) {
        value["segments"] = segments;
        value["timing"] = "segment-seconds".into();
    }
    Ok(value)
}

/// Entry point runs before app startup; it receives no owner configuration/environment.
pub fn native_worker() -> Result<()> {
    #[cfg(feature = "asr-runtime")]
    {
        use crate::core::asr::SpeechEngine;
        use std::io::BufRead;
        let args: Vec<_> = std::env::args_os().collect();
        let arg = |flag: &str| -> Result<PathBuf> {
            Ok(args
                .windows(2)
                .find(|a| a[0] == flag)
                .map(|a| PathBuf::from(&a[1]))
                .ok_or("Missing native argument")?)
        };
        let language = arg("--routing-native")?.to_string_lossy().into_owned();
        let output =
            PathBuf::from(std::env::var_os("SIDENOTE_BATCH_OUTPUT").ok_or("Missing output")?);
        let threads: usize = std::env::var("SIDENOTE_BATCH_THREADS")?.parse()?;
        let stdin = std::io::stdin();
        let mut lines = stdin.lock().lines();
        if lines.next().transpose()?.as_deref() != Some("start") {
            return Err("Missing start".into());
        }
        asr::initialize(&arg("--runtime-dll")?)?;
        let graph = arg("--graph")?;
        let mut model =
            asr::IndicConformer::load_with_threads(&graph, &arg("--tokens")?, &language, threads)?;
        for line in lines {
            if line? != "range" {
                return Err("Invalid native request".into());
            }
            let range: Range = serde_json::from_slice(&store_io::read_bounded(
                &output.join("range.json"),
                1 << 20,
            )?)?;
            if range.range.language.as_deref() != Some(&language)
                || model_catalog::model(&range.model_id)?.files[0].sha256 != range.model_sha256
            {
                return Err("Native route mismatch".into());
            }
            let samples = asr::read_wav(&range.wav)?;
            let reading = model.transcribe(&samples, 16000, &language)?;
            store_io::write(
                &output.join("result.json"),
                &json!({"text":reading.text,
                "timing":"range", "start_sample":range.range.context.0, "end_sample":range.range.context.1}),
            )?;
        }
        Ok(())
    }
    #[cfg(not(feature = "asr-runtime"))]
    Err("Native routing requires asr-runtime".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plain_hex_stamps_own_range_and_json_retains_segments() {
        let range: Range =
            serde_json::from_value(json!({"track":"system","piece":"piece","wav":"unused",
            "range":{"context":[16000,48000],"members":[[0,[16000,48000]]],"language":"en"},
            "model_id":"parakeet_unified_en","model_sha256":"0".repeat(64)}))
            .unwrap();
        let reading = |json| HexReading {
            request: HexRequest {
                range_id: "piece".into(),
                model_id: range.model_id.clone(),
                language: "en".into(),
            },
            text: "public".into(),
            json,
        };
        let plain = english_result(reading(None), &range).unwrap();
        assert_eq!(plain["start_sample"], 16000);
        assert_eq!(plain["end_sample"], 48000);
        assert_eq!(plain["timing"], "range");
        let timed = english_result(
            reading(Some(
                json!({"segments":[{"start":0.2,"end":1.8,"text":"public"}]}),
            )),
            &range,
        )
        .unwrap();
        assert_eq!(timed["segments"][0]["end"], 1.8);
    }
}
