use crate::core::failure::{AtStage, Stage};
use serde::Deserialize;
use std::{
    fs::{self, OpenOptions},
    io::{self, BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    os::windows::process::CommandExt,
    path::PathBuf,
    process::Command,
    time::Duration,
};

const HIDDEN: u32 = 0x08000000;
pub mod status;
mod timings;
#[derive(Deserialize)]
struct Discovery {
    port: u16,
    token: String,
}
enum Problem {
    Http(u16),
    Io(io::ErrorKind),
    Value,
}
impl From<io::Error> for Problem {
    fn from(e: io::Error) -> Self {
        Self::Io(e.kind())
    }
}
impl Problem {
    fn message(&self) -> String {
        match self {
            Self::Http(404) => "Labs > Meeting finished event is off".into(),
            Self::Http(409) => "another app is already listening".into(),
            Self::Http(code) => format!("HTTP {code}"),
            Self::Value => "Hex not reachable (JSONDecodeError)".into(),
            Self::Io(kind) => format!(
                "Hex not reachable ({})",
                match kind {
                    io::ErrorKind::NotFound => "FileNotFoundError",
                    io::ErrorKind::PermissionDenied => "PermissionError",
                    io::ErrorKind::ConnectionRefused => "URLError",
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => "TimeoutError",
                    _ => "OSError",
                }
            ),
        }
    }
}
pub(crate) fn log(message: &str) {
    log_at(&crate::core::log_path(), message);
}
fn log_at(path: &std::path::Path, message: &str) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(
            file,
            "{} listener: {message}",
            chrono::Local::now().format("%Y-%m-%dT%H:%M:%S")
        );
    }
}
pub(crate) fn pipeline(meeting_id: Option<&str>) -> io::Result<()> {
    pipeline_in(
        &crate::core::state_dir(),
        &crate::core::meetings_dir(),
        meeting_id,
    )
}
fn pipeline_in(
    state: &std::path::Path,
    meetings: &std::path::Path,
    meeting_id: Option<&str>,
) -> io::Result<()> {
    if let Some(id) = meeting_id
        && crate::core::recording_guard::eligibility(state, meetings, id)
            .at(Stage::CatchUpEligibility)?
            != crate::core::recording_guard::Eligibility::Eligible
    {
        return Ok(());
    }
    let activity = crate::updates::activity().at(Stage::CatchUpActivity)?;
    if let Some(id) = meeting_id
        && crate::core::meeting_router::jobs::configured()
    {
        if crate::core::meeting_router::jobs::enqueue(id).is_err() {
            log("could not queue meeting routing; existing pipeline continues");
        } else if crate::core::meeting_router::jobs::start_worker().is_err() {
            log("routing remains queued for the next drain");
        }
    }
    let settings = crate::core::notes::settings().at(Stage::CatchUpSettings)?;
    let script = match settings.engine() {
        crate::core::notes::Engine::App => {
            if let Some(id) = meeting_id {
                crate::core::notes::queue::enqueue(id).at(Stage::CatchUpEnqueue)?;
                crate::core::notes::queue::start_worker()?;
            } else {
                crate::core::notes::queue::catch_up()?;
            }
            return Ok(());
        }
        crate::core::notes::Engine::Scripts(script) => {
            if let Some(id) = meeting_id {
                crate::core::notes::draft::start(
                    state.to_owned(),
                    meetings.to_owned(),
                    id.to_owned(),
                    settings,
                );
            }
            script
        }
        crate::core::notes::Engine::Off => return Ok(()),
    };
    if !script.is_file() {
        log(crate::core::live::NOTES_UNAVAILABLE);
        return Ok(());
    }
    let mut folders = script_folders(state, meetings, meeting_id).at(Stage::CatchUpMeetings)?;
    let script_lock = match crate::core::store_io::lock(
        &crate::core::state_dir().join("local/notes-script.lock"),
    ) {
        Ok(lock) => lock,
        Err(e) if matches!(e.raw_os_error(), Some(32 | 33)) => return Ok(()),
        Err(e) => return Err(Stage::CatchUpScriptLock.error(e)),
    };
    let (mut command, filtered) =
        script_command(&script, &folders, log).at(Stage::CatchUpScriptCommand)?;
    if filtered && folders.is_empty() {
        return Ok(());
    }
    if !filtered {
        folders = legacy_script_folders(meetings, meeting_id).at(Stage::CatchUpMeetings)?;
    }
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let child = command.spawn().at(Stage::CatchUpScriptCommand);
    std::thread::spawn(move || {
        let _script_lock = script_lock;
        let _activity = activity;
        let result = script_result(child.and_then(|mut child| {
            if let Some(stdout) = child.stdout.take() {
                timings::capture(stdout, &folders, |message| log(&message));
            }
            child.wait().at(Stage::CatchUpScriptCommand)
        }));
        let succeeded = result.is_ok();
        for folder in &folders {
            if let Some(id) = folder.file_name().and_then(|n| n.to_str()) {
                let _ = crate::core::notes::draft::read(&crate::core::state_dir(), folder, id);
            }
        }
        script_markers(&folders, succeeded);
        if let Err(error) = result {
            log(&format!("catch-up: {error}"));
        }
    });
    Ok(())
}
fn script_result(result: io::Result<std::process::ExitStatus>) -> io::Result<()> {
    let status = result.at(Stage::CatchUpScriptCommand)?;
    if status.success() {
        Ok(())
    } else {
        Err(Stage::CatchUpScriptCommand.error(io::Error::other("Script exited unsuccessfully")))
    }
}

fn script_folders(
    state: &std::path::Path,
    meetings: &std::path::Path,
    meeting_id: Option<&str>,
) -> io::Result<Vec<PathBuf>> {
    Ok(fs::read_dir(meetings)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|folder| {
            folder
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|id| {
                    meeting_id.is_none_or(|requested| requested == id)
                        && crate::core::recording_guard::eligibility(state, meetings, id)
                            .is_ok_and(|e| e == crate::core::recording_guard::Eligibility::Eligible)
                })
                && !folder.join("english-summary.json").is_file()
        })
        .collect())
}
fn catch_up_routing(
    state: &std::path::Path,
    meetings: &std::path::Path,
    mut enqueue: impl FnMut(&str) -> io::Result<()>,
) -> io::Result<()> {
    for entry in fs::read_dir(meetings).at(Stage::CatchUpMeetings)? {
        let entry = entry.at(Stage::CatchUpMeetings)?;
        let id = entry.file_name().to_string_lossy().into_owned();
        if !id.is_empty()
            && id.bytes().any(|b| b.is_ascii_digit())
            && id.bytes().all(|b| b.is_ascii_digit() || b == b'-')
            && crate::core::recording_guard::eligibility(state, meetings, &id)
                .at(Stage::CatchUpEligibility)?
                == crate::core::recording_guard::Eligibility::Eligible
        {
            enqueue(&id).at(Stage::CatchUpRouting)?;
        }
    }
    Ok(())
}
const LEGACY_SCRIPT_NOTICE: &str =
    "notes script does not accept MeetingFolders; short-recording filter not applied to the script";
fn legacy_script_folders(
    meetings: &std::path::Path,
    meeting_id: Option<&str>,
) -> io::Result<Vec<PathBuf>> {
    Ok(fs::read_dir(meetings)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|folder| {
            !folder.join("english-summary.json").is_file()
                && (meeting_id.is_some_and(|id| folder.file_name().is_some_and(|n| n == id))
                    || fs::read(folder.join("manifest.json"))
                        .ok()
                        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                        .is_some_and(|m| m["status"] == "complete"))
        })
        .collect())
}
fn script_markers(folders: &[PathBuf], succeeded: bool) {
    for folder in folders {
        if !succeeded {
            let _ = fs::write(folder.join("notes-failed"), b"failed");
        } else {
            let _ = fs::remove_file(folder.join("notes-failed"));
        }
    }
}
fn script_command(
    script: &std::path::Path,
    folders: &[PathBuf],
    report: impl FnOnce(&str),
) -> io::Result<(Command, bool)> {
    let filtered = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-NonInteractive", "-Command",
            "$ErrorActionPreference='Stop'; if ((Get-Command -Name $env:SIDENOTE_NOTES_SCRIPT).Parameters.ContainsKey('MeetingFolders')) { exit 0 }; exit 1"])
        .env("SIDENOTE_NOTES_SCRIPT", script).creation_flags(HIDDEN)
        .output().is_ok_and(|output| output.status.success());
    let mut command = Command::new("powershell.exe");
    if filtered {
        command.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-NonInteractive", "-Command",
            "$ErrorActionPreference='Stop'; $folders=@(ConvertFrom-Json $env:SIDENOTE_MEETING_FOLDERS); & $env:SIDENOTE_NOTES_SCRIPT -MeetingFolders $folders; if (!$?) { exit 1 }"])
            .env("SIDENOTE_NOTES_SCRIPT", script)
            .env("SIDENOTE_MEETING_FOLDERS", serde_json::to_string(folders)?);
    } else {
        report(LEGACY_SCRIPT_NOTICE);
        command
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(script);
    }
    command.creation_flags(HIDDEN);
    Ok((command, filtered))
}
fn catch_up() -> io::Result<()> {
    let _activity = crate::updates::activity().at(Stage::CatchUpActivity)?;
    if crate::core::meeting_router::jobs::configured() {
        catch_up_routing(
            &crate::core::state_dir(),
            &crate::core::meetings_dir(),
            |id| crate::core::meeting_router::jobs::enqueue(id).map_err(io::Error::other),
        )
        .at(Stage::CatchUpRouting)?;
        crate::core::meeting_router::jobs::start_worker().at(Stage::CatchUpRouting)?;
    }
    pipeline(None)
}
fn continue_after_pipeline(result: io::Result<()>, report: impl FnOnce(&str)) {
    if let Err(error) = result {
        report(&format!("catch-up: {}", Stage::CatchUpEnqueue.error(error)));
    }
}
// A streaming HTTP body, with no TLS or proxy needed for the loopback API.
struct Body {
    stream: BufReader<TcpStream>,
    chunked: bool,
    remaining: usize,
    ended: bool,
}
impl Read for Body {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() || self.ended {
            return Ok(0);
        }
        if !self.chunked {
            return self.stream.read(out);
        }
        if self.remaining == 0 {
            let mut line = String::new();
            self.stream.read_line(&mut line)?;
            self.remaining = usize::from_str_radix(line.trim().split(';').next().unwrap_or(""), 16)
                .map_err(|_| io::ErrorKind::InvalidData)?;
            if self.remaining == 0 {
                self.ended = true;
                return Ok(0);
            }
        }
        let len = out.len().min(self.remaining);
        let n = self.stream.read(&mut out[..len])?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.remaining -= n;
        if self.remaining == 0 {
            let mut end = [0; 2];
            self.stream.read_exact(&mut end)?;
            if end != *b"\r\n" {
                return Err(io::ErrorKind::InvalidData.into());
            }
        }
        Ok(n)
    }
}
fn listen(finished: &std::sync::mpsc::Sender<String>) -> Result<(), Problem> {
    let path =
        PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("hex/local-api.json");
    let config: Discovery = serde_json::from_slice(&fs::read(path)?).map_err(|_| Problem::Value)?;
    // Reject header injection without ever including the credential in an error.
    if config.token.contains(['\r', '\n']) {
        return Err(Problem::Value);
    }
    let addr: SocketAddr = ([127, 0, 0, 1], config.port).into();
    let mut socket = TcpStream::connect_timeout(&addr, Duration::from_secs(60))?;
    let (watch, stopped) = std::sync::mpsc::channel::<()>();
    let _watch = watch;
    if let Ok(control) = socket.try_clone() {
        std::thread::spawn(move || {
            while !crate::updates::listener_stopping() {
                if matches!(
                    stopped.recv_timeout(Duration::from_millis(250)),
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
                ) {
                    return;
                }
            }
            let _ = control.shutdown(std::net::Shutdown::Both);
        });
    }
    socket.set_read_timeout(Some(Duration::from_secs(60)))?;
    socket.set_write_timeout(Some(Duration::from_secs(60)))?;
    write!(
        socket,
        "GET /meetings/events HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nAccept: text/event-stream\r\nConnection: close\r\n\r\n",
        config.port, config.token
    )?;
    let mut stream = BufReader::new(socket);
    let mut header = String::new();
    stream.read_line(&mut header)?;
    let status = header
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or(Problem::Value)?;
    if !(200..300).contains(&status) {
        return Err(Problem::Http(status));
    }
    let mut chunked = false;
    loop {
        header.clear();
        let n = stream.read_line(&mut header)?;
        if n == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
        }
        if header == "\r\n" || header == "\n" {
            break;
        }
        if header
            .to_ascii_lowercase()
            .starts_with("transfer-encoding:")
            && header.to_ascii_lowercase().contains("chunked")
        {
            chunked = true;
        }
    }
    log("connected");
    crate::hex_engine::meeting_event();
    continue_after_pipeline(catch_up(), log);
    let mut body = BufReader::new(Body {
        stream,
        chunked,
        remaining: 0,
        ended: false,
    });
    let mut raw = Vec::new();
    loop {
        raw.clear();
        if body.read_until(b'\n', &mut raw)? == 0 {
            break;
        }
        let decoded = String::from_utf8_lossy(&raw);
        let Some(data) = decoded.trim().strip_prefix("data:") else {
            continue;
        };
        let event: serde_json::Value = serde_json::from_str(data).map_err(|_| Problem::Value)?;
        crate::hex_engine::meeting_event();
        if event.get("type").and_then(|v| v.as_str()) == Some("meeting.finished") {
            let id = event.get("id").and_then(|v| v.as_str()).unwrap_or("None");
            // Only meeting ids belong in the shared log, never server-provided error bodies.
            if id.chars().all(|c| c.is_ascii_digit() || c == '-') {
                log(&format!("meeting finished {id}"));
            } else {
                log("meeting finished None");
            }
            let valid = !id.is_empty()
                && id.bytes().any(|c| c.is_ascii_digit())
                && id.bytes().all(|c| c.is_ascii_digit() || c == b'-');
            if valid {
                let _ = finished.send(id.to_owned());
            }
            continue_after_pipeline(pipeline(valid.then_some(id)), log);
        }
    }
    Ok(())
}

pub fn run() -> ! {
    let _instance = match crate::updates::listener_lock() {
        Ok(lock) if !crate::updates::listener_stopping() => lock,
        _ => std::process::exit(0),
    };
    let isolated = std::env::args().any(|a| a == "--listener-isolated")
        && std::env::args().any(|a| a == "--data-dir");
    crate::meeting_icon::start(isolated);
    if isolated && !std::env::args().any(|a| a == "--floating-icon-replay") {
        while !crate::updates::listener_stopping() {
            std::thread::sleep(Duration::from_millis(250));
        }
        std::process::exit(0);
    }
    status::start();
    // The watcher scans completed meetings even when the event API is unavailable.
    let finished = crate::core::meeting_watch::start(log);
    if let Err(error) = catch_up() {
        log(&format!(
            "startup catch-up: {}",
            Stage::CatchUpRouting.error(error)
        ));
    }
    std::thread::spawn(|| {
        while !crate::updates::listener_stopping() {
            std::thread::sleep(Duration::from_millis(250));
        }
        std::process::exit(0);
    });
    std::thread::spawn(|| {
        loop {
            let _ = crate::core::meeting_router::jobs::start_worker();
            if let Err(error) = crate::core::notes::queue::start_worker() {
                log(&format!(
                    "notes worker: {}",
                    Stage::NotesWorkerSpawn.error(error)
                ));
            }
            if crate::core::meeting_router::jobs::configured() {
                let _ = catch_up_routing(
                    &crate::core::state_dir(),
                    &crate::core::meetings_dir(),
                    |id| {
                        if crate::core::meeting_router::jobs::has_result(id) {
                            Ok(())
                        } else {
                            crate::core::meeting_router::jobs::enqueue(id).map_err(io::Error::other)
                        }
                    },
                );
            }
            if let Err(error) = pipeline(None) {
                log(&format!(
                    "catch-up: {}",
                    Stage::CatchUpMeetings.error(error)
                ));
            }
            std::thread::sleep(Duration::from_secs(30));
        }
    });
    let mut last = String::new();
    loop {
        let problem = match listen(&finished) {
            Ok(()) => "stream ended".into(),
            Err(e) => e.message(),
        };
        if problem != last {
            log(&format!("{problem}; retrying every 30 s"));
            last = problem;
        }
        std::thread::sleep(Duration::from_secs(30));
    }
}

#[cfg(test)]
mod pipeline_tests {
    use super::*;
    #[test]
    fn script_execution_failures_keep_only_stage_and_error_kind() {
        use std::os::windows::process::ExitStatusExt;
        for kind in [io::ErrorKind::PermissionDenied, io::ErrorKind::TimedOut] {
            let error =
                script_result(Err(io::Error::new(kind, "synthetic private detail"))).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("CATCH_UP_SCRIPT_COMMAND ({kind:?})")
            );
        }
        let error = script_result(Ok(std::process::ExitStatus::from_raw(1))).unwrap_err();
        assert_eq!(error.to_string(), "CATCH_UP_SCRIPT_COMMAND (Other)");
        assert!(script_result(Ok(std::process::ExitStatus::from_raw(0))).is_ok());
    }

    #[test]
    fn event_catch_up_and_script_folders_share_the_gate() {
        let root = std::env::temp_dir().join(format!(
            "rec-guard-listener-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let meetings = root.join("meetings");
        for (id, duration) in [("1793946600000-1", 29999), ("1793946600000-2", 30000)] {
            let folder = meetings.join(id);
            fs::create_dir_all(&folder).unwrap();
            fs::write(
                folder.join("manifest.json"),
                serde_json::json!({"status":"complete","duration_ms":duration}).to_string(),
            )
            .unwrap();
            fs::write(
                folder.join("transcript.ndjson"),
                serde_json::json!({"text":vec!["synthetic";25].join(" ")}).to_string(),
            )
            .unwrap();
        }
        // A rejected event returns before activity locks, config reads or worker launches.
        pipeline_in(&root, &meetings, Some("1793946600000-1")).unwrap();
        let mut queued = Vec::new();
        catch_up_routing(&root, &meetings, |id| {
            queued.push(id.to_owned());
            Ok(())
        })
        .unwrap();
        assert_eq!(queued, ["1793946600000-2"]);
        let folders = script_folders(&root, &meetings, None).unwrap();
        assert_eq!(folders, [meetings.join("1793946600000-2")]);
        assert!(
            script_folders(&root, &meetings, Some("1793946600000-1"))
                .unwrap()
                .is_empty()
        );
    }
    fn script_fixture() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "rec-guard-script-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }
    #[test]
    fn legacy_no_param_script_runs_without_arguments_or_changed_environment_and_logs_once() {
        let root = script_fixture();
        let script = root.join("notes.ps1");
        let output = root.join("legacy.json");
        let logfile = root.join("listener.log");
        fs::write(&script, "@{arguments=@($args); parameters=@($PSBoundParameters.Keys); folders=$env:SIDENOTE_MEETING_FOLDERS} | ConvertTo-Json | Set-Content -LiteralPath $env:REC_GUARD_OUTPUT").unwrap();
        let (mut command, filtered) =
            script_command(&script, &[root.join("1793946600000-2")], |message| {
                log_at(&logfile, message)
            })
            .unwrap();
        assert!(!filtered);
        assert_eq!(command.get_envs().count(), 0);
        assert_eq!(
            command
                .get_args()
                .map(|v| v.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            vec![
                "-NoProfile".to_owned(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-File".into(),
                script.to_string_lossy().into_owned()
            ]
        );
        let result = command.env("REC_GUARD_OUTPUT", &output).output().unwrap();
        assert!(result.status.success());
        let actual: serde_json::Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
        assert_eq!(actual["arguments"], serde_json::json!([]));
        assert_eq!(actual["parameters"], serde_json::json!([]));
        assert_eq!(
            actual["folders"].as_str(),
            std::env::var("SIDENOTE_MEETING_FOLDERS").ok().as_deref()
        );
        let log = fs::read_to_string(logfile).unwrap();
        assert_eq!(log.lines().count(), 1);
        assert!(log.contains(LEGACY_SCRIPT_NOTICE));
    }
    #[test]
    fn param_script_receives_only_eligible_folders() {
        let root = script_fixture();
        let meetings = root.join("meetings");
        let script = root.join("notes.ps1");
        let output = root.join("folders.json");
        for (id, duration) in [("1793946600000-1", 29999), ("1793946600000-2", 30000)] {
            let folder = meetings.join(id);
            fs::create_dir_all(&folder).unwrap();
            fs::write(
                folder.join("manifest.json"),
                serde_json::json!({"status":"complete","duration_ms":duration}).to_string(),
            )
            .unwrap();
            fs::write(
                folder.join("transcript.ndjson"),
                serde_json::json!({"text":vec!["synthetic";25].join(" ")}).to_string(),
            )
            .unwrap();
        }
        fs::write(&script, "param([string[]]$MeetingFolders)\nConvertTo-Json -InputObject @($MeetingFolders) | Set-Content -LiteralPath $env:REC_GUARD_OUTPUT").unwrap();
        let folders = script_folders(&root, &meetings, None).unwrap();
        let (mut command, filtered) = script_command(&script, &folders, |_| {
            panic!("parameter script must not log a legacy notice")
        })
        .unwrap();
        assert!(filtered);
        assert!(
            command
                .env("REC_GUARD_OUTPUT", &output)
                .output()
                .unwrap()
                .status
                .success()
        );
        let actual: Vec<PathBuf> = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
        assert_eq!(actual, [meetings.join("1793946600000-2")]);
        assert_eq!(legacy_script_folders(&meetings, None).unwrap().len(), 2);
    }
    #[test]
    fn param_script_keeps_mixed_separator_paths_by_meeting_id() {
        let root = script_fixture();
        let meetings = root.join("hex/meetings");
        let id = "1793946600000-2";
        for name in [id, "1793946600000-3"] {
            fs::create_dir_all(meetings.join(name)).unwrap();
        }
        let mixed = PathBuf::from(format!(r"{}\hex/meetings\{id}", root.display()));
        assert!(mixed.to_string_lossy().contains(r"hex/meetings\"));
        let script = root.join("leaf-filter.ps1");
        let output = root.join("kept.json");
        fs::write(&script, r#"param([string[]]$MeetingFolders)
$allowed = if ($PSBoundParameters.ContainsKey('MeetingFolders')) { @($MeetingFolders | ForEach-Object { Split-Path -Leaf $_ }) }
if ((Get-ExecutionPolicy -Scope Process) -ne 'Bypass') { throw 'Expected the dev16 execution policy' }
$kept = @(foreach ($dir in Get-ChildItem -LiteralPath $env:REC_GUARD_MEETINGS -Directory) {
    if ($null -ne $allowed -and $dir.Name -notin $allowed) { continue }
    $dir.Name
})
ConvertTo-Json -InputObject $kept | Set-Content -LiteralPath $env:REC_GUARD_OUTPUT
"#).unwrap();
        let (mut command, filtered) = script_command(&script, &[mixed], |_| {
            panic!("parameter script must not use the legacy fallback")
        })
        .unwrap();
        assert!(filtered);
        let result = command
            .env("REC_GUARD_MEETINGS", &meetings)
            .env("REC_GUARD_OUTPUT", &output)
            .output()
            .unwrap();
        assert!(result.status.success());
        let kept: Vec<String> = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
        assert_eq!(kept, [id]);
    }
    #[test]
    fn script_failure_markers_match_dev16() {
        let root = script_fixture();
        let folders = [root.clone()];
        script_markers(&folders, false);
        assert_eq!(fs::read(root.join("notes-failed")).unwrap(), b"failed");
        script_markers(&folders, true);
        assert!(!root.join("notes-failed").exists());
    }
    #[test]
    fn enqueue_failure_is_reported_without_ending_event_handling() {
        let mut messages = Vec::new();
        for result in [Err(io::Error::other("private failure")), Ok(())] {
            continue_after_pipeline(result, |message| messages.push(message.to_owned()));
        }
        assert_eq!(messages, ["catch-up: CATCH_UP_ENQUEUE (Other)"]);
    }
}
