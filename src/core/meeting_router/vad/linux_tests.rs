//! `Silero::load` and `Silero::push` on Linux over the public 60 s `test_silero_vad.wav`, with the
//! pinned runtime and the catalogue's pinned Silero pack. Fetched files are hash checked; a mismatch fails.
use super::*;
use crate::core::asr::fixtures::{library, silero_store, test_wav};
use std::fs;

/// Spans (sample offsets) from the first Linux run of this clip with the pinned pack and runtime.
const EXPECTED: &[Span] = &[
    (512, 32256),
    (43008, 74752),
    (79872, 109568),
    (149504, 212992),
    (217088, 241152),
    (245760, 253440),
    (261120, 286208),
    (294400, 313344),
    (325632, 521216),
    (523776, 569344),
    (572416, 601600),
    (607744, 622592),
    (638464, 692224),
    (698368, 713728),
    (720896, 748544),
    (781824, 799232),
    (817664, 867328),
    (871936, 918016),
    (920576, 952832),
];

#[test]
fn push_over_public_wav_gives_spans_and_resumes_across_a_cold_session() {
    let library = library();
    let (_temp, store) = silero_store();
    let audio = test_wav();
    assert_eq!(audio.len(), 960_000);
    let mut vad = Silero::load(&store, &library).expect("Silero::load on Linux");
    let id = ("job", "wav");
    let whole = vad
        .push(&mut Checkpoint::default(), id, 0, 16000, &audio, true)
        .unwrap();
    eprintln!("spans {whole:?}");
    assert!(whole.windows(2).all(|w| w[0].1 <= w[1].0));
    assert!(whole.iter().all(|s| s.1 - s.0 > 4000 && s.1 <= 960_000));
    assert_eq!(whole, EXPECTED);

    let seam = 480_123;
    let mut state = Checkpoint::default();
    let mut resumed = vad
        .push(&mut state, id, 0, 16000, &audio[..seam], false)
        .unwrap();
    let mut restored = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    let mut cold = Silero::load(&store, &library).unwrap();
    let tail = cold.push(&mut restored, id, seam as u64, 16000, &audio[seam..], true);
    resumed.extend(tail.unwrap());
    assert_eq!(resumed, whole, "cold seam");
    for key in [("new-job", "wav"), ("new-job", "other-track")] {
        let reset = cold
            .push(&mut restored, key, 0, 16000, &audio, true)
            .unwrap();
        assert_eq!(reset, whole);
    }
}

#[test]
fn a_changed_pack_is_refused() {
    let library = library();
    let (temp, store) = silero_store();
    assert!(
        Silero::load(&store, &library).is_ok(),
        "the intact pack loads"
    );
    let dir = temp.path().join("models/silero-vad");
    let model = dir.join("silero_vad.onnx");
    let size = fs::metadata(&model).unwrap().len();
    fs::write(&model, vec![0; size as usize]).unwrap();
    // The receipt is rewritten so it is newer than the changed file; only the hash check can refuse.
    fs::write(
        dir.join("installed.json"),
        fs::read(dir.join("installed.json")).unwrap(),
    )
    .unwrap();
    assert!(matches!(Silero::load(&store, &library), Err(Unavailable)));
}
