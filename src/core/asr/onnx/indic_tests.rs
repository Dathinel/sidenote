use super::*;

fn typed<T: std::fmt::Debug>(result: Result<T>, expected: IndicError) {
    assert_eq!(
        result.unwrap_err().downcast_ref::<IndicError>(),
        Some(&expected)
    );
}

#[test]
fn indic_rejects_untrusted_pack_and_input_without_runtime() {
    let temp = tempfile::tempdir().unwrap();
    let graph = temp.path().join("model.onnx");
    fs::write(&graph, b"wrong graph").unwrap();
    for language in ["hi", "te"] {
        let error = IndicConformer::load_for_language(&graph, &graph, language)
            .err()
            .unwrap();
        typed::<()>(Err(error), IndicError::ModelHash);
    }
    typed(indic_tokens(b"wrong tokens"), IndicError::Tokens);
    let valid =
        ["ai4bharat", "per_feature", "4", "5633", "EncDecCTCModelBPE"].map(|s| Some(s.to_owned()));
    indic_metadata(&valid).unwrap();
    for i in 0..valid.len() {
        for replacement in [None, Some("wrong".into())] {
            let mut wrong = valid.clone();
            wrong[i] = replacement;
            typed(indic_metadata(&wrong), IndicError::Metadata);
        }
    }
    typed(indic_hash(b"", "en"), IndicError::Language);
    for audio in [vec![0.0; 480_001], vec![f32::NAN; 1600], vec![]] {
        typed(indic_audio(&audio, "hi", "hi"), IndicError::Audio);
    }
    indic_audio(&vec![0.0; 480_000], "te", "te").unwrap();
    typed(indic_audio(&[0.0; 1600], "hi", "te"), IndicError::Language);
}

#[cfg(windows)]
fn parity(language: &str) {
    let root = PathBuf::from(std::env::var("INDIC_S45_ROOT").unwrap());
    let dll = PathBuf::from(std::env::var("INDIC_RUNTIME_DLL").unwrap());
    let temp = tempfile::tempdir().unwrap();
    let refs: serde_json::Value = serde_json::from_str(include_str!("../indic-s45.json")).unwrap();
    let reference = refs
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["language"] == language)
        .unwrap();
    let pack = root
        .join("s45/models")
        .join(reference["pack"].as_str().unwrap());
    // Public-only input is copied to an isolated directory; never consult app settings.
    let audio = fs::read(root.join(reference["clip"].as_str().unwrap())).unwrap();
    assert_eq!(hex::encode(Sha256::digest(&audio)), reference["sha256"]);
    let wav = temp.path().join("public.wav");
    fs::write(&wav, audio).unwrap();
    let samples = read_wav(&wav).unwrap();
    assert_eq!(
        samples.len() as u64,
        reference["end"].as_u64().unwrap() - reference["start"].as_u64().unwrap()
    );
    initialize(&dll).unwrap();
    let mut engine = IndicConformer::load_for_language(
        &pack.join("model.onnx"),
        &pack.join("tokens.txt"),
        language,
    )
    .unwrap();
    for _ in 0..2 {
        let result = engine.transcribe(&samples, language).unwrap();
        assert_eq!(result.text, reference["text"]);
        assert_eq!(
            serde_json::to_value(result.tokens.iter().map(|t| &t.text).collect::<Vec<_>>())
                .unwrap(),
            reference["tokens"]
        );
    }
    let other = if language == "hi" { "te" } else { "hi" };
    typed(engine.transcribe(&samples, other), IndicError::Language);
    typed(
        engine.transcribe(&vec![0.0; 480_001], language),
        IndicError::Audio,
    );
}

#[test]
#[cfg(windows)]
#[ignore = "requires the pinned local S45 Hindi pack, public clip and runtime DLL"]
fn indic_s45_hindi_parity() {
    parity("hi");
}

#[test]
#[cfg(windows)]
#[ignore = "requires the pinned local S45 Telugu pack, public clip and runtime DLL"]
fn indic_s45_telugu_parity() {
    parity("te");
}
