//! Public timing only, using the production Native loader, recognizer and cleanup.
#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use serde_json::json;
    use sidenote::core::{
        self, Config,
        meeting_router::{
            HexSpeech,
            batches::*,
            hex_bridge::{HexCommand, HexFailure},
            production,
        },
    };
    use std::{
        fs,
        io::{BufRead, Write},
        path::Path,
        time::Instant,
    };
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.iter().any(|a| a == "--routing-native") {
        return production::native_worker();
    }
    core::configure_paths(&args)?;
    let root = core::state_dir().canonicalize()?;
    assert!(root.starts_with(std::env::temp_dir().canonicalize()?));
    let value = |flag: &str| {
        args.windows(2)
            .find(|a| a[0] == flag)
            .map(|a| Path::new(&a[1]))
            .ok_or("Missing benchmark argument")
    };
    let config: Config = serde_json::from_slice(&fs::read(value("--config")?)?)?;
    let ranges: Vec<Range> = serde_json::from_slice(&fs::read(value("--ranges")?)?)?;
    assert_eq!(ranges.len(), 20);
    let language = match ranges[0].range.language.as_deref() {
        Some("en") => None,
        Some("hi") => Some(NativeLanguage::Hindi),
        Some("te") => Some(NativeLanguage::Telugu),
        _ => return Err("Expected Indic production ranges".into()),
    };
    let started = Instant::now();
    let mut session = language
        .map(|language| production::Native(&config).load(language, &ranges[0]))
        .transpose()?;
    let mut hex = HexSpeech {
        executable: value("--hex-cli")?.to_owned(),
    };
    println!(
        "{}",
        json!({"event":"loaded","seconds":started.elapsed().as_secs_f64()})
    );
    std::io::stdout().flush()?;
    for (range, line) in ranges.iter().zip(std::io::stdin().lock().lines()) {
        if line? != "range" {
            return Err("Invalid benchmark request".into());
        }
        let started = Instant::now();
        let empty = if let Some(session) = &mut session {
            let result = session.recognize(range)?;
            result["text"]
                .as_str()
                .ok_or("Missing native text")?
                .trim()
                .is_empty()
        } else {
            match hex.call(
                &[
                    "transcribe",
                    range.wav.to_str().ok_or("Invalid WAV path")?,
                    "--model",
                    &range.model_id,
                    "--language",
                    "en",
                    "--json",
                ],
                std::time::Duration::from_secs(180),
            ) {
                Ok(reply) => {
                    let result: serde_json::Value = serde_json::from_str(&reply)?;
                    let empty = result["text"]
                        .as_str()
                        .or_else(|| result["transcript"].as_str())
                        .ok_or("Missing CLI text")?
                        .trim()
                        .is_empty();
                    if !empty && !value("--capture")?.exists() {
                        fs::write(value("--capture")?, reply)?;
                    }
                    empty
                }
                Err(error) if error.kind() == &HexFailure::NoSpeech => true,
                Err(error) => return Err(error.into()),
            }
        };
        println!(
            "{}",
            json!({"event":"piece","seconds":started.elapsed().as_secs_f64(),
            "audio_seconds":(range.range.context.1-range.range.context.0) as f64/16000.0,"empty":empty})
        );
        std::io::stdout().flush()?;
    }
    let started = Instant::now();
    if let Some(session) = &mut session {
        session.finish()?;
    }
    println!(
        "{}",
        json!({"event":"finished","seconds":started.elapsed().as_secs_f64()})
    );
    Ok(())
}
#[cfg(not(windows))]
fn main() {
    panic!("This benchmark requires the Windows production Native loader");
}
