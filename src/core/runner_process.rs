use super::runners::{self, AskProgress, Runner};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Result<Self, String> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "System clock is invalid")?
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "sidenote-{}-{nonce}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).map_err(|_| "Could not create a temporary prompt folder")?;
        Ok(Self(path))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.0.join("prompt.txt"));
        let _ = fs::remove_dir(&self.0);
    }
}
fn safe_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric() || "_:.- ".contains(*c))
        .take(160)
        .collect()
}
fn diagnosis(text: &str) -> &'static str {
    let lower = text.to_lowercase();
    if lower.contains("model")
        && [
            "not found",
            "not supported",
            "unknown",
            "does not exist",
            "unavailable",
        ]
        .iter()
        .any(|s| lower.contains(s))
    {
        "The requested model is not available in this CLI account."
    } else if [
        "unauthorized",
        "authentication",
        "not logged in",
        "login required",
        "authentication_error",
        "please run /login",
        "api key",
        "invalid_api_key",
        "sign in",
        "login",
    ]
    .iter()
    .any(|s| lower.contains(s))
    {
        "The CLI needs a valid sign-in or provider connection."
    } else if lower.contains("permission") || lower.contains("approval") {
        "A CLI permission or approval blocked the request."
    } else if lower.contains("rate limit") || lower.contains("quota") {
        "The provider rate limit or quota blocked the request."
    } else if lower.contains("connection")
        || lower.contains("network")
        || lower.contains("fetch failed")
    {
        "The CLI could not reach its provider or server."
    } else {
        "The CLI did not return a final answer. Check the runner arguments and CLI account."
    }
}
// Only fixed categories and exit codes cross the notes boundary.
#[derive(Debug)]
pub(super) struct RunnerFailure {
    pub category: &'static str,
    pub exit: Option<i32>,
    message: String,
}
impl From<String> for RunnerFailure {
    fn from(message: String) -> Self {
        Self {
            category: if message == "The runner exceeded its time limit." {
                "HELPER_TIMEOUT"
            } else {
                "HELPER_BACKEND"
            },
            exit: None,
            message,
        }
    }
}
impl From<&str> for RunnerFailure {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}
fn category(text: &str) -> &'static str {
    let text = text.to_lowercase();
    if [
        "usage limit",
        "usage_limit",
        "rate limit",
        "rate_limit",
        "quota",
        "insufficient_quota",
        "limit reached",
        "hit your limit",
        "out of extra usage",
        "credit balance",
        "too many requests",
    ]
    .iter()
    .any(|v| text.contains(v))
    {
        "HELPER_LIMIT"
    } else if [
        "unauthorized",
        "authentication",
        "not logged in",
        "api key",
        "invalid_api_key",
        "refresh token",
        "refresh_token",
        "invalid_token",
        "oauth",
        "token has expired",
        "sign in",
        "sign-in",
        "login",
        "token expired",
        "401",
    ]
    .iter()
    .any(|v| text.contains(v))
    {
        "HELPER_AUTH"
    } else {
        "HELPER_BACKEND"
    }
}

enum Stream {
    Line(String),
    TooLarge,
    Done,
}
const STDOUT_CAP: usize = 4 * 1024 * 1024;

fn forward_stdout(stdout: impl Read, sender: mpsc::SyncSender<Stream>) {
    let mut reader = BufReader::new(stdout.take((STDOUT_CAP + 1) as u64));
    let mut total = 0;
    let mut bytes = Vec::new();
    loop {
        bytes.clear();
        let Ok(n) = reader.read_until(b'\n', &mut bytes) else {
            break;
        };
        if n == 0 {
            break;
        }
        total += n;
        if total > STDOUT_CAP {
            let _ = sender.send(Stream::TooLarge);
            return;
        }
        let Ok(line) = String::from_utf8(bytes.clone()) else {
            break;
        };
        if sender
            .send(Stream::Line(line.trim_end_matches(['\r', '\n']).into()))
            .is_err()
        {
            return;
        }
    }
    let _ = sender.send(Stream::Done);
}
#[derive(Default)]
pub(super) struct Output {
    pub(super) final_text: String,
    pub(super) streamed: String,
    pub(super) plain: String,
    pub(super) failed: bool,
    pub(super) tool_seen: std::collections::BTreeSet<String>,
}
impl Output {
    fn text(&mut self, text: String, progress: &impl Fn(AskProgress)) {
        if !text.is_empty() && self.streamed != text {
            self.streamed = text.clone();
            progress(AskProgress::Text { text });
        }
    }
    pub(super) fn line(
        &mut self,
        line: &str,
        harness: &str,
        structured: bool,
        progress: &impl Fn(AskProgress),
    ) {
        if !structured {
            self.plain.push_str(line);
            self.plain.push('\n');
            self.text(self.plain.clone(), progress);
            return;
        }
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            self.plain.push_str(line);
            self.plain.push('\n');
            return;
        };
        let kind = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if harness == "claude" {
            if kind == "stream_event" {
                if event.pointer("/event/type").and_then(|v| v.as_str()) == Some("message_start") {
                    self.final_text.clear();
                }
                if let Some(text) = event.pointer("/event/delta/text").and_then(|v| v.as_str()) {
                    self.final_text.push_str(text);
                    self.text(self.final_text.clone(), progress);
                }
                if let Some(text) = event
                    .pointer("/event/content_block/text")
                    .and_then(|v| v.as_str())
                {
                    self.final_text.push_str(text);
                    self.text(self.final_text.clone(), progress);
                }
            }
            if kind == "assistant"
                && let Some(content) = event.pointer("/message/content").and_then(|v| v.as_array())
            {
                let text: String = content
                    .iter()
                    .filter_map(|b| (b["type"] == "text").then(|| b["text"].as_str()).flatten())
                    .collect();
                if !text.is_empty() {
                    self.final_text = text.clone();
                    self.text(text, progress);
                }
            }
            if kind == "error" {
                self.failed = true;
                self.plain.push_str(line);
            }
            if kind == "result" {
                if let Some(text) = event.get("result").and_then(|v| v.as_str()) {
                    self.final_text = text.into();
                    self.text(text.into(), progress);
                }
                self.failed = event
                    .get("is_error")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if self.failed {
                    self.plain.push_str(line);
                }
            }
            if let Some(content) = event.pointer("/message/content").and_then(|v| v.as_array()) {
                for block in content {
                    if block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                        let id = block
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if self.tool_seen.insert(id)
                            && let Some(name) = block.get("name").and_then(|v| v.as_str())
                        {
                            progress(AskProgress::Tool {
                                name: safe_name(name),
                            });
                        }
                    }
                }
            }
        } else if harness == "codex" {
            if kind == "turn.failed" || kind == "error" {
                self.failed = true;
                self.plain.push_str(line);
            }
            if let Some(item) = event.get("item") {
                let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if matches!(kind, "item.updated" | "item.completed")
                    && item_type == "agent_message"
                    && let Some(text) = item.get("text").and_then(|v| v.as_str())
                {
                    self.final_text = text.into();
                    self.text(text.into(), progress);
                }
                if kind == "item.started" || kind == "item.completed" {
                    let name = match item_type {
                        "command_execution" => Some("Command"),
                        "mcp_tool_call" => item.get("tool").and_then(|v| v.as_str()),
                        "web_search" => Some("Web search"),
                        "file_change" => Some("File edit"),
                        _ => None,
                    };
                    if let Some(name) = name {
                        let id = item.get("id").and_then(|v| v.as_str()).unwrap_or(name);
                        if self.tool_seen.insert(id.into()) {
                            progress(AskProgress::Tool {
                                name: safe_name(name),
                            });
                        }
                    }
                }
            }
        } else {
            if kind == "error" {
                self.failed = true;
                self.plain.push_str(line);
            }
            if kind == "text"
                && let Some(text) = event
                    .pointer("/part/text")
                    .and_then(|v| v.as_str())
                    .or_else(|| event.get("text").and_then(|v| v.as_str()))
            {
                self.final_text.push_str(text);
                self.text(self.final_text.clone(), progress);
            }
            if let Some(part) = event.get("part")
                && let Some(tool) = part.get("tool").and_then(|v| v.as_str())
            {
                let id = part
                    .get("callID")
                    .or_else(|| part.get("id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(tool);
                if self.tool_seen.insert(id.into()) {
                    progress(AskProgress::Tool {
                        name: safe_name(tool),
                    });
                }
            }
        }
    }
}
/// Runs one configured CLI. The callback receives status and tool names, never tool inputs.
/// Call from a worker; `ask::spawn` supplies one for the normal meeting path.
pub fn run_runner(
    runner: &Runner,
    prompt: String,
    working_dir: &Path,
    progress: impl Fn(AskProgress),
) -> Result<String, String> {
    run_runner_with_timeout(
        runner,
        prompt,
        working_dir,
        Duration::from_secs(600),
        progress,
    )
}

pub(super) fn run_runner_with_timeout(
    runner: &Runner,
    prompt: String,
    working_dir: &Path,
    timeout: Duration,
    progress: impl Fn(AskProgress),
) -> Result<String, String> {
    run_classified(runner, prompt, working_dir, timeout, progress).map_err(|e| e.message)
}

pub(super) fn run_classified(
    runner: &Runner,
    prompt: String,
    working_dir: &Path,
    timeout: Duration,
    progress: impl Fn(AskProgress),
) -> Result<String, RunnerFailure> {
    runners::validate(runner)?;
    #[cfg(target_os = "linux")]
    if cfg!(target_os = "linux") {
        return Err(RunnerFailure {
            category: "HELPER_UNAVAILABLE_LINUX",
            exit: None,
            message: super::notes::LINUX_HELPER_UNAVAILABLE.into(),
        });
    }
    let _update_activity = crate::updates::activity()
        .map_err(|_| "An update is being installed. Try again after restart.")?;
    let harness = runners::kind(runner)?;
    let program = runners::executable(runner)?;
    let scratch = if runner.args.iter().any(|a| a.contains("{prompt_file}")) {
        Some(Scratch::new()?)
    } else {
        None
    };
    let file = if let Some(s) = &scratch {
        let file = s.0.join("prompt.txt");
        fs::write(&file, prompt.as_bytes()).map_err(|_| "Could not write the temporary prompt")?;
        Some(file)
    } else {
        None
    };
    let args: Vec<String> = runner
        .args
        .iter()
        .map(|a| {
            a.replace("{model}", runner.model.as_deref().unwrap_or(""))
                .replace(
                    "{prompt_file}",
                    &file
                        .as_ref()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                )
        })
        .collect();
    let pair = |flag: &str, value: &str| args.windows(2).any(|a| a[0] == flag && a[1] == value);
    let structured = pair("--output-format", "json")
        || pair("--output-format", "stream-json")
        || args.iter().any(|a| a == "--json")
        || pair("--format", "json");
    let started = Instant::now();
    progress(AskProgress::Started {
        runner: runner.name.clone(),
    });
    let mut child = crate::platform::UserRunner::spawn(
        std::process::Command::new(program)
            .args(&args)
            .current_dir(working_dir),
    )
    .map_err(|e| {
        format!(
            "Could not start {} (OS error {})",
            runner.name,
            e.raw_os_error().unwrap_or(0)
        )
    })?;
    if !runner.stdin {
        drop(child.stdin.take());
    }
    let stdout = child
        .stdout
        .take()
        .ok_or("Could not capture the CLI output")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("Could not capture the CLI status")?;
    let writer = child
        .stdin
        .take()
        .map(|mut stdin| std::thread::spawn(move || stdin.write_all(prompt.as_bytes())));
    let errors = std::thread::spawn(move || {
        let mut stderr = stderr;
        let mut bytes = Vec::new();
        let mut chunk = [0; 4096];
        while let Ok(n) = stderr.read(&mut chunk) {
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..n]);
            if bytes.len() > 65536 {
                bytes.drain(..bytes.len() - 65536);
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    });
    let (sender, receiver) = mpsc::sync_channel(8);
    let reader = std::thread::spawn(move || forward_stdout(stdout, sender));
    let mut output = Output {
        final_text: String::new(),
        streamed: String::new(),
        plain: String::new(),
        failed: false,
        tool_seen: Default::default(),
    };
    let mut done = false;
    let mut status = None;
    let mut too_large = false;
    loop {
        let stop = if too_large {
            Some("The runner exceeded its output limit.")
        } else if super::ask::stop_requested() {
            Some("Stopped because the window was closed.")
        } else if crate::service::stopping() || super::ask::cancel_requested() {
            Some(super::ask::STOPPED)
        } else if started.elapsed() >= timeout {
            Some("The runner exceeded its time limit.")
        } else {
            None
        };
        if let Some(message) = stop {
            drop(receiver);
            let _ = child.stop();
            let _ = reader.join();
            let _ = errors.join();
            if let Some(writer) = writer {
                let _ = writer.join();
            }
            return Err(message.into());
        }
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(Stream::Line(line)) => output.line(&line, harness, structured, &progress),
            Ok(Stream::TooLarge) => too_large = true,
            Ok(Stream::Done) | Err(mpsc::RecvTimeoutError::Disconnected) => done = true,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if status.is_none() {
            status = child
                .try_wait()
                .map_err(|_| "Could not read the CLI status")?;
        }
        if status.is_some() && done {
            break;
        }
    }
    child.stop().map_err(|_| "Could not stop the CLI process")?;
    let _ = reader.join();
    let error_text = errors.join().unwrap_or_default();
    let answer = if structured {
        output.final_text.trim()
    } else {
        output.plain.trim()
    };
    if !status.is_some_and(|s| s.success()) || output.failed || answer.is_empty() {
        let detail = format!("{error_text}\n{}\n{}", output.plain, output.final_text);
        return Err(RunnerFailure {
            category: category(&detail),
            exit: status.and_then(|s| s.code()),
            message: format!("{}: {}", runner.name, diagnosis(&detail)),
        });
    }
    if let Some(writer) = writer {
        writer
            .join()
            .map_err(|_| "Could not send the prompt to the CLI")?
            .map_err(|_| "Could not send the prompt to the CLI")?;
    }
    Ok(answer.into())
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_agent_runners_fail_before_executable_lookup_or_launch() {
        let root = tempfile::tempdir().unwrap();
        for (name, args) in [
            ("claude", vec!["-p"]),
            ("codex", vec!["exec", "--sandbox", "read-only"]),
            ("opencode", vec!["run"]),
            ("opencode2", vec!["run"]),
        ] {
            let runner = Runner {
                name: name.into(),
                program: root.path().join(name).to_string_lossy().into_owned(),
                args: args.into_iter().map(str::to_owned).collect(),
                stdin: true,
                tools: true,
                model: None,
            };
            assert_eq!(
                run_runner_with_timeout(
                    &runner,
                    "synthetic".into(),
                    root.path(),
                    Duration::from_secs(1),
                    |_| {}
                )
                .unwrap_err(),
                super::super::notes::LINUX_HELPER_UNAVAILABLE
            );
        }
        assert_eq!(
            super::super::notes::test_helper(super::super::notes::Helper::Claude, "synthetic")
                .unwrap_err(),
            super::super::notes::LINUX_HELPER_UNAVAILABLE
        );
    }
    use super::*;
    #[test]
    fn partial_answers_arrive_before_results_without_duplicating_text_or_leaking_tools() {
        let mut output = Output {
            final_text: String::new(),
            streamed: String::new(),
            plain: String::new(),
            failed: false,
            tool_seen: Default::default(),
        };
        let events = std::cell::RefCell::new(Vec::new());
        let progress = |event| events.borrow_mut().push(event);
        for event in [
            serde_json::json!({"type":"stream_event","event":{"type":"message_start"}}),
            serde_json::json!({"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"private reasoning"}}}),
            serde_json::json!({"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hello "}}}),
            serde_json::json!({"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"世界"}}}),
        ] {
            output.line(&event.to_string(), "claude", true, &progress);
        }
        assert_eq!(
            *events.borrow(),
            [
                AskProgress::Text {
                    text: "Hello ".into()
                },
                AskProgress::Text {
                    text: "Hello 世界".into()
                }
            ]
        );
        output.line(r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hello 世界"},{"type":"tool_use","id":"one","name":"Read","input":{"secret":"tool input"}}]}}"#, "claude", true, &progress);
        output.line(
            r#"{"type":"result","result":"Hello 世界","is_error":false}"#,
            "claude",
            true,
            &progress,
        );
        assert_eq!(output.final_text, "Hello 世界");
        assert_eq!(events.borrow().len(), 3);
        assert_eq!(
            events.borrow()[2],
            AskProgress::Tool {
                name: "Read".into()
            }
        );
        output.line(r#"{"type":"item.completed","item":{"id":"two","type":"agent_message","text":"Codex answer"}}"#, "codex", true, &progress);
        assert_eq!(
            events.borrow().last(),
            Some(&AskProgress::Text {
                text: "Codex answer".into()
            })
        );
        output.final_text.clear();
        output.line(
            r#"{"type":"text","part":{"text":"Open"}}"#,
            "opencode",
            true,
            &progress,
        );
        output.line(
            r#"{"type":"text","part":{"text":"Code"}}"#,
            "opencode",
            true,
            &progress,
        );
        assert_eq!(output.final_text, "OpenCode");
        assert_eq!(
            events.borrow().last(),
            Some(&AskProgress::Text {
                text: "OpenCode".into()
            })
        );
    }
    fn capture(bytes: Vec<u8>) -> (usize, bool) {
        let (sender, receiver) = mpsc::sync_channel(8);
        let reader =
            std::thread::spawn(move || forward_stdout(std::io::Cursor::new(bytes), sender));
        let mut total = 0;
        let mut limited = false;
        for event in receiver {
            match event {
                Stream::Line(line) => total += line.len(),
                Stream::TooLarge => limited = true,
                Stream::Done => {}
            }
        }
        reader.join().unwrap();
        (total, limited)
    }
    #[test]
    fn cli_failures_have_fixed_categories_and_ignore_private_details() {
        for text in [
            "You've hit your limit. PRIVATE",
            "usage_limit_reached",
            "rate limit exceeded",
            "quota exhausted",
        ] {
            assert_eq!(category(text), "HELPER_LIMIT");
        }
        for text in [
            "401 Unauthorized PRIVATE",
            "authentication failed",
            "not logged in",
        ] {
            assert_eq!(category(text), "HELPER_AUTH");
        }
        assert_eq!(category("unknown error PRIVATE"), "HELPER_BACKEND");
        let mut output = Output::default();
        output.line(
            r#"{"type":"result","result":"You've hit your limit","is_error":true}"#,
            "claude",
            true,
            &|_| {},
        );
        assert!(output.failed);
        assert_eq!(category(&output.final_text), "HELPER_LIMIT");
        output.line(
            r#"{"type":"turn.failed","error":{"message":"usage_limit_reached"}}"#,
            "codex",
            true,
            &|_| {},
        );
        assert!(output.failed);
        assert_eq!(category(&output.plain), "HELPER_LIMIT");
        for harness in ["claude", "codex"] {
            for diagnostic in [
                "invalid_api_key",
                "authentication_error",
                "login required",
                "please run /login",
            ] {
                let mut failed = Output::default();
                failed.line(
                    &serde_json::json!({"type":"error", "message":diagnostic}).to_string(),
                    harness,
                    true,
                    &|_| {},
                );
                assert!(failed.failed);
                assert_eq!(category(&failed.plain), "HELPER_AUTH");
            }
        }
        assert_eq!(category("insufficient_quota"), "HELPER_LIMIT");
        let mut plain = Output::default();
        plain.line("Invalid API key PRIVATE", "claude", true, &|_| {});
        assert_eq!(category(&plain.plain), "HELPER_AUTH");
    }
    #[test]
    fn stdout_caps_unterminated_lines_and_streams_without_running_a_cli() {
        assert_eq!(capture(vec![b'x'; STDOUT_CAP]), (STDOUT_CAP, false));
        assert_eq!(capture(vec![b'x'; STDOUT_CAP + 1]), (0, true));
        let (total, limited) = capture(
            format!("{}\n", "x".repeat(1023))
                .repeat(STDOUT_CAP / 1024 + 1)
                .into_bytes(),
        );
        assert!(limited);
        assert!(total <= STDOUT_CAP);
    }
    #[test]
    fn cancelling_bounded_stdout_does_not_leave_the_reader_blocked() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let reader = std::thread::spawn(move || {
            forward_stdout(std::io::Cursor::new(b"x\n".repeat(100)), sender)
        });
        drop(receiver);
        reader.join().unwrap();
    }
}
