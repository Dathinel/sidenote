//! Explicit file acquisition, never inference or an implicit startup download.
use crate::core::{
    self,
    model_catalog::{self, ModelStore, Runtime},
};
use std::{ffi::OsString, io, path::Path, sync::atomic::AtomicBool};

/// Runtime named in the status line. The pin is reported without loading anything; the version
/// comes from the library itself, only after `--runtime-library` made `initialize` accept it.
fn runtime_status(library: Option<&Path>) -> io::Result<String> {
    #[cfg(feature = "asr-runtime")]
    {
        const PINNED: &str = "onnxruntime-sherpa-v2.0.1-linux-x64-api22";
        let Some(library) = library else {
            return Ok(format!("{PINNED} not-loaded"));
        };
        core::asr::initialize(library).map_err(|_| io::Error::other("RUNTIME_REFUSED"))?;
        let version = core::asr::runtime_version().unwrap_or("unknown");
        Ok(format!("onnxruntime-{version}-linux-x64-api22 loaded"))
    }
    #[cfg(not(feature = "asr-runtime"))]
    {
        let _ = library;
        Ok("unavailable-built-without-asr-runtime".to_owned())
    }
}

fn execute(args: &[OsString]) -> io::Result<()> {
    if crate::service::role() == crate::service::DeviceRole::Hub {
        return Err(io::Error::other("CPU_MODELS_DISABLED_FOR_HUB"));
    }
    let position = args
        .iter()
        .position(|a| a == "--cpu-models")
        .ok_or(io::ErrorKind::InvalidInput)?;
    let operation = args
        .get(position + 1)
        .and_then(|a| a.to_str())
        .ok_or(io::ErrorKind::InvalidInput)?;
    let library = args
        .iter()
        .position(|a| a == "--runtime-library")
        .map(|i| {
            args.get(i + 1)
                .map(Path::new)
                .ok_or(io::ErrorKind::InvalidInput)
        })
        .transpose()?;
    if operation == "list" {
        let runtime = runtime_status(None)?;
        let rows: Vec<_> = model_catalog::catalog().iter().filter(|m| m.runtime == Runtime::SherpaOnnx).map(|m| {
            serde_json::json!({"id":m.id,"bytes":m.bytes(),"license":m.licence,"runtime":runtime})
        }).collect();
        println!("{}", serde_json::to_string(&rows)?);
        return Ok(());
    }
    if !matches!(operation, "fetch" | "verify") {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let id = args
        .get(position + 2)
        .and_then(|a| a.to_str())
        .ok_or(io::ErrorKind::InvalidInput)?;
    if model_catalog::model(id)?.runtime != Runtime::SherpaOnnx {
        return Err(io::ErrorKind::Unsupported.into());
    }
    // umask applies to partial files too. Private ancestor validation fails closed.
    unsafe {
        libc::umask(0o077);
    }
    sidenote_sync::validate_private_directory(&core::state_dir())
        .map_err(|_| io::Error::other("MODEL_STORAGE_UNSAFE"))?;
    let store = ModelStore::default();
    if operation == "fetch" {
        store.install(id, &AtomicBool::new(false), |_| {})?;
    }
    if !store.verify_installed(id)? {
        return Err(io::Error::other("MODEL_NOT_VERIFIED"));
    }
    println!(
        "CPU_MODEL_FILES_VERIFIED runtime={}",
        runtime_status(library)?
    );
    Ok(())
}
pub fn cli(args: &[OsString]) -> i32 {
    match execute(args) {
        Ok(()) => 0,
        Err(_) => {
            eprintln!("CPU_MODEL_OPERATION_FAILED");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_operations_models_and_hex_models_fail_before_network() {
        for tail in [
            vec!["remove"],
            vec!["fetch", "unknown"],
            vec!["verify", "unknown"],
            vec!["fetch"],
            vec![],
        ] {
            let args: Vec<_> = std::iter::once("--cpu-models")
                .chain(tail)
                .map(OsString::from)
                .collect();
            assert!(execute(&args).is_err());
        }
    }
}
