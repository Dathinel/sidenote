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
thread_local! { static BEFORE_ATTACH: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) }; }
/// Test-only stand-in for a slow scheduler: runs between the spawn and the job assignment.
pub(super) fn before_attach() {
    if let Some(hook) = BEFORE_ATTACH.with(|h| h.borrow_mut().take()) {
        hook();
    }
}
thread_local! { static READY_FILE: RefCell<Option<PathBuf>> = const { RefCell::new(None) }; }
thread_local! { static READY_AT: RefCell<Option<Instant>> = const { RefCell::new(None) }; }
/// Test-only: a PowerShell fake can take seconds to start on a cold machine. The product
/// deadline is unchanged; the test starts it only once the fake says it is running, by
/// writing its process id to this file as its first act.
pub(super) fn after_resume() {
    let Some(file) = READY_FILE.with(|f| f.borrow_mut().take()) else {
        return;
    };
    let started = Instant::now();
    while fs::read_to_string(&file)
        .ok()
        .and_then(|t| t.trim().parse::<u32>().ok())
        .is_none()
    {
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "the fake CLI never reported that it started"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    READY_AT.with(|t| *t.borrow_mut() = Some(Instant::now()));
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
        fs::write(&agent.path, client.config(Path::new("sidenote"))).unwrap();
        assert_eq!(
            Agent::inspect(client, agent.path, None, |_, _| panic!("no CLI for JSON")).status,
            Status::Added
        );
    }
}
#[cfg(feature = "nightly")]
const ORIGINAL: &[u8] = br#"{"keep":1,"mcpServers":{}}"#;
#[cfg(feature = "nightly")]
fn prepared(sandbox: &Sandbox, client: Client) -> Agent {
    let agent = sandbox.agent(client);
    fs::create_dir_all(agent.path.parent().unwrap()).unwrap();
    fs::write(&agent.path, ORIGINAL).unwrap();
    agent
}
fn files_in(agent: &Agent) -> usize {
    fs::read_dir(agent.path.parent().unwrap()).unwrap().count()
}
#[test]
#[cfg(feature = "nightly")]
fn json_agents_are_never_written_by_add_or_add_all() {
    let sandbox = Sandbox::new();
    for client in [Client::ClaudeDesktop, Client::Cursor, Client::OpenCode] {
        let agent = prepared(&sandbox, client);
        let error = agent.add(Path::new("sidenote")).unwrap_err();
        assert!(error.starts_with("Needs a paste"), "{error}");
        assert!(error.contains(&agent.path.display().to_string()), "{error}");
        let all = add_all(std::slice::from_ref(&agent), Path::new("sidenote"));
        assert!(all[0].1.as_ref().unwrap_err().starts_with("Needs a paste"));
        assert_eq!(fs::read(&agent.path).unwrap(), ORIGINAL);
        assert_eq!(files_in(&agent), 1, "no backup or temporary file");
    }
}
/// Does what the row's line says: put the copied text inside the named section.
fn paste_inside_section(existing: &str, key: &str, pasted: &str) -> String {
    let open = format!("\"{key}\": {{");
    let at = existing.find(&open).unwrap() + open.len();
    format!("{} {pasted}, {}", &existing[..at], &existing[at..])
}
#[test]
fn paste_config_is_the_inner_entry_and_names_the_sidenote_server() {
    let sandbox = Sandbox::new();
    let exe = Path::new("C:/A B/sidenote.exe");
    for client in [Client::ClaudeDesktop, Client::Cursor, Client::OpenCode] {
        let text = sandbox.agent(client).paste_config(exe);
        let value: serde_json::Value = serde_json::from_str(&format!("{{{text}}}")).unwrap();
        let entry = &value["sidenote"];
        let (command, args) = if client == Client::OpenCode {
            assert_eq!(entry["type"], "local");
            (&entry["command"][0], &entry["command"][1])
        } else {
            (&entry["command"], &entry["args"][0])
        };
        assert_eq!(command, "C:/A B/sidenote.exe");
        assert_eq!(args, "--mcp");
    }
}
#[test]
fn pasting_where_the_line_says_keeps_the_other_server() {
    let sandbox = Sandbox::new();
    for client in [Client::ClaudeDesktop, Client::Cursor, Client::OpenCode] {
        let agent = sandbox.agent(client);
        let key = super::json_config::key(client);
        assert!(
            agent
                .paste_note()
                .contains(&format!("inside the \"{key}\" section"))
        );
        let existing = format!(r#"{{"keep": 1, "{key}": {{"other": {{"command": "keep"}}}}}}"#);
        let merged =
            paste_inside_section(&existing, key, &agent.paste_config(Path::new("sidenote")));
        let value: serde_json::Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(value["keep"], 1);
        assert_eq!(value[key]["other"]["command"], "keep");
        assert!(value[key]["sidenote"].is_object(), "{merged}");
        assert_eq!(value[key].as_object().unwrap().len(), 2);
    }
}
#[test]
fn pasting_into_the_empty_file_text_parses_with_only_the_sidenote_server() {
    let sandbox = Sandbox::new();
    for client in [Client::ClaudeDesktop, Client::Cursor, Client::OpenCode] {
        let agent = sandbox.agent(client);
        let key = super::json_config::key(client);
        let note = agent.empty_file_note();
        let template = note.split_once("use: ").unwrap().1;
        assert!(template.contains("...paste here..."));
        let file = template.replace(
            "...paste here...",
            &agent.paste_config(Path::new("sidenote")),
        );
        let value: serde_json::Value = serde_json::from_str(&file).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 1);
        assert_eq!(value[key].as_object().unwrap().len(), 1);
        assert!(value[key]["sidenote"].is_object(), "{file}");
    }
}
#[test]
fn unreadable_configs_are_reported_and_never_changed() {
    let sandbox = Sandbox::new();
    let agent = sandbox.agent(Client::OpenCode);
    fs::create_dir_all(agent.path.parent().unwrap()).unwrap();
    for original in [
        "{ // comment\n}",
        "{/* comment */}",
        "{bad}",
        "[]",
        r#"{"mcp": []}"#,
    ] {
        fs::write(&agent.path, original).unwrap();
        assert!(super::json_config::has_entry(Client::OpenCode, &agent.path).is_err());
        let seen = Agent::inspect(Client::OpenCode, agent.path.clone(), None, |_, _| {
            panic!("no CLI for JSON")
        });
        assert_eq!(seen.status, Status::NotAdded);
        assert!(seen.detail.is_some());
        assert_eq!(fs::read_to_string(&agent.path).unwrap(), original);
        assert_eq!(files_in(&agent), 1);
    }
}
#[test]
#[cfg(feature = "nightly")]
fn add_all_never_writes_json_agents_and_says_where_to_paste() {
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
    for (result, agent) in results.iter().zip(&agents) {
        let error = result.1.as_ref().unwrap_err();
        assert!(error.contains("Needs a paste"), "{error}");
        assert!(error.contains(&agent.path.display().to_string()), "{error}");
    }
    assert_eq!(fs::read_to_string(&agents[0].path).unwrap(), "broken");
    assert!(!agents[1].path.exists() && !agents[2].path.exists());
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
fn json_strings_with_comment_markers_are_valid_and_unchanged() {
    let sandbox = Sandbox::new();
    let agent = sandbox.agent(Client::OpenCode);
    fs::create_dir_all(agent.path.parent().unwrap()).unwrap();
    let original = r#"{"url":"https://example.com/*literal*/"}"#;
    fs::write(&agent.path, original).unwrap();
    assert_eq!(
        super::json_config::has_entry(Client::OpenCode, &agent.path),
        Ok(false)
    );
    assert_eq!(fs::read_to_string(&agent.path).unwrap(), original);
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
        let error = super::json_config::has_entry(client, &agent.path).unwrap_err();
        assert!(error.contains("link"), "{error}");
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

#[cfg(windows)]
fn powershell() -> PathBuf {
    PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe")
}
#[cfg(windows)]
fn fake_cli(sandbox: &Sandbox, name: &str, body: &str) -> (PathBuf, PathBuf) {
    let root = sandbox.0.path().join(name);
    fs::create_dir(&root).unwrap();
    let script = root.join("fake-cli.ps1");
    let pid_file = root.join("pid.txt");
    let text = body
        .replace("PIDFILE", &pid_file.to_string_lossy())
        .replace("POWERSHELL", &powershell().to_string_lossy());
    fs::write(&script, text).unwrap();
    (script, pid_file)
}
#[cfg(windows)]
fn run_fake(
    script: &Path,
    limit: Duration,
    ready: Option<&Path>,
) -> (Result<std::process::Output, String>, Duration) {
    READY_FILE.with(|f| *f.borrow_mut() = ready.map(Path::to_path_buf));
    let args: Vec<&std::ffi::OsStr> = vec![
        "-NoProfile".as_ref(),
        "-NonInteractive".as_ref(),
        "-File".as_ref(),
        script.as_os_str(),
    ];
    let started = Instant::now();
    let result = super::run_within(&powershell(), &args, limit);
    // Time the deadline from the moment the fake reported that it started.
    let from = READY_AT.with(|t| t.borrow_mut().take()).unwrap_or(started);
    (result, from.elapsed())
}
#[cfg(windows)]
fn assert_process_gone(pid_file: &Path) {
    let pid: u32 = fs::read_to_string(pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    for _ in 0..50 {
        if !crate::platform::process_running(pid) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("process {pid} was still running after the deadline");
}

#[cfg(windows)]
#[test]
fn a_cli_that_exits_but_leaves_a_descendant_holding_its_output_is_cut_off() {
    let sandbox = Sandbox::new();
    let (script, pid_file) = fake_cli(
        &sandbox,
        "fake-cli-grandchild",
        r#"$info = New-Object System.Diagnostics.ProcessStartInfo
$info.FileName = 'POWERSHELL'
$info.Arguments = '-NoProfile -NonInteractive -Command Start-Sleep -Seconds 40'
$info.UseShellExecute = $false
$child = [System.Diagnostics.Process]::Start($info)
[IO.File]::WriteAllText('PIDFILE', [string]$child.Id)
Write-Output 'done'
exit 0
"#,
    );
    let (result, elapsed) = run_fake(&script, Duration::from_secs(3), Some(&pid_file));
    assert!(result.unwrap_err().contains("did not finish"));
    assert!(elapsed < Duration::from_secs(15), "{elapsed:?}");
    assert_process_gone(&pid_file);
}

#[cfg(windows)]
#[test]
fn a_descendant_started_before_the_job_exists_is_still_killed_at_the_deadline() {
    let sandbox = Sandbox::new();
    // The CLI starts its helper as its first act and then keeps running, so only a child that
    // joins the job before it executes can bring the helper into the job.
    let (script, pid_file) = fake_cli(
        &sandbox,
        "fake-cli-early",
        r#"$info = New-Object System.Diagnostics.ProcessStartInfo
$info.FileName = 'POWERSHELL'
$info.Arguments = '-NoProfile -NonInteractive -Command Start-Sleep -Seconds 40'
$info.UseShellExecute = $false
$child = [System.Diagnostics.Process]::Start($info)
[IO.File]::WriteAllText('PIDFILE', [string]$child.Id)
Start-Sleep -Seconds 40
"#,
    );
    // Give a child that is already running up to 5 seconds to start its helper before the
    // job assignment happens. A suspended child cannot, so it waits the full time.
    let waited = pid_file.clone();
    BEFORE_ATTACH.with(|h| {
        *h.borrow_mut() = Some(Box::new(move || {
            for _ in 0..50 {
                if waited.exists() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }))
    });
    let (result, elapsed) = run_fake(&script, Duration::from_secs(3), Some(&pid_file));
    assert!(result.unwrap_err().contains("did not finish"));
    assert!(elapsed < Duration::from_secs(20), "{elapsed:?}");
    assert_process_gone(&pid_file);
}

#[cfg(windows)]
#[test]
fn a_slow_cli_is_killed_at_the_deadline() {
    let sandbox = Sandbox::new();
    let (script, pid_file) = fake_cli(
        &sandbox,
        "fake-cli-slow",
        "[IO.File]::WriteAllText('PIDFILE', [string]$PID)\nStart-Sleep -Seconds 40\n",
    );
    let (result, elapsed) = run_fake(&script, Duration::from_secs(3), Some(&pid_file));
    assert!(result.unwrap_err().contains("did not finish"));
    assert!(elapsed < Duration::from_secs(15), "{elapsed:?}");
    assert_process_gone(&pid_file);
}

#[cfg(windows)]
#[test]
fn a_cli_that_finishes_in_time_returns_its_output_and_status() {
    let sandbox = Sandbox::new();
    let (script, _) = fake_cli(
        &sandbox,
        "fake-cli-quick",
        "Write-Output 'out-text'\n[Console]::Error.WriteLine('err-text')\nexit 1\n",
    );
    let (result, _) = run_fake(&script, Duration::from_secs(20), None);
    let output = result.unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("out-text"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("err-text"));
}

#[test]
#[cfg(feature = "nightly")]
fn add_all_keeps_added_agents_untouched_and_reports_every_result() {
    let sandbox = Sandbox::new();
    let mut added = prepared(&sandbox, Client::Cursor);
    let entry = Client::Cursor.config(Path::new("sidenote"));
    fs::write(&added.path, &entry).unwrap();
    added.status = Status::Added;
    let pending = prepared(&sandbox, Client::OpenCode);
    let mut missing = sandbox.agent(Client::ClaudeDesktop);
    missing.status = Status::NotInstalled;
    let results = add_all(
        &[added.clone(), pending.clone(), missing.clone()],
        Path::new("sidenote"),
    );
    assert_eq!(results.len(), 2);
    assert!(results[0].0 == Client::Cursor);
    assert!(results[0].1.as_ref().unwrap().starts_with("Already added"));
    assert_eq!(fs::read_to_string(&added.path).unwrap(), entry);
    assert_eq!(files_in(&added), 1);
    assert!(results[1].1.as_ref().unwrap_err().contains("Needs a paste"));
    assert_eq!(fs::read(&pending.path).unwrap(), ORIGINAL);
    assert!(!missing.path.exists());
}

#[test]
fn add_refuses_an_agent_that_is_not_installed_without_writing() {
    let sandbox = Sandbox::new();
    let mut agent = sandbox.agent(Client::Cursor);
    agent.status = Status::NotInstalled;
    assert!(agent.add(Path::new("sidenote")).is_err());
    assert!(!agent.path.exists());
    assert!(!agent.path.parent().unwrap().exists());
}
