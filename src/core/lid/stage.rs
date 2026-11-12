use super::*;
use crate::core::{
    asr::Result,
    meeting_router::{Audio, Chunk},
    store_io,
};
use std::{collections::BTreeSet, fs, path::Path, thread::JoinHandle};

pub fn start(
    request: Request,
    settings: Settings,
    enabled: bool,
    mode: String,
    output: PathBuf,
) -> std::io::Result<JoinHandle<Result<()>>> {
    start_with_benchmark(request, settings, enabled, mode, output, false)
}
pub(super) fn start_with_benchmark(
    request: Request,
    settings: Settings,
    enabled: bool,
    mode: String,
    output: PathBuf,
    benchmark: bool,
) -> std::io::Result<JoinHandle<Result<()>>> {
    worker::spawn(
        settings.clone(),
        output.join("local/cache/language_detect"),
        benchmark,
        move |worker| run(request, settings, enabled, &mode, &output, worker),
    )
}

fn validate(request: &Request, settings: &Settings, mode: &str) -> Result<()> {
    settings.validate()?;
    let languages = &request.meeting_languages;
    if languages.is_empty()
        || languages.len() > 23
        || languages.iter().any(|s| s.is_empty())
        || languages.iter().collect::<BTreeSet<_>>().len() != languages.len()
    {
        return Err("Declare distinct meeting languages".into());
    }
    if mode != "auto" && mode != "off" {
        let route = mode
            .strip_prefix("forced:")
            .ok_or("Choose auto, off or forced:<route>")?;
        let valid = if route == "mixed" {
            languages.iter().any(|s| s == "en") && languages.iter().any(|s| s == "hi")
        } else {
            !route.is_empty() && route.split('+').all(|s| languages.iter().any(|l| l == s))
        };
        if !valid {
            return Err("Forced route must use declared meeting languages".into());
        }
    }
    if request.jobs.is_empty()
        || request.jobs.len() > 1000
        || request.jobs.iter().map(|j| j.chunks.len()).sum::<usize>() > 10000
    {
        return Err("Provide 1 to 1000 jobs and at most 10000 chunks".into());
    }
    let mut ids = BTreeSet::new();
    for job in &request.jobs {
        if job.id.is_empty()
            || job.id.len() > 80
            || !job
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            || !ids.insert(&job.id)
            || !job.wav.is_absolute()
            || job
                .wav
                .extension()
                .is_none_or(|s| !s.eq_ignore_ascii_case("wav"))
        {
            return Err("Jobs need unique alphanumeric/hyphen IDs and absolute WAV paths".into());
        }
        let audio = Audio::open(&[("file".into(), job.wav.clone())])?;
        let mut chunk_ids = BTreeSet::new();
        let mut previous = 0.0;
        for chunk in &job.chunks {
            let start = chunk.audio_start.unwrap_or(chunk.start);
            let duration = chunk.end - chunk.start;
            if !chunk.start.is_finite()
                || !chunk.end.is_finite()
                || !start.is_finite()
                || start < 0.0
                || chunk.start < previous
                || chunk.start < 0.0
                || !(0.1..=24.0).contains(&duration)
                || start + duration > audio.duration()
                || chunk.id.is_empty()
                || !chunk_ids.insert(&chunk.id)
            {
                return Err(
                    "Chunks need unique IDs, ordered times and 0.1 to 24 seconds within the WAV"
                        .into(),
                );
            }
            previous = chunk.start;
        }
    }
    Ok(())
}

fn run(
    request: Request,
    settings: Settings,
    enabled: bool,
    mode: &str,
    output: &Path,
    worker: &mut Worker,
) -> Result<()> {
    validate(&request, &settings, mode)?;
    if !output.is_absolute() {
        return Err("Language sidecars need an absolute output directory".into());
    }
    for job in &request.jobs {
        if let Ok(target) = fs::canonicalize(output.join(&job.id).join("language-detect.v1.json")) {
            for input in request
                .jobs
                .iter()
                .map(|j| &j.wav)
                .chain([&settings.model, &settings.runtime_dll])
            {
                if fs::canonicalize(input).is_ok_and(|input| input == target) {
                    return Err("Language sidecar would overwrite an input file".into());
                }
            }
        }
    }
    let languages = &request.meeting_languages;
    let bypass = if !enabled {
        Some("disabled")
    } else if mode == "off" {
        Some("off")
    } else if mode.starts_with("forced:") {
        Some("forced")
    } else if languages.len() == 1 {
        Some("single_language")
    } else if languages.iter().any(|s| !LANGUAGES.contains(&s.as_str())) {
        Some("unsupported_language")
    } else {
        None
    };
    for job in request.jobs {
        let mut audio = Audio::open(&[("file".into(), job.wav.clone())])?;
        let mut guesses = Vec::new();
        let mut previous: Option<(&Chunk, Guess)> = None;
        let mut warnings = BTreeSet::from(["Experimental guesses; thresholds uncalibrated; energy is not speech/music classification; no transcription changes".to_owned()]);
        for chunk in &job.chunks {
            let mut guess = Guess::fallback(&chunk.id, &languages[0], "unchecked");
            let mut pcm = None;
            let mut inherited_from = None;
            if let Some(reason) = bypass {
                guess.status = reason.into();
                if let Some(route) = mode.strip_prefix("forced:").filter(|_| enabled) {
                    let language = if route == "mixed" {
                        "hi"
                    } else {
                        route.split('+').next().unwrap_or(&languages[0])
                    };
                    guess.language = language.into();
                    guess.forced_route = Some(route.into());
                }
            } else {
                let samples = audio.read(chunk)?;
                pcm = Some(pcm_sha256(&samples));
                let energy = samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32;
                if energy <= settings.silence_mean_square {
                    guess.status = "silence_unchecked".into();
                } else if chunk.end - chunk.start < 2.0 {
                    guess = inherit(chunk, previous.as_ref(), &languages[0]);
                    if guess.status == "inherited_previous_same_turn" {
                        inherited_from = Some(guess.chunk.clone());
                    }
                } else {
                    match worker.identify(&samples, languages) {
                        Ok(value) => guess = value,
                        Err(error) => {
                            guess.status = "detection_failed_unchecked".into();
                            warnings.insert(error.to_string());
                        }
                    }
                }
            }
            let row = ChunkResult::for_chunk(chunk, guess, pcm, inherited_from);
            previous = Some((chunk, row.guess.clone()));
            guesses.push(row);
        }
        store_io::write(&output.join(&job.id).join("language-detect.v1.json"), &Sidecar {
            job: job.id, source_wav: job.wav,
            schema: "language-detect.v1".into(), experimental: true, thresholds_calibrated: false,
            confidence_semantics: "mean sub-window softmax, normalized over declared languages; not calibrated correctness".into(),
            model_sha256: MODEL_SHA256.into(), settings: settings.clone(), language_mode: mode.into(),
            meeting_languages: languages.clone(), chunks: guesses, warnings: warnings.into_iter().collect(),
        })?;
    }
    Ok(())
}

pub(super) fn inherit(chunk: &Chunk, previous: Option<&(&Chunk, Guess)>, first: &str) -> Guess {
    if let Some((prior, guess)) = previous
        && chunk.track == prior.track
        && chunk.speaker.as_deref().is_some_and(|s| !s.is_empty())
        && chunk.speaker == prior.speaker
        && chunk.turn.as_deref().is_some_and(|s| !s.is_empty())
        && chunk.turn == prior.turn
        && chunk.start >= prior.end
        && chunk.start - prior.end <= 1.5
        && guess.confidence.is_some()
        && !guess.outside_meeting_languages
    {
        let mut guess = guess.clone();
        guess.status = "inherited_previous_same_turn".into();
        return guess;
    }
    Guess::fallback(&chunk.id, first, "short_without_same_turn_predecessor")
}
