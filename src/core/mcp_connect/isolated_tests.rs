use super::{agents::Agent, *};
use std::{cell::RefCell, fs};
thread_local! { static ROOT: RefCell<Option<PathBuf>> = const { RefCell::new(None) }; }
pub(super) fn check_path(path: &Path) {
    let mut resolved = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                assert!(resolved.pop(), "config path escaped temporary home");
            }
            std::path::Component::CurDir => {}
            _ => resolved.push(component.as_os_str()),
        }
    }
    ROOT.with(|root| {
        let root = root.borrow();
        let root = root.as_ref().expect("isolated test root required");
        assert!(
            resolved.starts_with(root),
            "config path escaped temporary home"
        );
        if let Some(existing) = resolved.ancestors().find(|p| p.exists()) {
            assert!(
                existing
                    .canonicalize()
                    .unwrap()
                    .starts_with(root.canonicalize().unwrap()),
                "config path escaped temporary home"
            );
        }
    });
}

pub(super) fn forbid_discovery() {
    panic!("tests must inject isolated paths and fake CLIs");
}
struct Sandbox(tempfile::TempDir);
impl Sandbox {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        ROOT.with(|r| *r.borrow_mut() = Some(temp.path().into()));
        for dir in ["home", "appdata", "codex"] {
            fs::create_dir(temp.path().join(dir)).unwrap();
        }
        Self(temp)
    }
    fn path(&self, client: Client) -> PathBuf {
        self.0
            .path()
            .join(format!("config-{}", client.name()))
            .join("config.json")
    }
    fn agent(&self, client: Client) -> Agent {
        Agent {
            client,
            path: self.path(client),
            cli: None,
            status: Status::NotAdded,
            detail: None,
        }
    }
}
fn output(ok: bool, text: &str) -> std::process::Output {
    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;
    #[cfg(windows)]
    use std::os::windows::process::ExitStatusExt;
    std::process::Output {
        status: std::process::ExitStatus::from_raw(if ok {
            0
        } else if cfg!(windows) {
            1
        } else {
            256
        }),
        stdout: vec![],
        stderr: text.as_bytes().into(),
    }
}
#[test]
fn all_installation_mixes_and_linux_visibility() {
    let sandbox = Sandbox::new();
    for mask in 0..32 {
        for (index, client) in Client::ALL.into_iter().enumerate() {
            let installed = mask & (1 << index) != 0;
            let cli = installed.then(|| sandbox.0.path().join("fake-cli"));
            let agent = Agent::inspect(client, sandbox.path(client), cli, |_, _| {
                Ok(output(
                    false,
                    if client == Client::Codex {
                        "No MCP server named 'sidenote' found."
                    } else {
                        "No MCP server found with name: sidenote"
                    },
                ))
            });
            assert_eq!(
                agent.status,
                if installed {
                    Status::NotAdded
                } else {
                    Status::NotInstalled
                }
            );
            assert_eq!(client.visible(true), client != Client::ClaudeDesktop);
            assert!(client.visible(false));
        }
    }
}
#[test]
fn config_folder_and_existing_entry_detection() {
    let sandbox = Sandbox::new();
    for client in [Client::ClaudeDesktop, Client::Cursor, Client::OpenCode] {
        let agent = sandbox.agent(client);
        fs::create_dir_all(agent.path.parent().unwrap()).unwrap();
        assert_eq!(
            Agent::inspect(client, agent.path.clone(), None, |_, _| panic!(
                "no CLI for JSON"
            ))
            .status,
            Status::NotAdded
        );
        agent.add(Path::new("sidenote")).unwrap();
        assert_eq!(
            Agent::inspect(client, agent.path, None, |_, _| panic!("no CLI for JSON")).status,
            Status::Added
        );
    }
}
#[test]
fn every_json_client_preserves_original_backup_and_other_keys() {
    let sandbox = Sandbox::new();
    for client in [Client::ClaudeDesktop, Client::Cursor, Client::OpenCode] {
        let agent = sandbox.agent(client);
        fs::create_dir_all(agent.path.parent().unwrap()).unwrap();
        let original =
            b"{ \"other\": {\"keep\": 12}, \"mcpServers\": {\"other\": {\"command\": \"keep\"}} }";
        fs::write(&agent.path, original).unwrap();
        let result = agent.add(Path::new("C:/A B/sidenote.exe")).unwrap();
        let backup = fs::read_dir(agent.path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.to_string_lossy().contains(".sidenote-backup-"))
            .unwrap();
        assert_eq!(fs::read(&backup).unwrap(), original);
        assert!(result.contains(&backup.display().to_string()));
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(&agent.path).unwrap()).unwrap();
        assert_eq!(value["other"]["keep"], 12);
        assert_eq!(value["mcpServers"]["other"]["command"], "keep");
        let saved = fs::read(&agent.path).unwrap();
        assert!(agent.add(Path::new("replacement")).is_err());
        assert_eq!(fs::read(&agent.path).unwrap(), saved);
        assert_eq!(
            fs::read_dir(agent.path.parent().unwrap()).unwrap().count(),
            2
        );
    }
}
#[test]
fn invalid_json_comments_and_wrong_shapes_refuse_without_backup() {
    let sandbox = Sandbox::new();
    let agent = sandbox.agent(Client::OpenCode);
    fs::create_dir_all(agent.path.parent().unwrap()).unwrap();
    for original in [
        "{ // comment\n}",
        "{/* comment */}",
        "{bad}",
        "[]",
        r#"{"mcp": []}"#,
        r#"{"mcp":{"sidenote":null}}"#,
    ] {
        fs::write(&agent.path, original).unwrap();
        assert!(agent.add(Path::new("sidenote")).is_err());
        assert_eq!(fs::read_to_string(&agent.path).unwrap(), original);
        assert_eq!(
            fs::read_dir(agent.path.parent().unwrap()).unwrap().count(),
            1
        );
    }
}
#[test]
fn add_all_continues_after_bad_json_and_skips_uninstalled() {
    let sandbox = Sandbox::new();
    let mut agents: Vec<_> = [Client::OpenCode, Client::Cursor, Client::ClaudeDesktop]
        .into_iter()
        .map(|c| sandbox.agent(c))
        .collect();
    fs::create_dir_all(agents[0].path.parent().unwrap()).unwrap();
    fs::write(&agents[0].path, "broken").unwrap();
    agents[2].status = Status::NotInstalled;
    let results = add_all(&agents, Path::new("sidenote"));
    assert_eq!(results.len(), 2);
    assert!(results[0].1.is_err());
    assert!(results[1].1.is_ok());
    assert!(!agents[2].path.exists());
}
#[test]
fn both_cli_clients_keep_existing_and_refuse_uncertain_checks() {
    let sandbox = Sandbox::new();
    for client in [Client::ClaudeCode, Client::Codex] {
        for reply in [output(true, "existing"), output(false, "unreadable config")] {
            let mut replies = Some(reply);
            assert!(
                sandbox
                    .agent(client)
                    .add_cli(Path::new("sidenote"), |_| Ok(replies
                        .take()
                        .expect("must not add")))
                    .is_err()
            );
        }
    }
}
#[test]
fn codex_arguments_add_failure_and_readback() {
    let sandbox = Sandbox::new();
    for (added, saved) in [(false, false), (true, false), (true, true)] {
        let mut calls = vec![];
        let exe = Path::new("C:/A B/sidenote.exe");
        let result = sandbox.agent(Client::Codex).add_cli(exe, |args| {
            calls.push(
                args.iter()
                    .map(|a| a.to_string_lossy().into_owned())
                    .collect::<Vec<_>>(),
            );
            Ok(match calls.len() {
                1 => output(false, "No MCP server named 'sidenote' found."),
                2 => output(added, ""),
                _ => output(saved, ""),
            })
        });
        assert_eq!(calls[0], ["mcp", "get", "sidenote"]);
        assert_eq!(
            calls[1],
            [
                "mcp",
                "add",
                "sidenote",
                "--",
                "C:/A B/sidenote.exe",
                "--mcp"
            ]
        );
        assert_eq!(calls.len(), if added { 3 } else { 2 });
        assert_eq!(result.is_ok(), added && saved);
        if saved {
            assert!(result.unwrap().contains("codex mcp remove sidenote"));
        }
    }
}
#[test]
#[should_panic(expected = "config path escaped temporary home")]
fn config_paths_outside_test_home_fail_before_read() {
    let _sandbox = Sandbox::new();
    let _ = super::json_config::has_entry(Client::Cursor, Path::new("C:/owner/.cursor/mcp.json"));
}

#[cfg(windows)]
#[test]
fn fake_clis_record_production_process_arguments_in_isolated_homes() {
    let sandbox = Sandbox::new();
    let powershell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    for client in [Client::ClaudeCode, Client::Codex] {
        let root = sandbox.0.path().join(format!("fake-cli-{}", client.name()));
        fs::create_dir(&root).unwrap();
        let script = root.join("fake-cli.ps1");
        fs::write(&script, r#"$ErrorActionPreference = 'Stop'
$env:HOME = Join-Path $PSScriptRoot 'home'
$env:APPDATA = Join-Path $PSScriptRoot 'appdata'
$env:CODEX_HOME = Join-Path $PSScriptRoot 'codex'
foreach ($dir in @($env:HOME, $env:APPDATA, $env:CODEX_HOME)) { [void][IO.Directory]::CreateDirectory($dir) }
Add-Content -LiteralPath (Join-Path $PSScriptRoot 'arguments.jsonl') -Value (ConvertTo-Json -InputObject @($args) -Compress)
$marker = Join-Path $env:HOME 'added'
if ($args[1] -eq 'add') { [IO.File]::WriteAllText($marker, 'added'); exit 0 }
if (Test-Path -LiteralPath $marker) { Write-Output 'sidenote'; exit 0 }
[Console]::Error.WriteLine("No MCP server named 'sidenote' found. No MCP server found with name: sidenote")
exit 1
"#).unwrap();
        let result = sandbox
            .agent(client)
            .add_cli(Path::new("C:/A B/sidenote.exe"), |args| {
                let mut command: Vec<&std::ffi::OsStr> = vec![
                    "-NoProfile".as_ref(),
                    "-NonInteractive".as_ref(),
                    "-File".as_ref(),
                    script.as_os_str(),
                ];
                command.extend_from_slice(args);
                super::run(&powershell, &command)
            })
            .unwrap();
        assert!(result.contains("To undo"));
        let lines = fs::read_to_string(root.join("arguments.jsonl")).unwrap();
        let calls: Vec<Vec<String>> = lines
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0], ["mcp", "get", "sidenote"]);
        assert_eq!(calls[2], calls[0]);
        assert_eq!(
            &calls[1][calls[1].len() - 3..],
            ["--", "C:/A B/sidenote.exe", "--mcp"]
        );
        for home in ["home", "appdata", "codex"] {
            assert!(root.join(home).is_dir());
        }
    }
}

#[test]
fn json_strings_with_comment_markers_are_valid_and_preserved() {
    let sandbox = Sandbox::new();
    let agent = sandbox.agent(Client::OpenCode);
    fs::create_dir_all(agent.path.parent().unwrap()).unwrap();
    fs::write(&agent.path, r#"{"url":"https://example.com/*literal*/"}"#).unwrap();
    agent.add(Path::new("sidenote")).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&fs::read(&agent.path).unwrap()).unwrap();
    assert_eq!(value["url"], "https://example.com/*literal*/");
}
#[test]
fn cli_detection_reads_existing_without_adding() {
    let sandbox = Sandbox::new();
    for client in [Client::ClaudeCode, Client::Codex] {
        let mut calls = 0;
        let agent = Agent::inspect(
            client,
            sandbox.path(client),
            Some(sandbox.0.path().join("fake-cli")),
            |_, args| {
                calls += 1;
                assert_eq!(args, ["mcp", "get", "sidenote"].map(std::ffi::OsStr::new));
                Ok(output(true, "existing"))
            },
        );
        assert_eq!(agent.status, Status::Added);
        assert_eq!(calls, 1);
    }
}

#[test]
fn missing_entry_requires_exit_one_and_an_exact_known_server_name() {
    let sandbox = Sandbox::new();
    let replies = [
        "No MCP server found with name: sidenote",
        "No MCP server named \"sidenote\". Configured servers: ...",
        "Error: No MCP server named 'sidenote' found.",
    ];
    for client in [Client::ClaudeCode, Client::Codex] {
        for text in replies
            .into_iter()
            .flat_map(|s| [s.to_string(), s.to_uppercase()])
        {
            let mut calls = 0;
            assert!(
                sandbox
                    .agent(client)
                    .add_cli(Path::new("sidenote"), |_| {
                        calls += 1;
                        Ok(if calls == 1 {
                            output(false, &text)
                        } else {
                            output(true, "")
                        })
                    })
                    .is_ok()
            );
            assert_eq!(calls, 3);
        }
        for text in [
            "No MCP server named 'another' found.",
            "No MCP server named sidenote",
            "config unreadable",
            "No MCP server found with name: sidenote-other",
        ] {
            let mut calls = 0;
            let result = sandbox.agent(client).add_cli(Path::new("sidenote"), |_| {
                calls += 1;
                Ok(output(false, text))
            });
            assert!(result.is_err(), "{text}");
            assert_eq!(calls, 1);
        }
    }
    let mut wrong_exit = output(false, replies[1]);
    #[cfg(windows)]
    {
        use std::os::windows::process::ExitStatusExt;
        wrong_exit.status = std::process::ExitStatus::from_raw(2);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        wrong_exit.status = std::process::ExitStatus::from_raw(2 << 8);
    }
    assert!(!super::missing(&wrong_exit));
}

#[test]
fn default_config_paths_resolve_inside_the_test_home() {
    let _sandbox = Sandbox::new();
    for client in Client::ALL {
        assert!(
            client.config_path().starts_with(crate::test_home()),
            "{}",
            client.name()
        );
    }
}

#[test]
#[should_panic(expected = "config path escaped temporary home")]
fn parent_traversal_outside_test_home_fails_before_read() {
    let sandbox = Sandbox::new();
    let path = sandbox.0.path().join("..").join("owner-config.json");
    let _ = super::json_config::has_entry(Client::Cursor, &path);
}

#[cfg(windows)]
fn link_folder(link: &Path, target: &Path) {
    // A junction needs no privilege, unlike a Windows symlink, and is the case std misses.
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "could not create the test junction");
}
#[cfg(unix)]
fn link_folder(link: &Path, target: &Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[test]
fn linked_config_folder_is_refused_and_the_destination_is_untouched() {
    let sandbox = Sandbox::new();
    for client in [Client::ClaudeDesktop, Client::Cursor, Client::OpenCode] {
        let elsewhere = sandbox
            .0
            .path()
            .join(format!("elsewhere-{}", client.name()));
        fs::create_dir(&elsewhere).unwrap();
        let original = br#"{"keep":1}"#;
        fs::write(elsewhere.join("config.json"), original).unwrap();
        let agent = sandbox.agent(client);
        link_folder(agent.path.parent().unwrap(), &elsewhere);
        let error = agent.add(Path::new("sidenote")).unwrap_err();
        assert!(error.contains("link"), "{error}");
        assert!(super::json_config::has_entry(client, &agent.path).is_err());
        assert_eq!(fs::read(elsewhere.join("config.json")).unwrap(), original);
        assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 1);
    }
}

#[cfg(unix)]
#[test]
fn linked_config_file_is_refused_and_stays_a_link() {
    let sandbox = Sandbox::new();
    let agent = sandbox.agent(Client::Cursor);
    let folder = agent.path.parent().unwrap();
    fs::create_dir(folder).unwrap();
    let real = folder.join("real.json");
    fs::write(&real, br#"{"keep":1}"#).unwrap();
    std::os::unix::fs::symlink(&real, &agent.path).unwrap();
    assert!(agent.add(Path::new("sidenote")).is_err());
    assert!(super::json_config::has_entry(Client::Cursor, &agent.path).is_err());
    assert_eq!(fs::read(&real).unwrap(), br#"{"keep":1}"#);
    assert!(
        fs::symlink_metadata(&agent.path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_dir(folder).unwrap().count(), 2);
}
