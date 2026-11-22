use super::{MemoryStore, RevisionRequest};
use crate::core::{Config, ask, notes, runner_process};
use serde::Deserialize;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(90);
const FAILED: &str = "Could not rewrite this memory. Try again.";

impl MemoryStore {
    pub fn spawn_rewrite(
        self,
        request: RevisionRequest,
        cancel: ask::Cancel,
        finished: impl FnOnce(Result<bool, &'static str>) + Send + 'static,
    ) {
        ask::spawn_cancellable(cancel, move || {
            let result = std::panic::catch_unwind(|| self.rewrite(&request)).unwrap_or(Err(FAILED));
            finished(result);
        });
    }

    pub fn rewrite(&self, request: &RevisionRequest) -> Result<bool, &'static str> {
        if stopped() {
            return Ok(false);
        }
        let text = self.answer(request, TIMEOUT)?;
        if stopped() {
            return Ok(false);
        }
        self.save_revision(request, &text).map_err(|_| FAILED)
    }

    fn answer(&self, request: &RevisionRequest, timeout: Duration) -> Result<String, &'static str> {
        let settings = Config::load_existing(&self.root)
            .map_err(|_| FAILED)?
            .notes
            .ok_or(FAILED)?;
        let helper = settings
            .effective_helper()
            .ok_or("Choose a notes helper to rewrite this memory.")?;
        let mut runner = notes::helper::runner(helper, &settings.model);
        if helper == notes::Helper::Codex {
            runner.args.pop();
            runner.args.extend(
                [
                    "-c",
                    "mcp_servers={}",
                    "-c",
                    "features.shell_tool=false",
                    "-c",
                    "features.unified_exec=false",
                    "-",
                ]
                .map(str::to_owned),
            );
        }
        let scratch = tempfile::Builder::new()
            .prefix("sidenote-memory-")
            .tempdir()
            .map_err(|_| FAILED)?;
        let (text, sources, comment) = request.input();
        let input = serde_json::json!({
            "instructions": "Rewrite the proposal using the owner's comment. Keep its source references and do not invent evidence. Return only a JSON object with a text string, up to 4000 characters.",
            "proposal": text,
            "sources": sources,
            "comment": comment,
        });
        let authorize = || !stopped() && self.sources_visible(sources);
        let reply = runner_process::run_classified(
            runner_process::Invocation::General(&runner, Some(&authorize)),
            input.to_string(),
            scratch.path(),
            timeout,
            true,
            |_| {},
        )
        .map_err(|_| FAILED)?;
        parse_reply(&reply).ok_or(FAILED)
    }
}

fn stopped() -> bool {
    ask::cancel_requested() || ask::stop_requested()
}

fn parse_reply(raw: &str) -> Option<String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Reply {
        text: String,
    }
    let raw = raw.trim();
    let json = raw
        .strip_prefix("```json")
        .and_then(|body| body.strip_suffix("```"))
        .unwrap_or(raw);
    let reply: Reply = serde_json::from_str(json.trim()).ok()?;
    let text = reply.text.trim();
    (!text.is_empty() && text.chars().count() <= 4000).then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::Path, process::Command};

    const STARTED: &str = "SIDENOTE_MEMORY_FIXTURE_STARTED";

    #[test]
    fn rewrite_fixture_child() {
        let Some(marker) = std::env::var_os(STARTED) else {
            return;
        };
        fs::write(&marker, "started").unwrap();
        let input = std::io::read_to_string(std::io::stdin()).unwrap();
        let prompt: serde_json::Value = serde_json::from_str(&input).unwrap();
        assert_eq!(prompt["comment"], "Use Friday afternoon.");
        assert_eq!(prompt["sources"].as_array().unwrap().len(), 1);
        fs::write(marker, "read stdin").unwrap();
        if std::env::var_os("SIDENOTE_MEMORY_FIXTURE_FAIL").is_some() {
            eprintln!("Private synthetic helper diagnostics");
            std::process::exit(7);
        }
        let reply = serde_json::json!({"text":"The demo is Friday afternoon."});
        eprintln!(
            "{}",
            serde_json::json!({"type":"item.completed","item":{"type":"agent_message","text":reply.to_string()}})
        );
    }

    fn command(parent: &Command, marker: &Path) -> Command {
        assert_eq!(
            fs::read_dir(parent.get_current_dir().unwrap())
                .unwrap()
                .count(),
            0
        );
        let mut child = Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "core::memory::revision::tests::rewrite_fixture_child",
                "--nocapture",
            ])
            .current_dir(parent.get_current_dir().unwrap())
            .env(STARTED, marker);
        child
    }

    fn setup() -> (MemoryStore, RevisionRequest) {
        let store = super::super::undo_tests::fixture();
        let mut config = Config::default();
        config.notes = Some(notes::NotesSettings {
            helper: Some(notes::Helper::Codex),
            ..Default::default()
        });
        fs::write(
            store.root.join("config.json"),
            serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
        let id = store
            .list_pending_proposals()
            .unwrap()
            .remove(0)
            .proposal
            .id;
        let request = store
            .comment_proposal(&id, "Use Friday afternoon.")
            .unwrap()
            .unwrap();
        (store, request)
    }

    #[test]
    fn rewrite_refuses_a_source_hidden_at_child_handoff() {
        let (store, request) = setup();
        let marker = store.root.join("helper-started");
        let child_marker = marker.clone();
        let receipt = store
            .root
            .join("local/recording-guard/1800000000000-1.json");
        let hook_ran = store.root.join("handoff-hook");
        let hook_path = hook_ran.clone();
        runner_process::TEST_CHILD.with(|child| {
            child.replace(Some(Box::new(move |parent| {
                fs::write(&receipt, "unreadable synthetic receipt").unwrap();
                fs::write(&hook_path, "handoff").unwrap();
                command(parent, &child_marker)
            })))
        });
        let result = store.rewrite(&request);
        runner_process::TEST_CHILD.with(|child| child.take());
        assert!(
            hook_ran.is_file(),
            "runner did not reach the spawn boundary"
        );
        assert!(
            !marker.exists(),
            "a child started after the source was hidden"
        );
        assert_eq!(result, Err(FAILED));
    }

    #[test]
    fn rewrite_keeps_helper_diagnostics_private() {
        let (store, request) = setup();
        let marker = store.root.join("helper-started");
        let child_marker = marker.clone();
        runner_process::TEST_CHILD.with(|child| {
            child.replace(Some(Box::new(move |parent| {
                let mut child = command(parent, &child_marker);
                child.env("SIDENOTE_MEMORY_FIXTURE_FAIL", "1");
                child
            })))
        });
        let result = store.rewrite(&request);
        runner_process::TEST_CHILD.with(|child| child.take());
        assert_eq!(fs::read_to_string(marker).unwrap(), "read stdin");
        assert_eq!(result, Err(FAILED));
    }
}
