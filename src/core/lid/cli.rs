use super::*;
use crate::core::{asr::Result, store_io};
use std::{ffi::OsString, path::Path};

#[derive(Default, Deserialize)]
#[serde(default)]
struct OptIn {
    labs: crate::core::meeting_watch::Labs,
    pipeline: crate::core::meeting_router::RoutingConfig,
}
fn json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = store_io::read_bounded(path, 16 * 1024 * 1024)?;
    Ok(serde_json::from_slice(
        bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes),
    )?)
}
pub fn run_cli(args: &[OsString]) -> i32 {
    let run = || -> Result<()> {
        let value = |flag: &str| -> Result<PathBuf> {
            args.windows(2)
                .find(|p| p[0] == flag)
                .map(|p| PathBuf::from(&p[1]))
                .ok_or_else(|| format!("{flag} requires an explicit path").into())
        };
        let request_path = value("--language-detect")?;
        let config_path = value("--lid-config")?;
        let output = value("--lid-output")?;
        if !output.is_absolute() {
            return Err("--lid-output needs an absolute directory".into());
        }
        let request: Request = json(&request_path)?;
        let config: OptIn = json(&config_path)?;
        std::fs::create_dir_all(&output)?;
        let output = std::fs::canonicalize(output)?;
        for job in &request.jobs {
            let target = output.join(&job.id).join("language-detect.v1.json");
            for input in [
                &request_path,
                &config_path,
                &job.wav,
                &config.labs.language_detection_settings.model,
                &config.labs.language_detection_settings.runtime_dll,
            ] {
                if target.exists()
                    && std::fs::canonicalize(&target)?
                        == std::fs::canonicalize(input).unwrap_or_default()
                {
                    return Err("Sidecar would overwrite an input file".into());
                }
            }
        }
        stage::start_with_benchmark(
            request,
            config.labs.language_detection_settings,
            config.labs.language_detection,
            config.pipeline.language_mode,
            output,
            args.iter().any(|a| a == "--lid-benchmark-handshake"),
        )?
        .join()
        .map_err(|_| "Language worker panicked")?
    };
    match run() {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("Language detection failed: {error}");
            1
        }
    }
}
