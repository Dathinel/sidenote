use super::runners::{self, AskProgress, Runner};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
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
        "api key",
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
enum Stream {
    Line(String),
    Done,
}
struct Output {
    final_text: String,
    plain: String,
    failed: bool,
    tool_seen: std::collections::BTreeSet<String>,
}
impl Output {
    fn line(
        &mut self,
        line: &str,
        harness: &str,
        structured: bool,
        progress: &impl Fn(AskProgress),
    ) {
        if !structured {
            self.plain.push_str(line);
            self.plain.push('\n');
            return;
        }
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            return;
        };
        let kind = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if harness == "claude" {
            if kind == "result" {
                if let Some(text) = event.get("result").and_then(|v| v.as_str()) {
                    self.final_text = text.into();
                }
                self.failed = event
                    .get("is_error")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
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
                if kind == "item.completed"
                    && item_type == "agent_message"
                    && let Some(text) = item.get("text").and_then(|v| v.as_str())
                {
                    self.final_text = text.into();
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
    runners::validate(runner)?;
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
    let structured = pair("--output-format", "stream-json")
        || args.iter().any(|a| a == "--json")
        || pair("--format", "json");
    let started = Instant::now();
    progress(AskProgress::Started {
        runner: runner.name.clone(),
    });
    let mut child = Command::new(program)
        .args(&args)
        .current_dir(working_dir)
        .creation_flags(0x08000000)
        .stdin(if runner.stdin {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            format!(
                "Could not start {} (OS error {})",
                runner.name,
                e.raw_os_error().unwrap_or(0)
            )
        })?;
    let job = match crate::platform::ProcessJob::attach(&child) {
        Ok(job) => job,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Could not contain the CLI process for timeout cleanup".into());
        }
    };
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
    let (sender, receiver) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(line) => {
                    if sender.send(Stream::Line(line)).is_err() {
                        return;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = sender.send(Stream::Done);
    });
    let mut output = Output {
        final_text: String::new(),
        plain: String::new(),
        failed: false,
        tool_seen: Default::default(),
    };
    let mut done = false;
    let mut status = None;
    loop {
        let stop = if super::ask::stop_requested() {
            Some("Stopped because the window was closed.")
        } else if super::ask::cancel_requested() {
            Some(super::ask::STOPPED)
        } else if started.elapsed() >= timeout {
            Some("The runner exceeded its time limit.")
        } else {
            None
        };
        if let Some(message) = stop {
            drop(job);
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            let _ = errors.join();
            if let Some(writer) = writer {
                let _ = writer.join();
            }
            return Err(message.into());
        }
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(Stream::Line(line)) => output.line(&line, harness, structured, &progress),
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
    drop(job);
    let _ = reader.join();
    let error_text = errors.join().unwrap_or_default();
    if let Some(writer) = writer {
        writer
            .join()
            .map_err(|_| "Could not send the prompt to the CLI")?
            .map_err(|_| "Could not send the prompt to the CLI")?;
    }
    let answer = if structured {
        output.final_text.trim()
    } else {
        output.plain.trim()
    };
    if !status.is_some_and(|s| s.success()) || output.failed || answer.is_empty() {
        return Err(format!(
            "{}: {}",
            runner.name,
            diagnosis(&format!(
                "{error_text}\n{}\n{}",
                output.plain, output.final_text
            ))
        ));
    }
    Ok(answer.into())
}
