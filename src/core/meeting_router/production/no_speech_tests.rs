//! Synthetic process replies through the production English adapter and batch entry.
use super::*;
use crate::core::meeting_router::{pieces, ranges::AsrRange};
use std::collections::BTreeMap;

pub(super) struct Fixture {
    pub root: tempfile::TempDir,
    pub work: PathBuf,
    pub plan: Plan,
    pub bridge: English,
    pub manifest: pieces::PieceManifest,
    pub baseline: Vec<Chunk>,
    _dispatch: std::sync::MutexGuard<'static, ()>,
}
impl Fixture {
    pub fn new(reply: &[u8], exit: i32, stderr: &[u8]) -> Self {
        let dispatch = super::super::hex_bridge::test_dispatch_guard();
        let root = tempfile::tempdir().unwrap();
        let work = root.path().join("local/router-work/123-4/batches");
        std::fs::create_dir_all(&work).unwrap();
        let dir = &work;
        let executable = dir.join("fake-hex.exe");
        fs::copy(super::fixture_tools::executable(false), &executable).unwrap();
        for (name, bytes) in [("reply", reply), ("stderr", stderr)] {
            std::fs::write(dir.join(name), bytes).unwrap();
        }
        std::fs::write(dir.join("exit"), exit.to_string()).unwrap();
        let mut command = Command::new(&executable);
        command.arg("probe");
        let probe = super::fixture_tools::output(command, Duration::from_secs(5)).unwrap();
        assert_eq!(probe.status.code(), Some(exit));
        assert_eq!(
            probe.stdout, reply,
            "fake must emit the exact recognizer bytes"
        );
        assert_eq!(probe.stderr, stderr);
        std::fs::remove_file(dir.join("calls")).unwrap();
        let wav = dir.join("system.wav");
        asr::write_wav(&wav, &[0.0; 96_000]).unwrap();
        store_io::write(&dir.join("manifest.json"), &json!({"status":"complete"})).unwrap();
        store_io::atomic_write(&dir.join("transcript.ndjson"), b"").unwrap();
        let manifest = pieces::plan(
            dir,
            &BTreeMap::from([("system".into(), 0)]),
            &RoutingConfig::default(),
        )
        .unwrap();
        let cut = dir.join("range.wav");
        asr::write_wav(&cut, &[0.0; 3200]).unwrap();
        let ranges = [
            ("en", (14_400, 17_600)),
            ("en", (46_400, 49_600)),
            ("hi", (64_000, 80_000)),
            ("te", (80_000, 96_000)),
        ]
        .into_iter()
        .map(|(lang, context)| Range {
            track: "system".into(),
            piece: manifest.tracks[0].pieces[0].id.clone(),
            wav: cut.clone(),
            range: AsrRange {
                context,
                members: vec![(0, context)],
                language: Some(lang.into()),
            },
            model_id: if lang == "en" {
                "parakeet_unified_en".into()
            } else {
                format!("fake-{lang}")
            },
            model_sha256: "a".repeat(64),
        })
        .collect();
        let plan = batches::spool(dir, ranges).unwrap();
        let baseline = (0..6)
            .map(|i| {
                serde_json::from_value(json!({
            "id":format!("baseline-{i}"), "start":i, "end":i+1, "track":"system",
            "english":format!("public baseline {i}")}))
                .unwrap()
            })
            .collect();
        Self {
            root,
            work,
            plan,
            bridge: English(HexBridge::new(HexSpeech { executable })),
            manifest,
            baseline,
            _dispatch: dispatch,
        }
    }
    pub fn run(&mut self, native: &mut impl NativeLoader) -> Result<()> {
        batches::run(&self.work, &self.plan, native, &mut self.bridge)
    }
}
#[derive(Default)]
pub(super) struct NativeFake(pub Vec<String>);
struct Session;
impl Recognizer for Session {
    fn recognize(&mut self, _: &Range) -> Result<Value> {
        Ok(json!({"text":"public native"}))
    }
}
impl NativeLoader for NativeFake {
    fn load(&mut self, _: NativeLanguage, range: &Range) -> Result<Box<dyn Recognizer>> {
        self.0.push(range.range.language.clone().unwrap());
        Ok(Box::new(Session))
    }
}
fn no_speech(reply: &[u8], exit: i32) {
    let mut f = Fixture::new(reply, exit, b"");
    let mut native = NativeFake::default();
    f.run(&mut native)
        .expect("no-speech must keep the remaining language passes running");
    assert_eq!(native.0, ["hi", "te"]);
    assert_eq!(std::fs::read_to_string(f.work.join("calls")).unwrap(), "2");
    let missing: Vec<_> = f.plan.batches["en"]
        .keys()
        .filter(|id| {
            batches::read_result(&f.work, &f.plan, id)
                .unwrap()
                .is_none()
        })
        .collect();
    assert_eq!(missing.len(), 1, "no empty ASR receipt may be published");
    let receipt = f
        .work
        .join(format!("{}-{}.receipt", f.plan.revision, missing[0]));
    assert!(
        !receipt.exists(),
        "no ASR receipt may be published for no-speech"
    );
    let revision = revision::assemble(&f.work, &f.plan, &f.manifest, &f.baseline, 1).unwrap();
    assert_eq!(
        revision.provenance["no_speech"][missing[0]]["reason"],
        "no-speech"
    );
    for baseline in &f.baseline {
        assert!(super::super::baseline::source_covered(
            baseline,
            &revision.transcript.segments
        ));
    }
    let absent_range = &f.plan.batches["en"][missing[0]];
    let first = (absent_range.range.context.0 / 16000) as usize;
    for unit in &f.baseline[first..first + 2] {
        assert!(
            revision
                .transcript
                .segments
                .iter()
                .any(|s| s.text_original == unit.english
                    && s.start == unit.start
                    && s.end == unit.end)
        );
    }
    store_io::write(&f.work.join("output.json"), &json!({"generation":"1", "source_revision":"synthetic", "plan":f.plan, "manifest":f.manifest})).unwrap();
    store_io::write(
        &f.root.path().join("local/router-queue/123-4.json"),
        &json!({"id":"123-4", "generation":1}),
    )
    .unwrap();
    let published = f.root.path().join("local/transcripts/123-4");
    revision::begin(&published, 1).unwrap();
    let id = jobs::commit_batches(f.root.path(), "123-4", 1, "synthetic", &f.baseline).unwrap();
    let saved = revision::load(&published, Some(&id)).unwrap().unwrap().1;
    assert_eq!(
        saved.provenance["no_speech"],
        revision.provenance["no_speech"]
    );
    assert!(
        revision::assemble(&f.work, &f.plan, &f.manifest, &[], 1).is_err(),
        "coverage gate still rejects absent baseline"
    );
}
#[test]
fn production_blank_success_keeps_baseline_and_remaining_languages() {
    no_speech(b"\n", 0);
}
#[test]
fn production_structured_no_speech_keeps_baseline_and_remaining_languages() {
    no_speech(
        br#"{"code":"no-speech","message":"no speech was heard in the file; nothing to print"}"#,
        4,
    );
}

fn fatal_with_cause(reply: &[u8], exit: i32) {
    let mut stderr = b"recognizer allocation failed C:\\private\\owner.txt /private/owner.txt confidential_meeting_words\n".to_vec();
    stderr.extend(vec![b'x'; 65_536]);
    let mut f = Fixture::new(reply, exit, &stderr);
    if exit == 0 {
        let direct = HexSpeech {
            executable: f.work.join("fake-hex.exe"),
        };
        assert_eq!(
            direct
                .run_for(&["probe"], None, Duration::from_secs(2))
                .unwrap()
                .as_bytes(),
            reply
        );
        std::fs::remove_file(f.work.join("calls")).unwrap();
    }
    let mut native = NativeFake::default();
    let message = f.run(&mut native).unwrap_err().to_string();
    assert!(
        native.0.is_empty(),
        "fatal English errors must stop the pass"
    );
    assert!(
        message.contains(&format!("exit={exit}")),
        "missing exit code: {message}"
    );
    assert!(
        message.contains("allocation failed"),
        "missing stderr excerpt: {message}"
    );
    if !reply.is_empty() {
        assert!(
            message.contains("unrecognized-code"),
            "missing JSON code: {message}"
        );
        assert!(
            message.contains("decoder failed"),
            "missing JSON reason: {message}"
        );
    }
    for private in [
        "owner.txt",
        "private",
        "confidential_meeting_words",
        "x".repeat(80).as_str(),
    ] {
        assert!(!message.contains(private), "unsanitized child diagnostic");
    }
    assert!(message.len() < 1400, "diagnostics must be bounded");
    for id in f.plan.batches.values().flat_map(|b| b.keys()) {
        assert!(
            batches::read_result(&f.work, &f.plan, id)
                .unwrap()
                .is_none()
        );
    }
}
#[test]
fn production_nonzero_exit_remains_fatal_with_cause() {
    fatal_with_cause(b"", 7);
}
#[test]
fn production_unknown_code_remains_fatal_with_cause() {
    for exit in [0, 4] {
        fatal_with_cause(br#"{"code":"unrecognized-code","reason":"decoder failed","text":"confidential_meeting_words"}"#, exit);
    }
}
