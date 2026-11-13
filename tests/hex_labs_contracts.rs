mod common;
use sidenote::core::{
    Manifest,
    live::parse_rolling,
    meeting_router::{Audio, RoutingConfig, input_revision},
};
use std::{fs, path::PathBuf};
struct Temp(PathBuf);
#[test]
fn default_paths_are_isolated_from_owner_appdata() {
    common::isolate();
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn chapters_and_future_manifest_fields_preserve_existing_metadata() {
    common::isolate();
    let manifest: Manifest =
        serde_json::from_str(include_str!("fixtures/hex-labs/manifest.json")).unwrap();
    assert_eq!(manifest.status.as_deref(), Some("complete"));
    assert_eq!(manifest.duration_ms, Some(5000.0));
    assert_eq!(
        manifest.title.as_deref(),
        Some("Synthetic compatibility meeting")
    );
}
#[test]
fn partial_rolling_line_preserves_all_complete_lines() {
    common::isolate();
    let lines = parse_rolling(include_str!("fixtures/hex-labs/rolling.ndjson"));
    assert_eq!(lines.len(), 2);
    assert!(lines[0].you);
    assert_eq!(lines[0].text, "Synthetic first line");
    assert_eq!(lines[1].text, "Synthetic second line");
}
#[test]
fn missing_and_expired_tracks_return_a_clear_error_and_keep_transcript() {
    common::isolate();
    let folder = std::env::temp_dir().join(format!(
        "hex-labs-contract-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    ));
    fs::create_dir_all(&folder).unwrap();
    let temp = Temp(folder);
    let text = include_str!("fixtures/hex-labs/original.ndjson");
    fs::write(temp.0.join("transcript.ndjson"), text).unwrap();
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/hex-labs/audio-tracks.json")).unwrap();
    let config = RoutingConfig::default();
    let paths = vec![temp.0.join("system.wav"), temp.0.join("microphone.wav")];
    let absent = input_revision(&config, &paths, &[], &serde_json::Value::Null).unwrap();
    for case in ["missing", "expired"] {
        let names = fixture[case].as_array().unwrap();
        let mut tracks = Vec::new();
        for name in names {
            let path = temp.0.join(name.as_str().unwrap());
            if case == "expired" {
                let spec = hound::WavSpec {
                    channels: 1,
                    sample_rate: 16000,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                };
                let mut writer = hound::WavWriter::create(&path, spec).unwrap();
                writer.write_sample(0_i16).unwrap();
                writer.finalize().unwrap();
                assert!(Audio::open(&[("system".into(), path.clone())]).is_ok());
                let present =
                    input_revision(&config, &paths, &[], &serde_json::Value::Null).unwrap();
                assert_ne!(absent, present);
                fs::remove_file(&path).unwrap();
            }
            if path.exists() {
                tracks.push(("system".into(), path));
            }
        }
        let error = Audio::open(&tracks)
            .err()
            .expect("missing tracks must fail");
        assert_eq!(error.to_string(), fixture["message"].as_str().unwrap());
        assert_eq!(
            fs::read_to_string(temp.0.join("transcript.ndjson")).unwrap(),
            text
        );
        assert_eq!(
            absent,
            input_revision(&config, &paths, &[], &serde_json::Value::Null).unwrap()
        );
    }
}
