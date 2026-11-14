use super::Helper;
use crate::core::{
    runner_process,
    runners::{self, Runner},
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

pub(super) const NOTES_TIMEOUT: Duration = Duration::from_secs(15 * 60);
pub(super) const TASKS_TIMEOUT: Duration = Duration::from_secs(5 * 60);
pub(super) const TITLE_TIMEOUT: Duration = Duration::from_secs(60);

impl Helper {
    /// Detection is independent of whether this release permits a helper to run.
    pub fn offered(self) -> bool {
        matches!(self, Self::Claude | Self::Codex)
    }
    pub fn models(self) -> Vec<crate::core::ask_session::Model> {
        crate::core::ask_session::models(&runner(self, ""))
    }
    pub fn default_model(self) -> &'static str {
        match self {
            Self::Claude => "claude-sonnet-5-5",
            Self::Codex => "gpt-6.1-terra",
            Self::OpenCode => "",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
            Self::OpenCode => "OpenCode",
        }
    }
}

pub(super) fn runner(helper: Helper, model: &str) -> Runner {
    let (program, args, tools): (&str, Vec<&str>, bool) = match helper {
        Helper::Claude => (
            "claude",
            vec![
                "-p",
                "--model",
                "{model}",
                "--strict-mcp-config",
                "--tools",
                "",
                "--no-session-persistence",
                "--output-format",
                "json",
            ],
            false,
        ),
        Helper::Codex => (
            "codex",
            vec![
                "exec",
                "--model",
                "{model}",
                "--sandbox",
                "read-only",
                "-c",
                "approval_policy=\"never\"",
                "--ephemeral",
                "--skip-git-repo-check",
                "--json",
                "-",
            ],
            true,
        ),
        Helper::OpenCode => (
            "opencode",
            vec!["run", "--format", "json", "--agent", "plan"],
            true,
        ),
    };
    Runner {
        name: helper.name().into(),
        program: program.into(),
        args: args.into_iter().map(str::to_owned).collect(),
        stdin: true,
        tools,
        model: Some(
            if model.trim().is_empty() {
                helper.default_model()
            } else {
                model
            }
            .into(),
        ),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Failure {
    Busy,
    #[cfg(target_os = "linux")]
    NetworkUnavailable,
    Consent,
    Missing,
    TimedOut,
    Backend {
        exit: Option<i32>,
    },
    BadOutput,
    Limit {
        exit: Option<i32>,
    },
    Auth {
        exit: Option<i32>,
    },
}

/// The test seam never launches a CLI or inspects the host's helper installation.
pub(super) trait Run {
    fn installed(&self, helper: Helper) -> bool;
    fn glossary(&self) -> Vec<String> {
        Vec::new()
    }
    fn run(
        &mut self,
        helper: Helper,
        model: &str,
        prompt: String,
        timeout: Duration,
    ) -> Result<String, Failure>;
}
pub(super) struct Cli {
    pub root: Option<PathBuf>,
}
pub(super) fn authorized(root: &std::path::Path, helper: Helper) -> Result<bool, Failure> {
    if !helper.offered() {
        return Ok(false);
    }
    Ok(crate::core::Config::load_in(root)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::WouldBlock {
                Failure::Busy
            } else {
                Failure::Backend { exit: None }
            }
        })?
        .notes
        .is_some_and(|s| {
            s.engine() == super::Engine::App
                && s.helper == Some(helper)
                && s.enabled_at_ms.is_some()
        }))
}
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> std::io::Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "sidenote-notes-{}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
impl Run for Cli {
    fn installed(&self, helper: Helper) -> bool {
        runners::installed(&runner(helper, ""))
    }
    fn glossary(&self) -> Vec<String> {
        crate::hex_engine::HexEngine::new()
            .vocabulary_list()
            .unwrap_or_default()
            .into_iter()
            .take(1000)
            .map(|entry| entry.term.chars().take(200).collect())
            .collect()
    }
    fn run(
        &mut self,
        helper: Helper,
        model: &str,
        prompt: String,
        timeout: Duration,
    ) -> Result<String, Failure> {
        if !helper.offered() {
            return Err(Failure::Consent);
        }
        #[cfg(target_os = "linux")]
        if cfg!(target_os = "linux") {
            return Err(Failure::NetworkUnavailable);
        }
        if let Some(root) = &self.root
            && !authorized(root, helper)?
        {
            return Err(Failure::Consent);
        }
        if !self.installed(helper) {
            return Err(Failure::Missing);
        }
        let scratch = Scratch::new().map_err(|_| Failure::Backend { exit: None })?;
        runner_process::run_classified(&runner(helper, model), prompt, &scratch.0, timeout, |_| {})
            .map_err(|error| {
                if error.category == "HELPER_TIMEOUT" {
                    Failure::TimedOut
                } else if error.category == "HELPER_LIMIT" {
                    Failure::Limit { exit: error.exit }
                } else if error.category == "HELPER_AUTH" {
                    Failure::Auth { exit: error.exit }
                } else {
                    Failure::Backend { exit: error.exit }
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locked_commands_validate_and_never_load_runner_settings() {
        for helper in [Helper::Claude, Helper::Codex, Helper::OpenCode] {
            let r = runner(helper, "test-model");
            runners::validate(&r).unwrap();
            assert!(r.stdin);
            assert!(!r.args.iter().any(|a| a.contains("prompt_file")));
            if helper == Helper::OpenCode {
                assert!(!r.args.iter().any(|a| a == "--model"));
            }
        }
        let claude = runner(Helper::Claude, "");
        assert!(!claude.tools);
        assert!(claude.args.windows(2).any(|a| a == ["--tools", ""]));
        assert!(
            runner(Helper::Codex, "")
                .args
                .windows(2)
                .any(|a| a == ["--sandbox", "read-only"])
        );
    }
    #[test]
    fn each_call_gets_an_empty_distinct_directory() {
        let a = Scratch::new().unwrap();
        let b = Scratch::new().unwrap();
        assert_ne!(a.0, b.0);
        assert_eq!(fs::read_dir(&a.0).unwrap().count(), 0);
        let path = a.0.clone();
        drop(a);
        assert!(!path.exists());
    }
}
