use super::*;

#[test]
#[ignore = "requires this lane's isolated public CLI proof"]
fn public_revision_reaches_notes_without_audio() {
    let root = PathBuf::from(std::env::var_os("SIDENOTE_ROUTING_PUBLIC_PROOF").unwrap());
    let name = root.file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with("sidenote-routing-8c-"));
    assert!(crate::core::is_proof_path(&root, name));
    let id = "1800000000000-88";
    let state = root.join("state");
    let dir = state.join("local/transcripts").join(id);
    let (revision_id, revision) = revision::load(&dir, None).unwrap().unwrap();
    let audio = root.join("hex/meetings").join(id);
    let hidden = root.join("audio-unavailable");
    fs::rename(&audio, &hidden).unwrap();
    struct Restore(PathBuf, PathBuf);
    impl Drop for Restore {
        fn drop(&mut self) {
            fs::rename(&self.0, &self.1).unwrap();
        }
    }
    let _restore = Restore(hidden, audio);
    let started = Instant::now();
    let now = chrono::Utc::now().timestamp_millis();
    for chosen in [None, Some(revision_id.as_str())] {
        let (chosen_id, transcript) =
            jobs::notes_input(&state, id, chosen, &revision.transcript.baseline, now, now)
                .unwrap()
                .unwrap();
        assert_eq!(chosen_id, revision_id);
        assert_eq!(
            serde_json::to_vec(&transcript).unwrap(),
            serde_json::to_vec(&revision.transcript).unwrap()
        );
        for language in ["en", "hi", "te"] {
            assert!(transcript.segments.iter().any(|s| s.lang == language));
        }
    }
    store_io::write(
        &root.join("notes-reader.json"),
        &json!({"revision":revision_id,
        "seconds":started.elapsed().as_secs_f64(),"audio_absent":true,"labels":["en","hi","te"],
        "helper_generation":"not run"}),
    )
    .unwrap();
}
