use super::*;
use std::ffi::OsString;

pub fn run_cli(args: &[OsString]) -> i32 {
    match run(args) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("Meeting transcription failed: {error}");
            1
        }
    }
}
fn run(args: &[OsString]) -> Result<()> {
    let value = |flag: &str| -> Option<PathBuf> {
        args.windows(2)
            .find(|a| a[0] == flag)
            .map(|a| PathBuf::from(&a[1]))
    };
    let text =
        |flag: &str| -> Option<String> { value(flag).map(|p| p.to_string_lossy().into_owned()) };
    let input = value("--transcribe-file").ok_or("--transcribe-file requires a WAV path")?;
    let out = value("--out").ok_or("--out requires a JSON path")?;
    let mut config = RoutingConfig {
        profile: "experimental".into(),
        ..RoutingConfig::default()
    };
    if let Some(languages) = text("--languages") {
        config.meeting_languages = languages.split(',').map(|s| s.trim().to_owned()).collect();
    }
    if let Some(mode) = text("--language-mode") {
        config.language_mode = mode;
    }
    if let Some(path) = value("--model-state-dir") {
        config.model_state_dir = path;
    }
    if let Some(path) = value("--runtime-dll") {
        config.runtime_dll = path;
    }
    config.validate()?;
    let mut audio = Audio::open(&[("file".into(), input.clone())])?;
    let hex = HexSpeech::default();
    let mut chunks = if let Some(path) = value("--chunks") {
        serde_json::from_slice::<Vec<Chunk>>(&fs::read(path)?)?
    } else {
        audio.speech_chunks()?
    };
    let baseline = if value("--chunks").is_some() {
        chunks
            .iter()
            .map(|c| c.english.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        hex.transcribe(&input, "parakeet_unified_en")?
    };
    // A plain CLI reading has no timestamps. Keep it whole instead of guessing word boundaries.
    if value("--chunks").is_none() && chunks.len() == 1 {
        chunks[0].english = baseline.clone();
    }
    let baselines = if value("--chunks").is_some() {
        chunks.clone()
    } else {
        vec![Chunk {
            id: "english-file".into(),
            start: 0.0,
            end: audio.duration(),
            track: "file".into(),
            speaker: None,
            turn: None,
            english: baseline.clone(),
            audio_start: None,
            words: Vec::new(),
        }]
    };
    let work = out
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!("local/router-{}", std::process::id()));
    let cache = out.parent().unwrap_or(Path::new(".")).join("local/cache");
    let mut result = route(Request {
        audio: &mut audio,
        baseline: baselines,
        chunks,
        config: &config,
        hex: &hex,
        work: &work,
        cache: &cache,
    })?;
    if value("--chunks").is_none() && result.segments.len() != 1 {
        result.warnings.push("Plain Hex output has no timestamps; the complete English reading is retained separately".into());
    }
    result.english_reading = baseline;
    write_transcript(&out, &result)?;
    for name in ["hi.wav", "mixed.wav"] {
        let _ = fs::remove_file(work.join(name));
    }
    Ok(())
}
