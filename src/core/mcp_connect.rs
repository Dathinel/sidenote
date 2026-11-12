//! Local client setup. Only the explicit Claude Code action changes client state.
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Client {
    #[default]
    ClaudeCode,
    ClaudeDesktop,
    Codex,
    Cursor,
    OpenCode,
}
impl Client {
    pub const ALL: [Self; 5] = [
        Self::ClaudeCode,
        Self::ClaudeDesktop,
        Self::Codex,
        Self::Cursor,
        Self::OpenCode,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::ClaudeDesktop => "Claude Desktop",
            Self::Codex => "Codex",
            Self::Cursor => "Cursor",
            Self::OpenCode => "OpenCode",
        }
    }
    pub fn config_path(self) -> PathBuf {
        let home = crate::platform::home();
        match self {
            Self::ClaudeCode => home.join(".claude.json"),
            Self::ClaudeDesktop => {
                crate::platform::data_home().join("Claude/claude_desktop_config.json")
            }
            Self::Codex => std::env::var_os("CODEX_HOME")
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".codex"))
                .join("config.toml"),
            Self::Cursor => home.join(".cursor/mcp.json"),
            Self::OpenCode => {
                let base = home.join(".config/opencode");
                if base.join("opencode.jsonc").is_file() {
                    base.join("opencode.jsonc")
                } else {
                    base.join("opencode.json")
                }
            }
        }
    }
    pub fn config(self, exe: &Path) -> String {
        let path = exe.to_string_lossy();
        match self {
            Self::ClaudeCode => format!("claude mcp add --transport stdio --scope user sidenote -- '{}' --mcp", path.replace('\'', "''")),
            Self::Codex => format!("[mcp_servers.sidenote]\ncommand = {}\nargs = [\"--mcp\"]", serde_json::to_string(path.as_ref()).unwrap()),
            Self::ClaudeDesktop | Self::Cursor => serde_json::to_string_pretty(&json!({"mcpServers":{"sidenote":{"command":path,"args":["--mcp"]}}})).unwrap(),
            Self::OpenCode => serde_json::to_string_pretty(&json!({"mcp":{"sidenote":{"type":"local","command":[path,"--mcp"],"enabled":true}}})).unwrap(),
        }
    }
}
#[cfg(windows)]
fn claude() -> Result<PathBuf, String> {
    let runner = super::Runner {
        name: "Claude Code".into(),
        program: "claude".into(),
        args: vec![],
        stdin: false,
        tools: false,
        model: None,
    };
    super::runners::executable(&runner).or_else(|_| {
        let path = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default().join(".local/bin/claude.exe");
        super::runners::executable(&super::Runner { program: path.to_string_lossy().into_owned(), ..runner })
    }).map_err(|_| "Claude Code was not found. Install its native Windows CLI, reopen Sidenote, or use the copyable command.".into())
}
fn run(program: &Path, args: &[&std::ffi::OsStr]) -> Result<std::process::Output, String> {
    use crate::platform::CommandExt;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(0x08000000)
        .spawn()
        .map_err(|_| {
            "Could not start Claude Code. Check its installation and try again.".to_string()
        })?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return child
                    .wait_with_output()
                    .map_err(|_| "Could not read the Claude Code result.".into());
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Claude Code did not finish within 30 seconds. Check its MCP settings before retrying.".into());
            }
        }
    }
}
pub fn add_to_claude(exe: &Path) -> Result<String, String> {
    let cli = claude()?;
    register(exe, |args| run(&cli, args))
}
fn register(
    exe: &Path,
    mut command: impl FnMut(&[&std::ffi::OsStr]) -> Result<std::process::Output, String>,
) -> Result<String, String> {
    let check = command(&["mcp".as_ref(), "get".as_ref(), "sidenote".as_ref()])?;
    if check.status.success() {
        return Err("Claude Code already has a sidenote connection. Nothing was replaced. Check it with claude mcp get sidenote.".into());
    }
    let absent = String::from_utf8_lossy(&check.stderr)
        .to_lowercase()
        .contains("no mcp server found with name")
        || String::from_utf8_lossy(&check.stdout)
            .to_lowercase()
            .contains("no mcp server found with name");
    if !absent {
        return Err("Could not check existing Claude Code connections. Nothing was added. Run claude mcp get sidenote to check.".into());
    }
    let result = command(&[
        "mcp".as_ref(),
        "add".as_ref(),
        "--transport".as_ref(),
        "stdio".as_ref(),
        "--scope".as_ref(),
        "user".as_ref(),
        "sidenote".as_ref(),
        "--".as_ref(),
        exe.as_os_str(),
        "--mcp".as_ref(),
    ])?;
    if !result.status.success() {
        return Err("Claude Code could not add the connection. Check claude mcp get sidenote, or use the copyable command.".into());
    }
    let saved = command(&["mcp".as_ref(), "get".as_ref(), "sidenote".as_ref()])?;
    if !saved.status.success() {
        return Err("Claude Code accepted the command, but the connection could not be verified. Check claude mcp get sidenote.".into());
    }
    Ok("Added to Claude Code for your user account. Restart its MCP session to connect. To undo, run claude mcp remove --scope user sidenote.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn config_preserves_windows_paths_and_arguments() {
        let path = Path::new(r#"C:\Users\A person's name\current\sidenote.exe"#);
        for client in [Client::ClaudeDesktop, Client::Cursor, Client::OpenCode] {
            let value: serde_json::Value = serde_json::from_str(&client.config(path)).unwrap();
            let command = if client == Client::OpenCode {
                &value["mcp"]["sidenote"]["command"][0]
            } else {
                &value["mcpServers"]["sidenote"]["command"]
            };
            assert_eq!(command, &json!(path.to_string_lossy()));
        }
        let config = Client::Codex.config(path);
        let value: String = serde_json::from_str(
            config
                .lines()
                .nth(1)
                .unwrap()
                .strip_prefix("command = ")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(value, path.to_string_lossy());
        assert!(Client::ClaudeCode.config(path).contains("A person''s name"));
        assert!(Client::ClaudeCode.config(path).contains("--scope user"));
    }
}

#[cfg(test)]
mod registration_tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;
    #[cfg(windows)]
    use std::os::windows::process::ExitStatusExt;
    fn output(ok: bool, stderr: &str) -> std::process::Output {
        std::process::Output {
            status: std::process::ExitStatus::from_raw(if ok { 0 } else { 1 }),
            stdout: vec![],
            stderr: stderr.as_bytes().to_vec(),
        }
    }
    #[test]
    fn existing_entry_or_inconclusive_check_never_adds() {
        for reply in [output(true, ""), output(false, "config unreadable")] {
            let mut replies = vec![reply].into_iter();
            assert!(
                register(Path::new("sidenote.exe"), |_| Ok(replies
                    .next()
                    .expect("must not add")))
                .is_err()
            );
        }
    }
    #[test]
    fn add_uses_separate_arguments_and_checks_exit_and_readback() {
        let exe = Path::new(r"C:\A B\sidenote.exe");
        for (add_ok, read_ok) in [(false, false), (true, false), (true, true)] {
            let mut step = 0;
            let result = register(exe, |args| {
                step += 1;
                if step == 1 {
                    return Ok(output(false, "No MCP server found with name: sidenote"));
                }
                if step == 2 {
                    assert_eq!(
                        args,
                        [
                            "mcp".as_ref(),
                            "add".as_ref(),
                            "--transport".as_ref(),
                            "stdio".as_ref(),
                            "--scope".as_ref(),
                            "user".as_ref(),
                            "sidenote".as_ref(),
                            "--".as_ref(),
                            exe.as_os_str(),
                            "--mcp".as_ref()
                        ]
                    );
                    return Ok(output(add_ok, ""));
                }
                assert_eq!(step, 3);
                Ok(output(read_ok, ""))
            });
            assert_eq!(result.is_ok(), add_ok && read_ok);
            assert_eq!(step, if add_ok { 3 } else { 2 });
        }
    }
}

#[cfg(not(windows))]
fn claude() -> Result<PathBuf, String> {
    Err(crate::platform::UNAVAILABLE.into())
}
