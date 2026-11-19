use super::{Client, claude, missing, register, run};
use std::{
    path::{Path, PathBuf},
    process::Output,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Added,
    NotAdded,
    NotInstalled,
}
impl Status {
    pub fn text(self) -> &'static str {
        match self {
            Self::Added => "Added",
            Self::NotAdded => "Not added",
            Self::NotInstalled => "Not installed",
        }
    }
}
#[derive(Clone)]
pub struct Agent {
    pub client: Client,
    pub path: PathBuf,
    pub cli: Option<PathBuf>,
    pub status: Status,
    pub detail: Option<String>,
}
impl Client {
    pub fn visible(self, linux: bool) -> bool {
        !(linux && self == Self::ClaudeDesktop)
    }
    /// Only an app with its own CLI is added by Sidenote, and that CLI writes its own config.
    pub fn adds_by_cli(self) -> bool {
        matches!(self, Self::ClaudeCode | Self::Codex)
    }
    fn program(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude",
            Self::ClaudeDesktop => "claude-desktop",
            Self::Codex => "codex",
            Self::Cursor => "cursor",
            Self::OpenCode => "opencode",
        }
    }
    fn cli(self) -> Option<PathBuf> {
        if self == Self::ClaudeCode {
            return claude().ok();
        }
        super::super::runners::executable(&super::super::Runner {
            name: self.name().into(),
            program: self.program().into(),
            args: vec![],
            stdin: false,
            tools: false,
            model: None,
        })
        .ok()
    }
}
pub fn discover() -> Vec<Agent> {
    // Unit tests must inject paths and runners, never discover the owner's configuration.
    #[cfg(test)]
    super::isolated_tests::forbid_discovery();
    Client::ALL
        .into_iter()
        .filter(|c| c.visible(cfg!(target_os = "linux")))
        .map(|client| {
            Agent::inspect(client, client.config_path(), client.cli(), |cli, args| {
                run(cli, args)
            })
        })
        .collect()
}
impl Agent {
    pub(super) fn inspect(
        client: Client,
        path: PathBuf,
        cli: Option<PathBuf>,
        mut command: impl FnMut(&Path, &[&std::ffi::OsStr]) -> Result<Output, String>,
    ) -> Self {
        #[cfg(test)]
        super::isolated_tests::check_path(&path);
        let installed = cli.is_some()
            || path.is_file()
            || path.parent().is_some_and(Path::is_dir) && client != Client::ClaudeCode
            || client == Client::ClaudeCode && path.with_file_name(".claude").is_dir();
        let mut agent = Self {
            client,
            path,
            cli,
            status: if installed {
                Status::NotAdded
            } else {
                Status::NotInstalled
            },
            detail: None,
        };
        if installed {
            let result = if client.adds_by_cli() {
                agent
                    .cli
                    .as_ref()
                    .ok_or_else(|| "CLI unavailable. Use the copyable config.".into())
                    .and_then(|cli| {
                        let output =
                            command(cli, &["mcp".as_ref(), "get".as_ref(), "sidenote".as_ref()])?;
                        if output.status.success() {
                            Ok(true)
                        } else if missing(&output) {
                            Ok(false)
                        } else {
                            Err("Could not check this connection. Use the copyable config.".into())
                        }
                    })
            } else {
                super::json_config::has_entry(client, &agent.path)
            };
            match result {
                Ok(true) => agent.status = Status::Added,
                Ok(false) => {}
                Err(e) => agent.detail = Some(e),
            }
        }
        agent
    }
    pub fn add(&self, exe: &Path) -> Result<String, String> {
        if self.status == Status::NotInstalled {
            return Err("This app is not installed.".into());
        }
        if !self.client.adds_by_cli() {
            return Err(format!(
                "Needs a paste: Sidenote does not edit {}'s config. {}",
                self.client.name(),
                self.paste_note()
            ));
        }
        let cli = self
            .cli
            .as_ref()
            .ok_or("CLI unavailable. Use the copyable config.")?;
        self.add_cli(exe, |args| run(cli, args))
    }
    /// The exact text to paste into an app that Sidenote does not edit.
    pub fn paste_config(&self, exe: &Path) -> String {
        self.client.config(exe)
    }
    /// Tells the owner where to paste, in one line.
    pub fn paste_note(&self) -> String {
        let place = match self.client {
            Client::OpenCode => "mcp",
            _ => "mcpServers",
        };
        format!(
            "Paste it into {} inside \"{place}\", keep the other entries, then restart {}.",
            self.path.display(),
            self.client.name()
        )
    }
    pub(super) fn add_cli(
        &self,
        exe: &Path,
        command: impl FnMut(&[&std::ffi::OsStr]) -> Result<Output, String>,
    ) -> Result<String, String> {
        match self.client {
            Client::ClaudeCode => register(exe, command),
            Client::Codex => codex(exe, command),
            _ => unreachable!(),
        }
    }
}
fn codex(
    exe: &Path,
    mut command: impl FnMut(&[&std::ffi::OsStr]) -> Result<Output, String>,
) -> Result<String, String> {
    let check = command(&["mcp".as_ref(), "get".as_ref(), "sidenote".as_ref()])?;
    if check.status.success() {
        return Err("Codex already has a sidenote connection. Nothing was replaced. Check codex mcp get sidenote.".into());
    }
    if !missing(&check) {
        return Err(
            "Could not check Codex connections. Nothing was added. Check codex mcp get sidenote."
                .into(),
        );
    }
    let added = command(&[
        "mcp".as_ref(),
        "add".as_ref(),
        "sidenote".as_ref(),
        "--".as_ref(),
        exe.as_os_str(),
        "--mcp".as_ref(),
    ])?;
    if !added.status.success() {
        return Err("Codex could not add the connection. Check codex mcp get sidenote.".into());
    }
    if !command(&["mcp".as_ref(), "get".as_ref(), "sidenote".as_ref()])?
        .status
        .success()
    {
        return Err(
            "Codex accepted the command, but verification failed. Check codex mcp get sidenote."
                .into(),
        );
    }
    Ok("Added to Codex. Restart its MCP session. To undo, run codex mcp remove sidenote.".into())
}
pub fn add_all(agents: &[Agent], exe: &Path) -> Vec<(Client, Result<String, String>)> {
    agents
        .iter()
        .filter(|a| a.status != Status::NotInstalled)
        .map(|a| {
            (
                a.client,
                if a.status == Status::Added {
                    Ok("Already added. Existing connection kept.".into())
                } else {
                    a.add(exe)
                },
            )
        })
        .collect()
}
