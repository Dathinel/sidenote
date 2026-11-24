#![cfg(all(windows, feature = "asr-runtime"))]

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sidenote::core::{asr, meeting_router::revision, notes::NotesSettings, recording_guard};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
};

fn hex_status_fixture(root: &Path, meeting_state: &str) -> PathBuf {
    let hex = std::path::absolute(root.join("hex.cmd")).unwrap();
    let status = json!({
        "version": 4,
        "model": {"preparing": null},
        "capture": {"state": "idle", "pending_jobs": 0},
        "meeting": {"state": meeting_state},
    });
    fs::write(
        &hex,
        format!(
            "@echo off\r\n\
             if not \"%1 %2\"==\"status --json\" exit /b 2\r\n\
             if not \"%3\"==\"\" exit /b 2\r\n\
             echo {status}\r\n\
             exit /b 0\r\n"
        ),
    )
    .unwrap();
    hex
}

fn route_pending(root: &Path, id: &str, hex: &Path) -> ExitStatus {
    let state = root.join("state");
    Command::new(env!("CARGO_BIN_EXE_sidenote"))
        .args(["--route-pending", "--automatic", "--meeting-id", id])
        .arg("--data-dir")
        .arg(&state)
        .arg("--meetings-dir")
        .arg(root.join("meetings"))
        .arg("--peers-dir")
        .arg(root.join("peers"))
        .arg("--hex-cli")
        .arg(hex)
        .env("SIDENOTE_DATA_DIR", &state)
        .status()
        .unwrap()
}

#[test]
fn route_pending_cli_uses_the_supplied_recording_status() {
    for (meeting_state, expected_error) in [
        ("idle", "cpu_language_assets_unavailable"),
        ("recording", "paused: recording"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        let id = "100-1";
        let folder = root.path().join("meetings").join(id);
        fs::create_dir(&state).unwrap();
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            state.join("config.json"),
            json!({
                "version": 1,
                "features": {"indic_engine": true},
                "labs": {"after_stop_language_routing": true},
                "pipeline": {"runtime_dll": root.path().join("missing/onnxruntime.dll")},
            })
            .to_string(),
        )
        .unwrap();
        fs::write(
            folder.join("manifest.json"),
            json!({
                "status": "complete", "duration_ms": 30_000,
                "ended_at_ms": chrono::Utc::now().timestamp_millis(),
                "system": {"first_arrival_us": 0},
            })
            .to_string(),
        )
        .unwrap();
        fs::write(
            folder.join("transcript.ndjson"),
            json!({
                "source": "system", "start_ms": 0, "end_ms": 30_000,
                "text": "synthetic ".repeat(25),
            })
            .to_string(),
        )
        .unwrap();
        asr::write_wav(&folder.join("system.wav"), &vec![0.0; 30 * 16_000]).unwrap();

        let hex = hex_status_fixture(root.path(), meeting_state);
        let status = route_pending(root.path(), id, &hex);
        assert!(status.success(), "{meeting_state}: routing exited {status}");
        let work = state.join("local/router-work").join(id);
        let attempt: Value =
            serde_json::from_slice(&fs::read(work.join("attempt.json")).unwrap()).unwrap();
        assert_eq!(attempt["child"]["error"], expected_error, "{meeting_state}");
        let pending = state.join("local/router-queue").join(format!("{id}.json"));
        assert!(pending.is_file(), "{meeting_state}: request removed");
        assert!(!work.join("released.json").exists());
        if meeting_state == "recording" {
            assert_eq!(attempt["pending_retained"], true);
        }
    }
}

#[test]
#[ignore = "run once per Windows release build with the isolated public CPU ASR bundle"]
fn default_cpu_passes_route_public_hindi_and_telugu() {
    let bundle = PathBuf::from(
        std::env::var_os("SIDENOTE_PUBLIC_CPU_ASR_BUNDLE")
            .expect("point SIDENOTE_PUBLIC_CPU_ASR_BUNDLE at the public CPU test bundle"),
    );
    assert!(bundle.is_absolute());
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    let meetings = root.path().join("meetings");
    let hex = hex_status_fixture(root.path(), "idle");
    fs::create_dir(&state).unwrap();
    fs::write(
        state.join("config.json"),
        json!({
            "version": 1, "features": {"indic_engine": true}, "notes": NotesSettings::default(),
            "pipeline": {
                "model_state_dir": bundle,
                "runtime_dll": bundle.join("onnxruntime.dll"),
            },
        })
        .to_string(),
    )
    .unwrap();

    for (id, language, sha) in [
        (
            "100-1",
            "hi",
            "7b32fd2d4ec90aecb0a26a68b68511a49801028bf0430339c9cb64585947ae90",
        ),
        (
            "100-2",
            "te",
            "75b140c10a6acc4867b676461e94b8bdd01dc5f4e70e2e29a4b1dbc223ad906b",
        ),
    ] {
        let folder = meetings.join(id);
        fs::create_dir_all(&folder).unwrap();
        let audio = fs::read(bundle.join(format!("{language}.wav"))).unwrap();
        assert_eq!(
            hex::encode(Sha256::digest(&audio)),
            sha,
            "{language} public audio"
        );
        let wav = folder.join("system.wav");
        fs::write(&wav, audio).unwrap();
        let samples = hound::WavReader::open(wav).unwrap().duration();
        fs::write(
            folder.join("transcript.ndjson"),
            json!({
                "source": "microphone", "start_ms": 0, "end_ms": 1000,
                "text": "Synthetic captured baseline for publication.",
            })
            .to_string(),
        )
        .unwrap();
        fs::write(
            folder.join("manifest.json"),
            json!({
                "status": "complete", "duration_ms": samples / 16,
                "ended_at_ms": chrono::Utc::now().timestamp_millis(),
                "system": {"first_arrival_us": 0},
            })
            .to_string(),
        )
        .unwrap();
        recording_guard::keep_in(&state, id).unwrap();

        let status = route_pending(root.path(), id, &hex);
        assert!(
            status.success(),
            "{language} routing process exited {status}"
        );
        let work = state.join("local/router-work").join(id);
        let released: Value = serde_json::from_slice(
            &fs::read(work.join("released.json")).unwrap_or_else(|error| {
                let attempt = fs::read_to_string(work.join("attempt.json")).unwrap_or_default();
                panic!("{language} default routing did not release a transcript: {error} {attempt}")
            }),
        )
        .unwrap();
        let published = state.join("local/transcripts").join(id);
        let (_, revision) = revision::load(&published, released["revision"].as_str())
            .unwrap()
            .unwrap();
        let segments: Vec<_> = revision
            .transcript
            .segments
            .iter()
            .filter(|segment| segment.track == "system")
            .collect();
        assert!(
            !segments.is_empty()
                && segments.iter().all(|segment| segment.lang == language
                    && segment.provider == "local"
                    && !segment.text_original.trim().is_empty()),
            "{language} needs non-empty CPU output in the requested language"
        );
        println!(
            "{language}: {} samples, {} routed segments",
            samples,
            segments.len()
        );
    }
}
