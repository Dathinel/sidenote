use super::*;
use crate::core::runner_process::TEST_CHILD;
use std::{process::Command, time::Instant};

const BILLING_NAMES: [&str; 4] = [
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
];

const REPLY: &str = r#"
import json, os, sys
from pathlib import Path
Path(sys.argv[1]).touch()
assert all(name not in os.environ for name in sys.argv[2:])
_, data = sys.stdin.read().split("Selected transcript data:\n", 1)
segments = [{"id": line["id"], "text_en": "English fixture"} for line in json.loads(data)]
print(json.dumps({"type": "result", "is_error": False, "result": json.dumps({"segments": segments})}))
"#;

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
fn run_claude(revoke: bool) -> (Temp, String, Fake) {
    let (root, _, settings) = meeting_with(&["en", "hi", "te"]);
    let mut config = crate::core::Config::load_in(&root.0).unwrap();
    config.engine_settings.stages.translation_helper = Helper::Claude;
    config.save(&root.0).unwrap();
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
        if revoke {
            let mut config = crate::core::Config::load_in(&path).unwrap();
            config.engine_settings.steps.translation = false;
            config.save(&path).unwrap();
        }
        let mut child = Command::new("/usr/bin/python3");
        child
            .env_clear()
            .args(["-c", REPLY])
            .arg(path.join("claude-started"))
            .args(BILLING_NAMES);
        for name in BILLING_NAMES {
            child.env(name, "fixture");
        }
        for (name, value) in command.get_envs() {
            if value.is_none() {
                child.env_remove(name);
            }
        }
        child
    };
    TEST_CHILD.with(|c| assert!(c.replace(Some(Box::new(factory))).is_none()));
    let _boundary = ChildBoundary;
    let mut run = Fixture {
        cli: helper::Cli {
            root: Some(root.0.clone()),
        },
        notes: Fake::new(vec![Ok(summary()), Ok(sorted())]),
    };
    queue::drain(&root.0, &root.0.join("meetings"), &mut run, || {
        Ok(settings.clone())
    })
    .unwrap();
    let job = queue::read(&root.0, ID).unwrap().unwrap();
    assert!(matches!(job.state, JobState::Done { .. }));
    assert!(root.0.join("claude-prepared").is_dir());
    (root, job.translation.unwrap().reason, run.notes)
}

#[test]
fn claude_translation_runs_tool_free() {
    let (root, reason, notes) = run_claude(false);
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
    let (root, reason, notes) = run_claude(true);
    assert_eq!(reason, "translation_consent_changed");
    assert!(!root.0.join("claude-started").exists());
    assert!(notes.calls[0].0.contains("\u{915}\u{915}\u{915} 11"));
}
