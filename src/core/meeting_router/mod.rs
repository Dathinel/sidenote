//! Experimental meeting-only routing. The compatibility profile never opens a local model.
mod audio;
mod baseline;
mod cache;
mod cli;
mod config;
mod hex;
pub(crate) mod inputs;
pub mod jobs;
pub mod pieces;
pub mod vad;
use super::{
    asr::{self, LanguageScores, Result},
    store_io,
};
pub use audio::Audio;
pub use baseline::{english_fallback, reconcile};
pub use cli::run_cli;
pub use config::{LanguageRoutes, ProviderRoute, RoutingConfig};
pub use hex::HexSpeech;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

/// Selected on the public pilot, not calibrated on held-out meetings.
pub const HINDI_THRESHOLD: f32 = 0.9;
const MAX_SAMPLES: usize = 24 * asr::SAMPLE_RATE as usize;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Chunk {
    pub id: String,
    pub start: f64,
    pub end: f64,
    #[serde(default)]
    pub track: String,
    #[serde(default)]
    pub speaker: Option<String>,
    #[serde(default)]
    pub turn: Option<String>,
    #[serde(default)]
    pub english: String,
    #[serde(default)]
    pub audio_start: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Offset {
    pub chunk: Chunk,
    pub joined_start: f64,
    pub joined_end: f64,
    pub scores: Option<LanguageScores>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Segment {
    #[serde(default)]
    pub start: f64,
    #[serde(default)]
    pub end: f64,
    #[serde(default)]
    pub track: String,
    pub route: String,
    pub lang: String,
    pub lang_conf: Option<f32>,
    pub text_original: String,
    pub text_en: Option<String>,
    pub english_reading: String,
    pub provider: String,
    pub model: String,
    pub status: String,
    pub sources: Vec<Offset>,
    pub translate: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transcript {
    #[serde(default)]
    pub baseline: Vec<Chunk>,
    #[serde(default)]
    pub mixed_calls: usize,
    #[serde(default)]
    pub mixed_seconds: f64,
    pub schema: String,
    pub experimental: bool,
    pub input_checksum: String,
    #[serde(default)]
    pub config_checksum: String,
    pub segments: Vec<Segment>,
    pub english_reading: String,
    pub wall_seconds: f64,
    pub lid_load_seconds: Option<f64>,
    pub hi_load_seconds: Option<f64>,
    pub warnings: Vec<String>,
}

pub(crate) fn english(chunk: &Chunk, scores: Option<LanguageScores>, status: &str) -> Segment {
    Segment {
        start: chunk.start,
        end: chunk.end,
        track: chunk.track.clone(),
        route: "en".into(),
        lang: "en".into(),
        lang_conf: scores.as_ref().map(|s| s.english),
        text_original: chunk.english.clone(),
        text_en: Some(chunk.english.clone()),
        english_reading: chunk.english.clone(),
        provider: "hex".into(),
        model: "parakeet_unified_en".into(),
        status: status.into(),
        sources: vec![Offset {
            chunk: chunk.clone(),
            joined_start: 0.0,
            joined_end: chunk.end - chunk.start,
            scores,
        }],
        translate: false,
    }
}

#[cfg(feature = "asr-runtime")]
fn pooled_audio(audio: &mut Audio, chunks: &[Chunk], index: usize) -> Result<Vec<f32>> {
    let chunk = &chunks[index];
    if chunk.speaker.is_none() || chunk.turn.is_none() {
        return audio.read(chunk);
    }
    let same =
        |a: &Chunk, b: &Chunk| a.speaker == b.speaker && a.turn == b.turn && a.track == b.track;
    let mut start = index;
    while start > 0 && same(&chunks[start - 1], chunk) {
        start -= 1;
    }
    loop {
        let mut end = start;
        let mut pooled = Vec::new();
        while end < chunks.len() && same(&chunks[end], chunk) {
            let part = audio.read(&chunks[end])?;
            if pooled.len() + part.len() > MAX_SAMPLES {
                break;
            }
            pooled.extend_from_slice(&part);
            end += 1;
        }
        if end > index {
            return Ok(pooled);
        }
        if end == start {
            return Err("Could not pool speech window".into());
        }
        start = end;
    }
}

pub struct Request<'a> {
    pub audio: &'a mut Audio,
    pub baseline: Vec<Chunk>,
    pub chunks: Vec<Chunk>,
    pub config: &'a RoutingConfig,
    pub hex: &'a HexSpeech,
    pub work: &'a Path,
    pub cache: &'a Path,
}

struct RouteAudio(Vec<PathBuf>);
impl Drop for RouteAudio {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = fs::remove_file(path);
        }
    }
}

pub fn route(request: Request<'_>) -> Result<Transcript> {
    let Request {
        audio,
        baseline,
        chunks,
        config,
        hex,
        work,
        cache: cache_root,
    } = request;
    let config_checksum = configuration_key(config)?;
    let mut config = config.clone();
    if config.model_state_dir.as_os_str().is_empty() {
        config.model_state_dir = super::state_dir();
    }
    if config.runtime_dll.as_os_str().is_empty() {
        config.runtime_dll =
            std::env::current_exe()?.with_file_name(crate::platform::asr_library_name());
    }
    config.validate()?;
    fs::create_dir_all(work)?;
    let mut route_audio = RouteAudio(Vec::new());
    let tick = Instant::now();
    let mut transcript = Transcript {
        schema: "transcript.segments.v1".into(),
        experimental: config.enabled(),
        input_checksum: input_revision(
            &config,
            &audio.paths(),
            &baseline,
            &serde_json::to_value(&chunks)?,
        )?,
        config_checksum,
        baseline: baseline.clone(),
        mixed_calls: 0,
        mixed_seconds: 0.0,
        segments: Vec::new(),
        english_reading: chunks
            .iter()
            .map(|c| c.english.as_str())
            .collect::<Vec<_>>()
            .join(" "),
        wall_seconds: 0.0,
        lid_load_seconds: None,
        hi_load_seconds: None,
        warnings: Vec::new(),
    };
    let mut ids = std::collections::HashSet::new();
    for chunk in &chunks {
        audio.read(chunk)?;
        if !ids.insert(&chunk.id) {
            return Err("Duplicate speech window id".into());
        }
    }
    let enabled = config.enabled();
    let supported = config
        .meeting_languages
        .iter()
        .all(|s| matches!(s.as_str(), "en" | "hi" | "mixed"));
    let detect = enabled
        && supported
        && config.language_mode == "auto"
        && config.meeting_languages.len() > 1
        && config.meeting_languages.iter().any(|l| l != "en");
    #[cfg(feature = "asr-runtime")]
    let lid_checksum = if detect {
        cache::installed_identity(&config.model_state_dir, "anuvani-lid-int8")
    } else {
        Err("Detector not required".into())
    };
    #[cfg(feature = "asr-runtime")]
    let mut lid = if detect {
        let result = (|| {
            lid_checksum
                .as_ref()
                .map_err(|_| "Language model unavailable")?;
            asr::initialize(&config.runtime_dll)?;
            asr::Anuvani::load(&asr::model_path(
                &config.model_state_dir,
                "anuvani-lid-int8",
            )?)
        })();
        match result {
            Ok(model) => {
                transcript.lid_load_seconds = Some(model.load_seconds);
                Some(model)
            }
            Err(_) => {
                transcript
                    .warnings
                    .push("Language detector unavailable; English retained".into());
                None
            }
        }
    } else {
        None
    };
    #[cfg(not(feature = "asr-runtime"))]
    if detect {
        transcript
            .warnings
            .push("Local runtime was not included in this build".into());
    }
    let mut decisions = Vec::new();
    for index in 0..chunks.len() {
        #[cfg(not(feature = "asr-runtime"))]
        let _ = index;
        #[cfg(feature = "asr-runtime")]
        let mut scores = None;
        #[cfg(not(feature = "asr-runtime"))]
        let scores = None;
        let mut status = "checked";
        let route = if !enabled {
            status = "compat";
            "en"
        } else if let Some(forced) = config.language_mode.strip_prefix("forced:") {
            status = "forced";
            forced
        } else if !supported {
            status = "language not checked: unsupported declared language";
            "en"
        } else if config.language_mode == "off" {
            status = "declared";
            config.meeting_languages[0].as_str()
        } else if !detect {
            config.meeting_languages[0].as_str()
        } else {
            #[cfg(feature = "asr-runtime")]
            {
                use asr::LanguageId;
                if let Some(lid) = &mut lid {
                    let pooled = pooled_audio(audio, &chunks, index)?;
                    let waveform = pooled.as_slice();
                    let key = cache::key(
                        waveform,
                        "runtime:anuvani-lid-int8",
                        lid_checksum.as_deref().unwrap_or("unavailable"),
                        "en,hi",
                    );
                    let result = match cache::get(cache_root, "language_detect", &key) {
                        Some(value) => Ok(value),
                        None => {
                            lid.identify(waveform, &["en".into(), "hi".into()])
                                .inspect(|value| {
                                    let _ = cache::put(cache_root, "language_detect", &key, value);
                                })
                        }
                    };
                    match result {
                        Ok(value) => {
                            let hi = value.hindi > HINDI_THRESHOLD;
                            scores = Some(value);
                            if hi { config.non_english_route() } else { "en" }
                        }
                        Err(_) => {
                            status = "language not checked";
                            "en"
                        }
                    }
                } else {
                    status = "language not checked";
                    "en"
                }
            }
            #[cfg(not(feature = "asr-runtime"))]
            {
                status = "language not checked";
                "en"
            }
        };
        decisions.push((route.to_owned(), scores, status.to_owned()));
    }
    // Only a known contiguous speaker turn can lend a short chunk its decision.
    for i in 0..chunks.len() {
        if detect && chunks[i].end - chunks[i].start < 2.0 {
            let same = |j: usize| {
                chunks[j].speaker.is_some()
                    && chunks[j].turn.is_some()
                    && chunks[j].speaker == chunks[i].speaker
                    && chunks[j].turn == chunks[i].turn
                    && chunks[j].track == chunks[i].track
            };
            let mut left = i;
            let mut right = i + 1;
            while left > 0 && same(left - 1) {
                left -= 1;
            }
            while right < chunks.len() && same(right) {
                right += 1;
            }
            let neighbour = (left..right)
                .filter(|&j| {
                    j != i
                        && chunks[j].end - chunks[j].start >= 2.0
                        && chunks[j].speaker == chunks[i].speaker
                        && chunks[j].turn == chunks[i].turn
                        && chunks[j].track == chunks[i].track
                        && decisions[j].1.is_some()
                })
                .min_by_key(|&j| i.abs_diff(j));
            if let Some(j) = neighbour {
                decisions[i] = decisions[j].clone();
                decisions[i].2 = "same speaker turn".into();
            } else {
                decisions[i].0 = "en".into();
                decisions[i].2 = "short speech; language not checked".into();
            }
        }
    }
    for (chunk, (route, scores, status)) in chunks.iter().zip(&decisions) {
        if route == "en" {
            transcript
                .segments
                .push(english(chunk, scores.clone(), status));
        }
    }

    #[cfg(feature = "asr-runtime")]
    let mut hindi_engine = None;
    #[cfg(feature = "asr-runtime")]
    let mut hindi_checksum = String::new();
    #[cfg(feature = "asr-runtime")]
    let mut hindi_unavailable = false;
    for route_name in ["hi", "mixed"] {
        let selected: Vec<_> = chunks
            .iter()
            .enumerate()
            .filter(|(i, _)| decisions[*i].0 == route_name)
            .map(|(i, _)| i)
            .collect();
        let mut groups: Vec<Vec<usize>> = Vec::new();
        for i in selected {
            let joins = route_name == "hi"
                || groups.last().and_then(|g| g.last()).is_some_and(|&j| {
                    i == j + 1
                        && chunks[i].track == chunks[j].track
                        && chunks[i].start - chunks[j].end < 3.0
                });
            if joins && let Some(group) = groups.last_mut() {
                group.push(i);
            } else {
                groups.push(vec![i]);
            }
        }
        for (group_index, group) in groups.iter().enumerate() {
            let wav = work.join(format!("{route_name}-{group_index}.wav"));
            route_audio.0.push(wav.clone());
            let mut writer = hound::WavWriter::create(
                &wav,
                hound::WavSpec {
                    channels: 1,
                    sample_rate: 16000,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                },
            )?;
            let mut count = 0usize;
            let mut offsets = Vec::new();
            for &i in group {
                let waveform = audio.read(&chunks[i])?;
                let start = count as f64 / 16000.0;
                for sample in waveform {
                    writer.write_sample((sample * 32768.0) as i16)?;
                    count += 1;
                }
                offsets.push(Offset {
                    chunk: chunks[i].clone(),
                    joined_start: start,
                    joined_end: count as f64 / 16000.0,
                    scores: decisions[i].1.clone(),
                });
                for _ in 0..8000 {
                    writer.write_sample(0i16)?;
                    count += 1;
                }
            }
            writer.finalize()?;
            store_io::write(
                &work.join(format!("{route_name}-{group_index}-offsets.json")),
                &offsets,
            )?;
            if route_name == "mixed" {
                let key = cache::key(
                    &[],
                    "hex:nemotron_3_5_asr",
                    &cache::model_checksum(&wav)?,
                    "group-v2",
                );
                let result = match cache::get::<String>(cache_root, "transcribe", &key) {
                    Some(text) => Ok(text),
                    None => {
                        let tick = Instant::now();
                        transcript.mixed_calls += 1;
                        let result = hex.transcribe(&wav, "nemotron_3_5_asr");
                        transcript.mixed_seconds += tick.elapsed().as_secs_f64();
                        result.inspect(|text| {
                            let _ = cache::put(cache_root, "transcribe", &key, text);
                        })
                    }
                };
                match result {
                    Ok(text) if !text.trim().is_empty() => {
                        transcript.segments.push(Segment {
                            start: chunks[group[0]].start,
                            end: chunks[*group.last().unwrap()].end,
                            track: chunks[group[0]].track.clone(),
                            route: "mixed".into(),
                            lang: "hi+en".into(),
                            lang_conf: None,
                            text_original: text,
                            text_en: None,
                            english_reading: group
                                .iter()
                                .map(|&i| chunks[i].english.as_str())
                                .collect::<Vec<_>>()
                                .join(" "),
                            provider: "hex".into(),
                            model: "nemotron_3_5_asr".into(),
                            status: "checked".into(),
                            sources: offsets,
                            translate: true,
                        });
                        continue;
                    }
                    Err(error) if error.to_string().contains("FATAL:") => return Err(error),
                    _ => {}
                }
            }
            #[cfg(feature = "asr-runtime")]
            if hindi_engine.is_none() && !hindi_unavailable {
                let result = (|| {
                    asr::initialize(&config.runtime_dll)?;
                    let model = asr::model_path(&config.model_state_dir, "indicconformer-hi-ctc")?;
                    hindi_checksum = cache::installed_identity(
                        &config.model_state_dir,
                        "indicconformer-hi-ctc",
                    )?;
                    asr::IndicConformer::load(&model, &model.with_file_name("tokens.txt"))
                })();
                match result {
                    Ok(engine) => {
                        transcript.hi_load_seconds = Some(engine.load_seconds);
                        hindi_engine = Some(engine);
                    }
                    Err(_) => hindi_unavailable = true,
                }
            }
            for (&i, offset) in group.iter().zip(offsets) {
                #[cfg(feature = "asr-runtime")]
                let reading: Result<asr::Reading> = (|| {
                    use asr::SpeechEngine;
                    let engine = hindi_engine.as_mut().ok_or("Hindi model unavailable")?;
                    let waveform = audio.read(&chunks[i])?;
                    let key = cache::key(
                        &waveform,
                        "runtime:indicconformer-hi-ctc",
                        &hindi_checksum,
                        "hi",
                    );
                    if let Some(reading) = cache::get(cache_root, "transcribe", &key) {
                        return Ok(reading);
                    }
                    let reading = engine.transcribe(&waveform, "hi")?;
                    let _ = cache::put(cache_root, "transcribe", &key, &reading);
                    Ok(reading)
                })();
                #[cfg(not(feature = "asr-runtime"))]
                let reading: Result<asr::Reading> = Err("Local runtime unavailable".into());
                match reading {
                    Ok(reading) if !reading.text.trim().is_empty() => {
                        transcript.segments.push(Segment {
                            start: chunks[i].start,
                            end: chunks[i].end,
                            track: chunks[i].track.clone(),
                            route: "hi".into(),
                            lang: "hi".into(),
                            lang_conf: decisions[i].1.as_ref().map(|s| s.hindi),
                            text_original: reading.text,
                            text_en: None,
                            english_reading: chunks[i].english.clone(),
                            provider: "runtime".into(),
                            model: "indicconformer-hi-ctc".into(),
                            status: if route_name == "mixed" {
                                "mixed model unavailable; Hindi fallback".into()
                            } else {
                                decisions[i].2.clone()
                            },
                            sources: vec![offset],
                            translate: true,
                        })
                    }
                    _ => transcript.segments.push(english(
                        &chunks[i],
                        decisions[i].1.clone(),
                        "model unavailable or recognition failed; English retained",
                    )),
                }
            }
        }
    }
    reconcile(&mut transcript, &baseline);
    transcript.segments.sort_by(|a, b| {
        a.sources[0]
            .chunk
            .start
            .total_cmp(&b.sources[0].chunk.start)
    });
    transcript.wall_seconds = tick.elapsed().as_secs_f64();
    Ok(transcript)
}

pub fn write_transcript(path: &Path, transcript: &Transcript) -> Result<()> {
    store_io::write(path, transcript)?;
    Ok(())
}

pub fn configuration_key(config: &RoutingConfig) -> Result<String> {
    let mut config = config.clone();
    if config.model_state_dir.as_os_str().is_empty() {
        config.model_state_dir = super::state_dir();
    }
    if config.runtime_dll.as_os_str().is_empty() {
        config.runtime_dll =
            std::env::current_exe()?.with_file_name(crate::platform::asr_library_name());
    }
    let mut resources = Vec::new();
    if config.enabled()
        && (config.meeting_languages.iter().any(|l| l != "en")
            || matches!(config.language_mode.as_str(), "forced:hi" | "forced:mixed"))
    {
        for id in ["anuvani-lid-int8", "indicconformer-hi-ctc"] {
            let checksum = cache::installed_identity(&config.model_state_dir, id)
                .unwrap_or_else(|_| "unavailable".into());
            resources.push((id.to_owned(), checksum));
        }
        resources.push((
            "runtime".into(),
            cache::model_checksum(&config.runtime_dll).unwrap_or_else(|_| "unavailable".into()),
        ));
    }
    Ok(cache::key(
        &[],
        &serde_json::to_string(&config)?,
        &serde_json::to_string(&resources)?,
        &format!("routing-v2:{}", config.meeting_languages.join(",")),
    ))
}

/// Point-in-time job identity; opens still have to handle removal after this snapshot.
pub fn input_revision(
    config: &RoutingConfig,
    paths: &[PathBuf],
    baseline: &[Chunk],
    windows: &serde_json::Value,
) -> Result<String> {
    let files: Vec<_> = paths
        .iter()
        .map(|p| {
            let metadata = fs::metadata(p).ok();
            (
                p.to_string_lossy().into_owned(),
                metadata.as_ref().map(|m| m.len()),
                metadata
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos()),
            )
        })
        .collect();
    Ok(cache::key(
        &[],
        &configuration_key(config)?,
        &serde_json::to_string(&(files, baseline, windows))?,
        "input-v2",
    ))
}

impl Transcript {
    pub fn degraded(&self) -> bool {
        self.segments.iter().any(|s| {
            [
                "unavailable",
                "failed",
                "not checked",
                "fallback",
                "retained",
            ]
            .iter()
            .any(|word| s.status.contains(word))
        })
    }
    pub fn revision(&self) -> String {
        cache::key(
            &[],
            &serde_json::to_string(self).unwrap_or_default(),
            "transcript",
            "revision-v2",
        )
    }

    pub fn original_lines(&self) -> Vec<super::Line> {
        self.segments
            .iter()
            .map(|s| {
                let first = s.sources.first();
                let timed = s.sources.len() == 1;
                let seconds = if s.end > s.start {
                    Some(s.start as u64)
                } else {
                    first.filter(|_| timed).map(|o| o.chunk.start as u64)
                };
                let route = match s.route.as_str() {
                    "en" => "English",
                    "hi" => "Hindi",
                    "mixed" => "Mixed",
                    other => other,
                };
                let model = match s.model.as_str() {
                    "parakeet_unified_en" => "Parakeet",
                    "indicconformer-hi-ctc" => "IndicConformer",
                    "nemotron_3_5_asr" => "Nemotron",
                    other => other,
                };
                let marked = [
                    "not checked",
                    "unavailable",
                    "failed",
                    "fallback",
                    "retained",
                ]
                .iter()
                .any(|word| s.status.contains(word));
                let caption = if s.route == "en" && !marked {
                    String::new()
                } else if s.status == "checked" {
                    format!("{route} ({model})")
                } else {
                    format!("{route} ({model}) · {}", s.status)
                };
                super::Line {
                    at: seconds
                        .map(|t| format!("{:02}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60))
                        .unwrap_or_default(),
                    seconds,
                    who: first
                        .filter(|_| timed)
                        .and_then(|o| o.chunk.speaker.clone())
                        .unwrap_or_default(),
                    language: caption,
                    text: s.text_original.clone(),
                }
            })
            .collect()
    }
}
