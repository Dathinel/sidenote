use super::*;
mod authorization;
mod failed;
mod payment;
mod qualification;
use crate::core::meeting_router::{
    self as router,
    batches::{self, *},
    jobs, pieces,
};
use std::{
    io::Read,
    process::Command,
    sync::{Arc, atomic::AtomicBool},
};

fn child(mode: &str) -> Command {
    let mut c = Command::new(std::env::current_exe().unwrap());
    c.args([
        "--exact",
        "core::notes::tests::translation::fixture_child",
        "--nocapture",
        "--skip",
        mode,
    ]);
    c
}
#[test]
fn fixture_child() {
    let Some(mode) = std::env::args().skip_while(|a| a != "--skip").nth(1) else {
        return;
    };
    if mode == "recognizer-failed" {
        let mut start = String::new();
        std::io::stdin().read_line(&mut start).unwrap();
        assert_eq!(start, "start\n");
        std::process::exit(23);
    }
    if let Some(work) = mode.strip_prefix("router-failed|") {
        failed::router_child(Path::new(work));
    }
    if mode == "recognizer" {
        let output = PathBuf::from(std::env::var_os("SIDENOTE_BATCH_OUTPUT").unwrap());
        let mut lines = std::io::BufRead::lines(std::io::stdin().lock());
        assert_eq!(lines.next().unwrap().unwrap(), "start");
        for line in lines {
            assert_eq!(line.unwrap(), "range");
            let range: Range =
                serde_json::from_slice(&fs::read(output.join("range.json")).unwrap()).unwrap();
            let answer = serde_json::json!({"text":format!("selected {} words",range.range.language.unwrap())});
            store_io::write(&output.join("result.json"), &answer).unwrap();
        }
        std::process::exit(0);
    }
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    let (purpose, path) = mode.split_once('|').unwrap_or(("notes", &mode));
    let root = PathBuf::from(path);
    if purpose == "capability" {
        qualification::capability_child(&root, &input);
    }
    if purpose == "translation" {
        assert!(
            std::env::var("OPENCODE_CONFIG_CONTENT")
                .is_ok_and(|c| { fs::read_to_string("opencode.json").is_ok_and(|f| f == c) })
        );
        fs::write(root.join("translation-input"), input).unwrap();
        let config: serde_json::Value =
            serde_json::from_slice(&fs::read("opencode.json").unwrap()).unwrap();
        let binding = &config["sidenote_route"];
        let expected_model = queue::read(&root, ID)
            .unwrap()
            .unwrap()
            .translation
            .unwrap()
            .snapshot
            .unwrap()
            .model;
        assert!(
            binding["model"] == expected_model
                && binding["provider"] == "fixture"
                && binding["id"] == "subscription"
                && binding["auth_mode"] == "subscription"
                && binding["payment"] == "subscription"
                && binding["location"] == "hosted"
                && binding["automatic_fallback"] == false
        );
        if root.join("emit-tool").exists() {
            println!(
                "{}",
                serde_json::json!({"type":"tool_use","part":{"tool":"read","callID":"forbidden"}})
            );
        }
        println!(
            "{}",
            serde_json::json!({"type":"text","part":{"text":"valid-looking translation output"}})
        );
        std::process::exit(0);
    }
    let tasks = input.contains("## Known projects");
    fs::write(
        root.join(if tasks { "tasks-input" } else { "notes-input" }),
        input,
    )
    .unwrap();
    println!(
        "{}",
        serde_json::json!({"type":"result","is_error":false,"result":if tasks { sorted() } else { summary() }})
    );
    std::process::exit(0);
}
struct Recognizers;
impl NativeLoader for Recognizers {
    fn load(
        &mut self,
        _: NativeLanguage,
        _: &Range,
    ) -> crate::core::asr::Result<Box<dyn Recognizer>> {
        Ok(Box::new(batches::worker::Worker::spawn(
            child("recognizer"),
            Arc::new(AtomicBool::new(false)),
            Duration::from_secs(10),
        )?))
    }
}
impl EnglishBridge for Recognizers {
    fn transcribe(&mut self, r: &Range) -> crate::core::asr::Result<serde_json::Value> {
        let mut worker = self.load(NativeLanguage::Hindi, r)?;
        let reading = worker.recognize(r)?;
        worker.finish()?;
        Ok(reading)
    }
}
fn fixture() -> (Temp, PathBuf, NotesSettings) {
    let (root, folder, settings) = routing::setup();
    let raw = (0..3).map(|i| serde_json::json!({"start_ms":i*1000,"end_ms":(i+1)*1000,"source":"system","text":vec![format!("baseline{i}");25].join(" ")}).to_string()).collect::<Vec<_>>().join("\n");
    fs::write(folder.join("transcript.ndjson"), raw).unwrap();
    crate::core::asr::write_wav(&folder.join("system.wav"), &vec![0.0; 48000]).unwrap();
    let baseline = transcript::chosen(&root.0, ID, &folder, None, 0, 1)
        .unwrap()
        .unwrap()
        .transcript
        .baseline;
    jobs::enqueue_in(&root.0, &root.0.join("meetings"), ID).unwrap();
    let pending: serde_json::Value =
        store_io::read(&root.0.join("local/router-queue").join(format!("{ID}.json"))).unwrap();
    let generation = pending["generation"].as_u64().unwrap() as u128;
    let manifest = pieces::plan(
        &folder,
        &std::collections::BTreeMap::from([("system".into(), 0)]),
        &router::RoutingConfig::default(),
    )
    .unwrap();
    let source = manifest.source_revision.clone();
    let work = root.0.join("local/router-work").join(ID).join("batches");
    fs::create_dir_all(&work).unwrap();
    let ranges = ["en","hi","te"].iter().enumerate().map(|(i, lang)| {
        let a = i as u64 * 16000; let b = a + 16000;
        let wav = work.join(format!("{lang}.wav"));
        crate::core::asr::write_wav(&wav, &vec![0.0;16000]).unwrap();
        serde_json::from_value(serde_json::json!({"track":"system","piece":manifest.tracks[0].pieces[0].id,"wav":wav,"range":{"context":[a,b],"members":[[0,[a,b]]],"language":lang},"model_id":format!("fixture-{lang}"),"model_sha256":"a".repeat(64)})).unwrap()
    }).collect();
    let plan = batches::spool(&work, ranges).unwrap();
    batches::run(&work, &plan, &mut Recognizers, &mut Recognizers).unwrap();
    let output = work.join("output.json");
    jobs::BatchOutput::publish(&output, generation, &source, plan, manifest, vec![]).unwrap();
    jobs::commit_batches(&root.0, ID, generation, &source, &baseline).unwrap();
    (root, folder, settings)
}
struct Boundary;
impl Drop for Boundary {
    fn drop(&mut self) {
        let _ = crate::core::runner_process::TEST_CHILD.with(|c| c.replace(None));
    }
}
fn boundary(root: &Path) -> Boundary {
    let root = root.to_owned();
    let factory = move |command: &Command| {
        let args: Vec<_> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let translating = args.iter().any(|a| a == "--standalone");
        let capability = args.iter().any(|a| a == "--sidenote-capability");
        if translating && !capability {
            let job = queue::read(&root, ID).unwrap().unwrap();
            let snapshot = job.translation.unwrap().snapshot.unwrap();
            assert_eq!(
                job.transcript_revision.as_deref(),
                Some(snapshot.revision.as_str())
            );
            assert_eq!(
                args.join(" "),
                format!(
                    "run --standalone --agent sidenote-translation --model {} --format json",
                    snapshot.model
                )
            );
            let content = command
                .get_envs()
                .find(|(k, _)| *k == "OPENCODE_CONFIG_CONTENT")
                .unwrap()
                .1
                .unwrap();
            let config: serde_json::Value =
                serde_json::from_str(content.to_str().unwrap()).unwrap();
            assert_eq!(
                fs::read_to_string(command.get_current_dir().unwrap().join("opencode.json"))
                    .unwrap(),
                content.to_str().unwrap()
            );
            assert_eq!(config["agents"]["sidenote-translation"]["steps"], 1);
            let deny = serde_json::json!([{"action":"*","resource":"*","effect":"deny"}]);
            assert_eq!(config["permissions"], deny);
            assert_eq!(
                config["agents"]["sidenote-translation"]["permissions"],
                deny
            );
            fs::write(
                root.join("translation-invocation.json"),
                serde_json::to_vec(&serde_json::json!({"args":args,"config":config})).unwrap(),
            )
            .unwrap();
            if root.join("withdraw-before-spawn").exists() {
                let mut config = crate::core::Config::load_in(&root).unwrap();
                config.engine_settings.steps.translation = false;
                config.save(&root).unwrap();
                fs::write(root.join("withdrawal-fired"), "").unwrap();
            }
        }
        let mode = format!(
            "{}|{}",
            if capability {
                "capability"
            } else if translating {
                "translation"
            } else {
                "notes"
            },
            root.display()
        );
        let mut result = child(&mode);
        result.current_dir(command.get_current_dir().unwrap());
        for (key, value) in command.get_envs() {
            if let Some(value) = value {
                result.env(key, value);
            }
        }
        result
    };
    let _ = crate::core::runner_process::TEST_CHILD.with(|c| c.replace(Some(Box::new(factory))));
    Boundary
}
fn process(root: &Path, settings: &NotesSettings) {
    let _boundary = boundary(root);
    let mut cli = helper::Cli {
        root: Some(root.to_owned()),
    };
    queue::drain(root, &root.join("meetings"), &mut cli, || {
        Ok(settings.clone())
    })
    .unwrap();
}
#[test]
fn translation_input_uses_released_selection() {
    for labs in [true, false] {
        let (root, folder, settings) = fixture();
        fs::write(folder.join("english-transcript.md"), "stale export").unwrap();
        if !labs {
            let mut c = crate::core::Config::load_in(&root.0).unwrap();
            c.labs.after_stop_language_routing = false;
            store_io::write(&root.0.join("config.json"), &c).unwrap();
        }
        enqueue(&root.0, &settings, None);
        process(&root.0, &settings);
        let job = queue::read(&root.0, ID).unwrap().unwrap();
        assert!(matches!(job.state, JobState::Done { .. }));
        let result = job.translation.unwrap();
        let input = fs::read_to_string(root.0.join("notes-input")).unwrap();
        if labs {
            let pin = job.transcript_revision.as_deref();
            let selection = transcript::chosen(&root.0, ID, &folder, pin, 0, 1)
                .unwrap()
                .unwrap();
            assert_eq!(
                selection
                    .transcript
                    .segments
                    .iter()
                    .map(|s| (s.lang.as_str(), s.translate))
                    .collect::<Vec<_>>(),
                [("en", false), ("hi", true), ("te", true)]
            );
            let keys: Vec<_> = (0..3)
                .map(|i| format!("{}:{i}", selection.revision))
                .collect();
            assert_eq!(result.segment_keys, keys);
            assert_eq!(result.source, transcript::Source::ReleasedRevision);
            assert_eq!(result.reason, "translation_off");
            for lang in ["en", "hi", "te"] {
                assert!(input.contains(&format!("selected {lang} words")));
            }
            assert!(!input.contains("stale export"));
        } else {
            assert_eq!(result.reason, "language_passes_not_available");
            assert!(input.contains("stale export"));
        }
        assert_eq!(
            fs::read_to_string(folder.join("english-transcript.md")).unwrap(),
            "stale export"
        );
    }
}

fn retry(root: &Path, folder: &Path, settings: &NotesSettings) {
    fs::remove_file(folder.join("english-summary.json")).unwrap();
    enqueue(root, settings, Some(Request::TryAgain));
    process(root, settings);
}
#[test]
fn translation_requires_explicit_activation_and_pins_settings() {
    let (root, folder, settings) = fixture();
    let mut config = crate::core::Config::load_in(&root.0).unwrap();
    assert!(
        config
            .engine_settings
            .stages
            .translation_enabled_at_ms
            .is_none()
    );
    config.engine_settings.steps.translation = true;
    config.save(&root.0).unwrap();
    enqueue(&root.0, &settings, None);
    process(&root.0, &settings);
    assert!(
        queue::read(&root.0, ID)
            .unwrap()
            .unwrap()
            .translation
            .unwrap()
            .snapshot
            .is_none()
    );
    assert!(!root.0.join("translation-input").exists());
    config.engine_settings.stages.translation_enabled_at_ms = Some(1);
    config.engine_settings.stages.translation_model = "translation-original".into();
    config.engine_settings.stages.translation_deadline_seconds = 17;
    config.engine_settings.translation_prompt = Some("First translation instructions".into());
    config.save(&root.0).unwrap();
    retry(&root.0, &folder, &settings);
    let first = queue::read(&root.0, ID).unwrap().unwrap();
    let snapshot = first.translation.unwrap().snapshot.unwrap();
    assert_eq!(snapshot.revision, first.transcript_revision.unwrap());
    assert_eq!(snapshot.helper, Helper::OpenCode);
    assert_eq!(snapshot.model, "translation-original");
    assert_eq!(snapshot.deadline_seconds, 17);
    assert_eq!(snapshot.prompt, "First translation instructions");
    assert_eq!(snapshot.location, "hosted");
    assert_eq!(snapshot.payment, "subscription");
    config.engine_settings.stages.translation_model = "translation-edited".into();
    config.engine_settings.stages.translation_deadline_seconds = 23;
    config.engine_settings.translation_prompt = Some("Later instructions".into());
    config.save(&root.0).unwrap();
    retry(&root.0, &folder, &settings);
    let again = queue::read(&root.0, ID).unwrap().unwrap();
    let pinned = again.translation.unwrap().snapshot.unwrap();
    assert_eq!(pinned.model, snapshot.model);
    assert_eq!(pinned.deadline_seconds, snapshot.deadline_seconds);
    assert_eq!(pinned.prompt, snapshot.prompt);
    assert_eq!(again.model, first.model);
    config.engine_settings.stages.translation_helper = Helper::Codex;
    config.save(&root.0).unwrap();
    retry(&root.0, &folder, &settings);
    assert_eq!(
        queue::read(&root.0, ID)
            .unwrap()
            .unwrap()
            .translation
            .unwrap()
            .reason,
        "translation_consent_changed"
    );
    config.engine_settings.steps.translation = false;
    config.save(&root.0).unwrap();
    let _ = fs::remove_file(root.0.join("translation-input"));
    retry(&root.0, &folder, &settings);
    let revoked = queue::read(&root.0, ID).unwrap().unwrap();
    assert_eq!(revoked.translation.unwrap().reason, "translation_off");
    assert!(!root.0.join("translation-input").exists());
    assert!(matches!(revoked.state, JobState::Done { .. }));
}

#[test]
fn translation_uses_shared_text_only_child() {
    for (helper, reason) in [
        (Helper::OpenCode, "translation_validation_not_available"),
        (Helper::OpenCode, "helper_tool_use"),
        (Helper::Codex, "helper_not_text_only"),
    ] {
        let tool = reason == "helper_tool_use";
        let (root, folder, settings) = fixture();
        let mut config = crate::core::Config::load_in(&root.0).unwrap();
        config.engine_settings.steps.translation = true;
        config.engine_settings.stages.translation_enabled_at_ms = Some(1);
        config.engine_settings.stages.translation_helper = helper;
        config.engine_settings.stages.translation_model = "translation-fixture".into();
        config.engine_settings.translation_prompt =
            Some("Distinctive translation instructions".into());
        config.save(&root.0).unwrap();
        if tool {
            fs::write(root.0.join("emit-tool"), "").unwrap();
        }
        enqueue(&root.0, &settings, None);
        process(&root.0, &settings);
        let job = queue::read(&root.0, ID).unwrap().unwrap();
        assert!(matches!(job.state, JobState::Done { .. }));
        assert_eq!(job.helper, Some(Helper::Claude));
        let disposition = job.translation.unwrap();
        assert_eq!(disposition.reason, reason);
        let input = fs::read_to_string(root.0.join("notes-input")).unwrap();
        for lang in ["en", "hi", "te"] {
            assert!(input.contains(&format!("selected {lang} words")));
        }
        assert!(!folder.join("english-transcript.md").exists());
        if helper == Helper::OpenCode {
            let sent = fs::read_to_string(root.0.join("translation-input")).unwrap();
            assert!(sent.contains("Distinctive translation instructions"));
            assert!(sent.contains("selected hi words") && sent.contains("selected te words"));
            assert!(!sent.contains("selected en words"));
            assert!(!helper::authorized(&root.0, Helper::OpenCode).unwrap());
            if !tool {
                fs::remove_file(root.0.join("translation-input")).unwrap();
                for request in [None, Some(Request::SortAgain), Some(Request::RedoTasks)] {
                    enqueue(&root.0, &settings, request);
                    process(&root.0, &settings);
                    assert!(!root.0.join("translation-input").exists());
                }
            }
        } else {
            assert!(!root.0.join("translation-input").exists());
        }
    }
}
