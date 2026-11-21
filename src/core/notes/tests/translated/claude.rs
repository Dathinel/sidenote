use super::*;
use crate::core::runner_process::{
    TEST_CHILD,
    deadline::fixture::{FrozenClock, Phase},
};
use serde_json::json;
use std::{io::Read, process::Command, time::Instant};

const BILLING_NAMES: [&str; 5] = [
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "oThEr_Service_TOKEN",
];

const WAITING_FOR_REPLY: &str = "claude-waiting-for-reply";
const HOLDING_PIPES: &str = "claude-holding-pipes";
const RELEASE: &str = "release-claude";
const WATCHDOG: &str = "claude-watchdog";
const DEADLINE_SECONDS: u32 = 1;

fn wait_for_release(marker: &Path) {
    let root = marker.parent().unwrap();
    fs::write(marker, "").unwrap();
    let start = Instant::now();
    while root.is_dir() && !root.join(RELEASE).exists() {
        if start.elapsed() >= Duration::from_secs(30) {
            let _ = fs::write(root.join(WATCHDOG), format!("{:?}", start.elapsed()));
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn child(mode: &str, marker: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "core::notes::tests::translated::claude::fixture_child",
            "--nocapture",
            "--skip",
            &format!("claude-fixture:{mode}"),
        ])
        .arg("--skip")
        .arg(marker);
    command
}

#[test]
fn fixture_child() {
    let mut args = std::env::args_os();
    let Some(mode) = args.find_map(|arg| {
        arg.to_str()?
            .strip_prefix("claude-fixture:")
            .map(str::to_owned)
    }) else {
        return;
    };
    assert_eq!(args.next().as_deref().unwrap(), "--skip");
    let marker = PathBuf::from(args.next().unwrap());
    if mode == "hold-pipes" {
        wait_for_release(&marker);
        return;
    }
    fs::write(&marker, "").unwrap();
    for name in BILLING_NAMES.into_iter().chain(["HOME", "PATH"]) {
        assert!(std::env::var_os(name).is_none(), "{name} leaked");
    }
    let mut prompt = String::new();
    std::io::stdin().read_to_string(&mut prompt).unwrap();
    let (_, data) = prompt.split_once("Selected transcript data:\n").unwrap();
    let items: Lines = serde_json::from_str(data).unwrap();
    let segments: Lines = items
        .iter()
        .map(|line| json!({"id": line["id"], "text_en": "English fixture"}))
        .collect();
    let text = json!({"segments": segments}).to_string();
    let mut reply =
        json!({"type": "result", "subtype": "success", "is_error": false, "result": text});
    let mut content = vec![json!({"type": "text", "text": text})];
    match mode.as_str() {
        "partial" => {
            reply = json!({"type": "assistant", "message": {"content": content}});
        }
        "tools" => {
            content.push(json!({"type": "tool_use", "name": "Read"}));
            eprintln!(
                "{}",
                json!({"type": "assistant", "message": {"content": content}})
            );
        }
        "error" => {
            eprintln!(
                "{}",
                json!({"type": "result", "subtype": "error_during_execution", "is_error": true})
            );
        }
        "missing" => {
            reply.as_object_mut().unwrap().remove("is_error");
        }
        "malformed" => reply["is_error"] = "false".into(),
        "refusal" => reply["stop_reason"] = "refusal".into(),
        "late" => {
            wait_for_release(&marker.with_file_name(WAITING_FOR_REPLY));
            return;
        }
        "pipes" => {
            let mut holder = child("hold-pipes", &marker.with_file_name(HOLDING_PIPES));
            holder.stdin(std::process::Stdio::null());
            hold_pipes(holder);
        }
        "ok" => {}
        _ => panic!("unexpected Claude fixture mode {mode}"),
    }
    eprintln!("{reply}");
    std::process::exit(0);
}

fn hold_pipes(mut command: Command) {
    let mut holder = command.spawn().unwrap();
    std::thread::spawn(move || holder.wait());
}

struct Fixture {
    cli: helper::Cli,
    notes: Fake,
}
impl Run for Fixture {
    fn translate(
        &mut self,
        snapshot: &Snapshot,
        prompt: String,
        end: Instant,
    ) -> Result<String, &'static str> {
        self.cli.translate(snapshot, prompt, end)
    }
    fn installed(&self, h: Helper) -> bool {
        self.notes.installed(h)
    }
    fn run(&mut self, h: Helper, m: &str, p: String, t: Duration) -> Result<String, Failure> {
        self.notes.run(h, m, p, t)
    }
}

struct ChildBoundary;
impl Drop for ChildBoundary {
    fn drop(&mut self) {
        let _ = TEST_CHILD.with(|c| c.take());
    }
}
fn run_claude(mode: &'static str) -> (Temp, String, Fake) {
    let (root, _, settings) = meeting_with(&["en", "hi", "te"]);
    let mut config = crate::core::Config::load_in(&root.0).unwrap();
    config.engine_settings.stages.translation_helper = Helper::Claude;
    let phase = match mode {
        "delayed" => Some((Phase::Launch, "claude-prepared")),
        "late" => Some((Phase::Reply, WAITING_FOR_REPLY)),
        "pipes" => Some((Phase::Pipes, HOLDING_PIPES)),
        _ => None,
    };
    let timed = phase.is_some();
    if timed {
        config.engine_settings.stages.translation_deadline_seconds = DEADLINE_SECONDS;
    }
    config.save(&root.0).unwrap();
    let _held = (mode == "locked").then(|| crate::core::config::lock(&root.0).unwrap());
    let path = root.0.clone();
    let factory = move |command: &Command| {
        let args = command
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect::<Vec<_>>()
            .join("|");
        assert_eq!(
            args,
            "-p|--model|translation-fixture|--strict-mcp-config|--tools||--no-session-persistence|--output-format|json"
        );
        fs::create_dir(path.join("claude-prepared")).unwrap();
        if mode == "revoked" {
            let mut config = crate::core::Config::load_in(&path).unwrap();
            config.engine_settings.steps.translation = false;
            config.save(&path).unwrap();
        }
        let mut child = child(mode, &path.join("claude-started"));
        for name in BILLING_NAMES {
            child.env(name, "fixture");
        }
        child
    };
    if mode != "unqualified" {
        TEST_CHILD.with(|c| assert!(c.replace(Some(Box::new(factory))).is_none()));
    }
    let _boundary = ChildBoundary;
    let mut run = Fixture {
        cli: helper::Cli {
            root: Some(root.0.clone()),
        },
        notes: Fake::new(vec![Ok(summary()), Ok(sorted())]),
    };
    // Keep the same deadline from batch dispatch through cleanup and expire it at the observed phase.
    let _clock = phase.map(|(target, marker)| {
        let ready = root.0.join(marker);
        FrozenClock::until(
            Duration::from_secs(u64::from(DEADLINE_SECONDS)),
            move |phase| {
                assert!(
                    (target, phase) != (Phase::Launch, Phase::Spawn),
                    "delayed: runner attempted to spawn after launch deadline"
                );
                phase == target && ready.exists()
            },
        )
    });
    queue::drain(&root.0, &root.0.join("meetings"), &mut run, || {
        Ok(settings.clone())
    })
    .unwrap();
    let job = queue::read(&root.0, ID).unwrap().unwrap();
    assert!(matches!(job.state, JobState::Done { .. }));
    if timed {
        fs::write(root.0.join(RELEASE), "").unwrap();
        let watchdog = root.0.join(WATCHDOG);
        assert!(
            !watchdog.exists(),
            "{mode}: fixture watchdog fired after {}",
            fs::read_to_string(&watchdog).unwrap_or_default()
        );
    }
    if mode != "ok" {
        let notes = &run.notes.calls[0].0;
        assert!(notes.contains("\u{915}\u{915}\u{915} 11"), "{mode}");
    }
    (root, job.translation.unwrap().reason, run.notes)
}

#[test]
fn claude_translation_runs_tool_free() {
    let (root, reason, notes) = run_claude("ok");
    assert_eq!(reason, "translated");
    assert!(root.0.join("claude-started").is_file());
    for lang in ["Hindi", "Telugu"] {
        assert!(
            notes.calls[0]
                .0
                .contains(&format!("({lang}, translated): English fixture"))
        );
    }
}

#[test]
fn revoked_before_spawn() {
    for mode in ["revoked", "locked"] {
        let (root, reason, _) = run_claude(mode);
        assert_eq!(reason, "translation_consent_changed", "{mode}");
        assert!(!root.0.join("claude-started").exists(), "{mode}");
    }
}

#[test]
fn unqualified_claude_is_refused() {
    let (_, reason, _) = run_claude("unqualified");
    assert_eq!(reason, "helper_unqualified");
}

#[test]
fn deadline_covers_launch_reply_and_cleanup() {
    for mode in ["delayed", "late", "pipes"] {
        let (root, reason, _) = run_claude(mode);
        assert_eq!(reason, "translation_timed_out", "{mode}");
        assert_eq!(
            root.0.join("claude-started").exists(),
            mode != "delayed",
            "{mode}: child start marker"
        );
    }
}

#[test]
fn claude_requires_one_successful_result_without_tools() {
    for mode in ["partial", "error", "missing", "malformed", "refusal"] {
        let (_, reason, _) = run_claude(mode);
        assert_eq!(reason, "helper_unsupported", "{mode}");
    }
    assert_eq!(run_claude("tools").1, "helper_tool_use");
}
