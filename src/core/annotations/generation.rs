//! Image-only CLI transport and generation ownership. No owner capture is used in tests.
use super::{Capture, media::ApprovedCapture, note};
use crate::core::{
    ask::Cancel,
    notes::Helper,
    runners::{self, Runner},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
pub struct Epoch(Arc<AtomicU64>);
#[derive(Clone)]
pub struct Ticket {
    pub id: u64,
    epoch: Epoch,
    pub cancel: Cancel,
}
impl Epoch {
    pub fn begin(&self) -> Ticket {
        Ticket {
            id: self.0.fetch_add(1, Ordering::SeqCst) + 1,
            epoch: self.clone(),
            cancel: Cancel::new(),
        }
    }
    pub fn invalidate(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl Ticket {
    pub fn current(&self) -> bool {
        self.id == self.epoch.0.load(Ordering::SeqCst) && !self.cancel.stopped()
    }
    pub fn check(&self) -> Result<(), String> {
        if self.current() {
            Ok(())
        } else {
            Err("Stopped. The older job cannot replace this note.".into())
        }
    }
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Options {
    pub helper: Helper,
    pub model: String,
}
pub fn load_options(root: &Path) -> std::io::Result<Option<Options>> {
    match crate::core::file_reads::read(&root.join("annotations/settings.json"), 64 * 1024) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
pub fn save_options(root: &Path, options: &Options) -> std::io::Result<()> {
    if options.model.trim().is_empty()
        || options.model.len() > 256
        || options.model.chars().any(char::is_control)
    {
        return Err(std::io::Error::other(
            "Choose a valid annotation model name.",
        ));
    }
    crate::core::store_io::atomic_write(
        &root.join("annotations/settings.json"),
        &serde_json::to_vec_pretty(options)?,
    )
}
#[derive(Clone, Debug)]
pub enum Progress {
    Frame {
        index: u64,
        draft: Value,
        seconds: f64,
    },
    NoteDelta(String),
    Final {
        note: Value,
        seconds: f64,
    },
}

pub fn available(options: &Options) -> Result<(), String> {
    if options.model.trim().is_empty() {
        return Err("Choose a model in Models and agents.".into());
    }
    if !matches!(options.helper, Helper::Claude | Helper::Codex) {
        return Err(format!(
            "{} cannot receive annotation images in this version. Choose Claude Code or Codex.",
            options.helper.name()
        ));
    }
    executable(options).map(|_| ())
}
fn executable(options: &Options) -> Result<PathBuf, String> {
    runners::executable(&Runner {
        name: options.helper.name().into(),
        program: match options.helper {
            Helper::Claude => "claude",
            Helper::Codex => "codex",
            _ => "opencode",
        }
        .into(),
        args: Vec::new(),
        stdin: true,
        tools: false,
        model: Some(options.model.clone()),
    })
    .map_err(|_| {
        format!(
            "{} is not installed. Open Models and agents to choose an installed agent.",
            options.helper.name()
        )
    })
}
pub fn claude_message(prompt: &str, images: &[PathBuf]) -> Result<String, String> {
    let mut content = vec![json!({"type":"text","text":prompt})];
    for path in images {
        let bytes = crate::core::file_reads::read(path, super::IMAGE_MAX_BYTES)
            .map_err(|_| "Could not read an approved PNG.")?;
        content.push(json!({"type":"text","text":format!("Image: {}",path.file_name().unwrap_or_default().to_string_lossy())}));
        content.push(json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":STANDARD.encode(&bytes)}}));
    }
    let input =
        json!({"type":"user","message":{"role":"user","content":content},"parent_tool_use_id":null})
            .to_string() + "\n";
    if input.len() > 64 * 1024 * 1024 {
        return Err(
            "Approved image input is too large. Leave out some frames and review again.".into(),
        );
    }
    Ok(input)
}
pub fn command_args(
    options: &Options,
    effort: &str,
    images: &[PathBuf],
    final_note: bool,
) -> Vec<String> {
    let mut args: Vec<String> = match options.helper {
        Helper::Claude => [
            "-p",
            "--model",
            &options.model,
            "--effort",
            effort,
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--tools",
            "",
            "--strict-mcp-config",
            "--no-session-persistence",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        Helper::Codex => vec![
            "exec".into(),
            "--model".into(),
            options.model.clone(),
            "--sandbox".into(),
            "read-only".into(),
            "--ephemeral".into(),
            "--skip-git-repo-check".into(),
            "--json".into(),
            "-c".into(),
            format!("model_reasoning_effort=\"{effort}\""),
            "-c".into(),
            "approval_policy=\"never\"".into(),
            "-c".into(),
            "mcp_servers={}".into(),
            "-c".into(),
            "features.shell_tool=false".into(),
            "-c".into(),
            "features.unified_exec=false".into(),
        ],
        _ => Vec::new(),
    };
    if options.helper == Helper::Claude && final_note {
        args.extend(["--json-schema".into(), note::cli_schema()]);
    }
    if options.helper == Helper::Codex {
        for image in images {
            args.extend(["--image".into(), image.to_string_lossy().into_owned()]);
        }
        if final_note {
            args.extend(["--output-schema".into(), "note.schema.json".into()]);
        }
        args.push("-".into());
    }
    args
}
struct Stream {
    text: String,
    result: Option<String>,
    failed: bool,
}
impl Stream {
    fn line(&mut self, line: &str, delta: &impl Fn(String)) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return;
        };
        if v["type"] == "stream_event" {
            let change = &v["event"]["delta"];
            if let Some(s) = change["text"]
                .as_str()
                .or_else(|| change["partial_json"].as_str())
            {
                self.text.push_str(s);
                delta(self.text.clone());
            }
        }
        if v["type"] == "result" {
            self.failed =
                v["is_error"] == true || v["subtype"].as_str().is_some_and(|s| s != "success");
            if v["structured_output"].is_object() {
                self.result = Some(v["structured_output"].to_string());
            } else if let Some(s) = v["result"].as_str() {
                self.result = Some(s.into());
            }
        }
        if v["type"] == "item.completed"
            && v["item"]["type"] == "agent_message"
            && let Some(s) = v["item"]["text"].as_str()
        {
            self.result = Some(s.into());
            self.text = s.into();
            delta(self.text.clone());
        }
        if v["type"] == "turn.failed" || v["type"] == "error" {
            self.failed = true;
        }
    }
}
struct Input<'a> {
    prompt: String,
    images: &'a [PathBuf],
    effort: &'a str,
    final_note: bool,
}
fn invoke(
    options: &Options,
    folder: &Path,
    input: Input<'_>,
    ticket: &Ticket,
    delta: impl Fn(String),
) -> Result<String, String> {
    let Input {
        prompt,
        images,
        effort,
        final_note,
    } = input;
    ticket.check()?;
    let input = if options.helper == Helper::Claude {
        claude_message(&prompt, images)?
    } else {
        prompt
    };
    let mut child = Command::new(executable(options)?)
        .args(command_args(options, effort, images, final_note))
        .current_dir(folder)
        .creation_flags(0x08000000)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "Could not start the annotation agent.")?;
    let job = match crate::platform::ProcessJob::attach(&child) {
        Ok(job) => job,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Could not contain the annotation agent process.".into());
        }
    };
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let stderr = child.stderr.take().unwrap();
    let errors = std::thread::spawn(move || {
        let mut discard = std::io::sink();
        let _ = std::io::copy(&mut stderr.take(4 * 1024 * 1024), &mut discard);
    });
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::sync_channel(32);
    let reader = std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut total = 0;
        loop {
            let mut line = Vec::new();
            let count = reader
                .by_ref()
                .take(4 * 1024 * 1024 + 1)
                .read_until(b'\n', &mut line);
            match count {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    total += n;
                    if n > 4 * 1024 * 1024 || total > 16 * 1024 * 1024 {
                        let _ = tx.send(Err(()));
                        break;
                    }
                    if tx
                        .send(Ok(String::from_utf8_lossy(&line).into_owned()))
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    });
    let start = Instant::now();
    let mut stream = Stream {
        text: String::new(),
        result: None,
        failed: false,
    };
    let mut stop = None;
    let mut eof = false;
    let mut status = None;
    loop {
        if !ticket.current() {
            stop = Some("Stopped. The older job cannot replace this note.");
            break;
        }
        if start.elapsed() > Duration::from_secs(600) {
            stop = Some("The annotation agent timed out. Retry the note.");
            break;
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Ok(line)) => stream.line(&line, &delta),
            Ok(Err(())) => {
                stop = Some("The annotation agent exceeded its output limit.");
                break;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => eof = true,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        match child.try_wait() {
            Ok(s) => status = s,
            Err(_) => {
                stop = Some("Could not read annotation agent status.");
                break;
            }
        }
        if eof && status.is_some() {
            break;
        }
    }
    drop(rx);
    drop(job);
    let _ = child.kill();
    let _ = child.wait();
    let _ = reader.join();
    let _ = errors.join();
    let written = writer.join();
    if let Some(error) = stop {
        return Err(error.into());
    }
    if !status.is_some_and(|s| s.success()) || stream.failed || !matches!(written, Ok(Ok(()))) {
        return Err(format!(
            "{} could not complete the note. Check its sign-in and selected model, then retry.",
            options.helper.name()
        ));
    }
    ticket.check()?;
    stream
        .result
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| "The agent returned no annotation note.".into())
}
pub fn run(
    capture: &Capture,
    approved: &ApprovedCapture,
    options: &Options,
    ticket: &Ticket,
    progress: impl Fn(Progress) + Sync,
) -> Result<Value, String> {
    available(options)?;
    ticket.check()?;
    approved.verify().map_err(|e| e.to_string())?;
    if super::media::source_fingerprint(capture).map_err(|e| e.to_string())?
        != approved.approval.source_fingerprint
    {
        return Err("Capture changed after approval. Review it again.".into());
    }
    crate::core::store_io::atomic_write(
        &approved.folder.join("note.schema.json"),
        note::SCHEMA.as_bytes(),
    )
    .map_err(|_| "Could not save the note schema.")?;
    let started = Instant::now();
    let mut drafts = Vec::new();
    for batch in approved.approval.frames.chunks(2) {
        let results = std::thread::scope(|scope| {
            let handles: Vec<_> = batch
                .iter()
                .map(|frame| {
                    let progress = &progress;
                    scope.spawn(move || {
                        approved.verify().map_err(|e| e.to_string())?;
                        let answer = invoke(
                            options,
                            &approved.folder,
                            Input {
                                prompt: note::frame_prompt(frame),
                                images: &approved.images_for(frame),
                                effort: "low",
                                final_note: false,
                            },
                            ticket,
                            |_| {},
                        )?;
                        let draft = note::parse_json(&answer)?;
                        note::validate_draft(&draft, frame)?;
                        ticket.check()?;
                        progress(Progress::Frame {
                            index: frame.index,
                            draft: draft.clone(),
                            seconds: started.elapsed().as_secs_f64(),
                        });
                        Ok::<_, String>(draft)
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join()
                        .map_err(|_| "A frame worker stopped unexpectedly.".to_owned())
                        .and_then(|r| r)
                })
                .collect::<Vec<_>>()
        });
        for result in results {
            drafts.push(result?);
        }
    }
    ticket.check()?;
    approved.verify().map_err(|e| e.to_string())?;
    let images: Vec<_> = approved
        .approval
        .frames
        .iter()
        .map(|f| approved.folder.join(&f.marked))
        .collect();
    let answer = invoke(
        options,
        &approved.folder,
        Input {
            prompt: note::notebook_prompt(approved, &drafts),
            images: &images,
            effort: "medium",
            final_note: true,
        },
        ticket,
        |text| progress(Progress::NoteDelta(text)),
    )?;
    let value = note::parse_json(&answer)?;
    note::validate(&value, approved)?;
    ticket.check()?;
    approved.verify().map_err(|e| e.to_string())?;
    if super::media::source_fingerprint(capture).map_err(|e| e.to_string())?
        != approved.approval.source_fingerprint
    {
        return Err("Capture changed while the note was being written. Review it again.".into());
    }
    note::save(&approved.folder, &value).map_err(|_| "Could not save the annotation note.")?;
    progress(Progress::Final {
        note: value.clone(),
        seconds: started.elapsed().as_secs_f64(),
    });
    Ok(value)
}
pub fn stream_preview(text: &str) -> String {
    let mut out = Vec::new();
    for key in ["title", "summary"] {
        if let Some(start) = text.find(&format!("\"{key}\"")) {
            let s = &text[start + key.len() + 2..];
            if let Some(colon) = s.find(':') {
                let s = s[colon + 1..].trim_start();
                if let Some(s) = s.strip_prefix('"') {
                    let mut end = s.len();
                    let mut escape = false;
                    for (i, c) in s.char_indices() {
                        if c == '"' && !escape {
                            end = i;
                            break;
                        }
                        escape = c == '\\' && !escape;
                    }
                    let raw = &s[..end];
                    if let Ok(value) = serde_json::from_str::<String>(&format!(
                        "\"{}\"",
                        raw.trim_end_matches('\\')
                    )) {
                        out.push(value);
                    }
                }
            }
        }
    }
    out.join("\n\n")
}

/// Explicitly authorized proof entry point. Its input can only be the invented fixture.
pub fn live_proof(model: &str, root: &Path) -> Result<(), String> {
    let started = Instant::now();
    let capture = super::read_capture(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/annotations/synthetic.scrawl"),
    )
    .map_err(|e| e.to_string())?;
    let review = capture
        .data
        .keyframes
        .iter()
        .enumerate()
        .map(|(i, f)| super::media::ReviewFrame {
            index: f.index,
            included: true,
            masks: if i == 0 {
                vec![[0.0, 0.0, 0.15, 0.10]]
            } else {
                Vec::new()
            },
        })
        .collect::<Vec<_>>();
    let approved = super::media::approve(&capture, &review, root).map_err(|e| e.to_string())?;
    let options = Options {
        helper: Helper::Claude,
        model: model.into(),
    };
    let ticket = Epoch::default().begin();
    let timings = std::sync::Mutex::new((None::<f64>, 0.0));
    let value = run(&capture, &approved, &options, &ticket, |p| match p {
        Progress::Frame { index, seconds, .. } => {
            let mut times = timings.lock().unwrap();
            let elapsed = started.elapsed().as_secs_f64();
            if times.0.is_none() {
                times.0 = Some(elapsed);
            }
            println!("Frame {index} draft ready: {elapsed:.2} s ({seconds:.2} s generation)");
        }
        Progress::Final { seconds, .. } => {
            timings.lock().unwrap().1 = started.elapsed().as_secs_f64();
            println!(
                "Full note validated: {:.2} s ({seconds:.2} s generation)",
                started.elapsed().as_secs_f64()
            );
        }
        Progress::NoteDelta(_) => {}
    })?;
    let times = timings.lock().unwrap();
    let proof = json!({"helper":"Claude Code","model":model,"fixture":"synthetic.scrawl","first_frame_seconds":times.0,"full_note_seconds":times.1,"approved_frames":approved.approval.frames.len(),"approved_folder":approved.folder,"note_title":value["title"],"schema_valid":true,"transport":"stream-json user message with base64 image/png blocks; tools disabled; strict MCP config; no session persistence"});
    crate::core::store_io::atomic_write(
        &root.join("live-timing.json"),
        &serde_json::to_vec_pretty(&proof).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    println!(
        "Synthetic image proof passed. Timing: {}",
        root.join("live-timing.json").display()
    );
    Ok(())
}
