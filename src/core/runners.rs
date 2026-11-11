use super::state_dir;
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Mutex,
};
static CONFIG: Mutex<()> = Mutex::new(());
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Runner {
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    pub stdin: bool,
    #[serde(default = "tools_on")]
    pub tools: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}
fn tools_on() -> bool {
    true
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AskProgress {
    StartingCodex,
    Fallback { runner: String },
    Started { runner: String },
    Tool { name: String },
    Text { text: String },
    Finished,
    Failed { message: String },
}
#[derive(Default, Serialize, Deserialize)]
struct Selection {
    last_used_runner: Option<String>,
}
fn write_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    fs::create_dir_all(
        path.parent()
            .ok_or_else(|| io::Error::other("Missing config directory"))?,
    )?;
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temp, serde_json::to_vec_pretty(value)?)?;
    crate::platform::replace_file(&temp, path)
}
pub fn list_runners() -> Result<Vec<Runner>, String> {
    list_runners_at(&state_dir())
}
pub(super) fn list_runners_at(root: &Path) -> Result<Vec<Runner>, String> {
    let _guard = CONFIG
        .lock()
        .map_err(|_| "Runner configuration lock failed")?;
    let path = root.join("runners.json");
    let mut runners: Vec<Runner> = match fs::read(&path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|_| "runners.json is not a valid runner list")?
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let runners: Vec<Runner> =
                serde_json::from_str(include_str!("../../default-runners.json"))
                    .map_err(|_| "Default runner list is invalid")?;
            write_json(&path, &runners).map_err(|_| "Could not create runners.json")?;
            runners
        }
        Err(_) => return Err("Could not read runners.json".into()),
    };
    for name in ["Daily check (Opus)", "Memory migration (answer only)"] {
        if !runners.iter().any(|r| r.name == name) {
            let daily =
                serde_json::from_str::<Vec<Runner>>(include_str!("../../default-runners.json"))
                    .map_err(|_| "Default runner list is invalid")?
                    .into_iter()
                    .find(|r| r.name == name)
                    .ok_or("Default background runner is missing")?;
            runners.push(daily);
            write_json(&path, &runners).map_err(|_| "Could not add the background runner")?;
        }
    }
    let mut names = std::collections::BTreeSet::new();
    for runner in &runners {
        validate(runner)?;
        if !names.insert(&runner.name) {
            return Err("Runner names must be unique".into());
        }
    }
    Ok(runners)
}
pub fn last_used_runner() -> Result<Option<String>, String> {
    match fs::read(state_dir().join("runner-state.json")) {
        Ok(bytes) => serde_json::from_slice::<Selection>(&bytes)
            .map(|s| s.last_used_runner)
            .map_err(|_| "runner-state.json is invalid".into()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("Could not read the last runner".into()),
    }
}
pub fn remember_runner(name: &str) -> Result<(), String> {
    let _guard = CONFIG
        .lock()
        .map_err(|_| "Runner configuration lock failed")?;
    write_json(
        &state_dir().join("runner-state.json"),
        &Selection {
            last_used_runner: Some(name.into()),
        },
    )
    .map_err(|_| "Could not remember the selected runner".into())
}
pub(super) fn kind(runner: &Runner) -> Result<&'static str, String> {
    let name = Path::new(&runner.program)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match name.as_str() {
        "claude" => Ok("claude"),
        "codex" => Ok("codex"),
        "opencode" | "opencode2" => Ok("opencode"),
        _ => Err("Supported runner programs are claude, codex, opencode and opencode2".into()),
    }
}
pub fn validate(runner: &Runner) -> Result<(), String> {
    if runner.name.trim().is_empty() {
        return Err("A runner needs a name".into());
    }
    let harness = kind(runner)?;
    if runner.args.iter().any(|a| {
        a.contains("danger-full-access")
            || a.contains("dangerously-")
            || a == "bypassPermissions"
            || a == "--bare"
            || a == "--safe-mode"
            || a == "--ignore-user-config"
            || a == "--ignore-rules"
    }) {
        return Err("Runner arguments may not bypass hooks, approvals or the sandbox".into());
    }
    if !runner.stdin && !runner.args.iter().any(|a| a.contains("{prompt_file}")) {
        return Err("A runner without stdin needs {prompt_file} in its arguments".into());
    }
    if runner.args.iter().any(|a| a.contains("{model}"))
        && runner.model.as_deref().is_none_or(str::is_empty)
    {
        return Err("A {model} argument needs a model field".into());
    }
    let pair = |flag: &str, value: &str| {
        runner
            .args
            .windows(2)
            .any(|a| a[0] == flag && a[1] == value)
    };
    match harness {
        "claude" if !runner.args.iter().any(|a| a == "-p" || a == "--print") => {
            Err("Claude runners must use print mode".into())
        }
        "claude"
            if !runner.tools
                && (!pair("--tools", "")
                    || !runner.args.iter().any(|a| a == "--strict-mcp-config")) =>
        {
            Err("Answer-only Claude must disable tools and MCP".into())
        }
        "claude"
            if runner.tools
                && (pair("--tools", "")
                    || (runner.args.iter().any(|a| a == "--strict-mcp-config")
                        && !pair("--tools", "Read,Grep,Glob"))) =>
        {
            Err("A tools-enabled Claude runner must allow normal tools and MCP".into())
        }
        "codex" if runner.args.first().is_none_or(|a| a != "exec") => {
            Err("Codex runners must use exec".into())
        }
        "codex" if !runner.tools => {
            Err("Answer-only Codex has not been verified; use the Claude answer-only runner".into())
        }
        // Ask never writes files (owner, 2026-11-06), so Codex always runs in the read-only sandbox.
        "codex" if !pair("--sandbox", "read-only") => {
            Err("Codex runners must use --sandbox read-only".into())
        }
        "opencode" if runner.args.first().is_none_or(|a| a != "run") => {
            Err("OpenCode runners must use run".into())
        }
        "opencode" if !runner.tools => Err(
            "Answer-only OpenCode has not been verified; use the Claude answer-only runner".into(),
        ),
        _ => Ok(()),
    }
}
/// True when the runner's program resolves to a native executable, as a start would find it.
pub fn installed(runner: &Runner) -> bool {
    executable(runner).is_ok()
}
pub(super) fn executable(runner: &Runner) -> Result<PathBuf, String> {
    fn windows_exe(path: &Path) -> bool {
        use std::io::{Read, Seek, SeekFrom};
        let Ok(mut file) = fs::File::open(path) else {
            return false;
        };
        let mut header = [0; 64];
        if file.read_exact(&mut header).is_err() || &header[..2] != b"MZ" {
            return false;
        }
        let offset = u32::from_le_bytes(header[60..64].try_into().unwrap());
        let mut signature = [0; 4];
        file.seek(SeekFrom::Start(offset as u64)).is_ok()
            && file.read_exact(&mut signature).is_ok()
            && signature == *b"PE\0\0"
    }
    let candidate = PathBuf::from(&runner.program);
    if candidate.is_file() {
        return if windows_exe(&candidate) {
            Ok(candidate)
        } else {
            Err("The configured program is not a native Windows executable".into())
        };
    }
    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let exe = dir.join(if candidate.extension().is_some() {
            runner.program.clone()
        } else {
            format!("{}.exe", runner.program)
        });
        if windows_exe(&exe) {
            return Ok(exe);
        }
        if !dir.join(format!("{}.cmd", runner.program)).is_file() {
            continue;
        }
        let native=match runner.program.as_str(){
            "codex"=>dir.join("node_modules/@openai/codex/node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/bin/codex.exe"),
            "opencode"=>dir.join("node_modules/@opencode/cli/bin/opencode.exe"),
            "opencode2"=>dir.join("node_modules/@opencode-ai/cli/bin/opencode2.exe"),_=>continue,
        };
        if windows_exe(&native) {
            return Ok(native);
        }
    }
    Err(format!("{} is not available on PATH", runner.program))
}
