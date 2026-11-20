use super::Helper;
use crate::core::{
    runner_process,
    runners::{self, Runner},
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
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

pub(super) use crate::core::meeting_job::Failure;

/// The test seam never launches a CLI or inspects the host's helper installation.
pub(super) trait Run {
    fn translate(
        &mut self,
        _snapshot: &super::translation::Snapshot,
        _prompt: String,
        _end: Instant,
    ) -> Result<String, &'static str> {
        Err("helper_not_text_only")
    }
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
    fn translate(
        &mut self,
        snapshot: &super::translation::Snapshot,
        prompt: String,
        end: Instant,
    ) -> Result<String, &'static str> {
        let root = self.root.as_ref().ok_or("translation_consent_changed")?;
        if !crate::core::Config::load_existing(root).is_ok_and(|c| snapshot.authorized_by(&c)) {
            return Err("translation_consent_changed");
        }
        if !matches!(snapshot.helper, Helper::OpenCode | Helper::Claude) {
            return Err("helper_not_text_only");
        }
        let scratch = Scratch::new().map_err(|_| "translation_storage_failed")?;
        let authorize =
            || crate::core::Config::load_existing(root).is_ok_and(|c| snapshot.authorized_by(&c));
        let claude =
            (snapshot.helper == Helper::Claude).then(|| runner(Helper::Claude, &snapshot.model));
        let invoke = |capability, input| {
            runner_process::run_classified(
                match &claude {
                    Some(runner) => {
                        runner_process::Invocation::ClaudeTranslation(runner, &authorize)
                    }
                    None => runner_process::Invocation::Translation {
                        model: &snapshot.model,
                        capability,
                        location: &snapshot.location,
                        payment: &snapshot.payment,
                        authorize: &authorize,
                    },
                },
                input,
                &scratch.0,
                end.checked_duration_since(Instant::now())
                    .filter(|left| !left.is_zero())
                    .ok_or("translation_timed_out")?,
                false,
                |_| {},
            )
            .map_err(|e| match e.category {
                "HELPER_TOOL_USE" => "helper_tool_use",
                "HELPER_TIMEOUT" => "translation_timed_out",
                "HELPER_AUTH" => "helper_auth",
                "HELPER_LIMIT" => "helper_limit",
                "HELPER_UNAVAILABLE_LINUX" => "helper_unavailable_linux",
                "HELPER_UNQUALIFIED" => "helper_unqualified",
                "HELPER_PAYMENT" => "translation_payment_route_unavailable",
                "HELPER_CONSENT" => "translation_consent_changed",
                _ => "helper_unsupported",
            })
        };
        if claude.is_some() {
            return invoke(None, prompt);
        }
        let capability = invoke(
            None,
            serde_json::json!({"model":snapshot.model}).to_string(),
        )?;
        invoke(Some(capability.as_str()), prompt)
    }
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
            return Err(Failure::LinuxUnavailable);
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
        runner_process::run_classified(
            runner_process::Invocation::General(&runner(helper, model)),
            prompt,
            &scratch.0,
            timeout,
            false,
            |_| {},
        )
        .map_err(|error| {
            if error.category == "HELPER_TIMEOUT" {
                Failure::TimedOut
            } else if error.category == "HELPER_UNAVAILABLE_LINUX" {
                Failure::LinuxUnavailable
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
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_helper_stops_with_typed_unavailable_before_lookup_or_launch() {
        assert_eq!(
            Cli { root: None }.run(Helper::Codex, "", "synthetic".into(), Duration::ZERO),
            Err(Failure::LinuxUnavailable)
        );
    }
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
