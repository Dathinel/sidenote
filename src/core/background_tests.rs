//! Native fake executables on a child test process's PATH; no account or host config access.
use super::*;

#[test]
fn background_fake_cli_paths() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("fake.cs");
    fs::write(&source, r#"
using System;
using System.IO;
class Fake {
 static void Main(string[] args) {
  string name = Path.GetFileNameWithoutExtension(Environment.GetCommandLineArgs()[0]);
  File.AppendAllText(Environment.GetEnvironmentVariable("SIDENOTE_FAKE_CALLS"),
    name + "\n" + String.Join("\n", args) + "\nEND\n");
  Console.In.ReadToEnd();
  if (name == "claude") {
   Console.WriteLine("{\"type\":\"result\",\"result\":\"Synthetic answer\"}");
   return;
  }
  if (Environment.GetEnvironmentVariable("SIDENOTE_FAKE_TOOL") == "1")
   Console.WriteLine("{\"type\":\"item.started\",\"item\":{\"id\":\"write\",\"type\":\"file_change\"}}");
  Console.WriteLine("{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"Synthetic answer\"}}");
 }
}"#).unwrap();
    let exe = root.path().join("codex.exe");
    let compile = format!(
        "Add-Type -OutputType ConsoleApplication -OutputAssembly '{}' -Path '{}'",
        exe.display(),
        source.display()
    );
    let status = crate::platform::worker_command("powershell.exe")
        .unwrap()
        .args(["-NoProfile", "-Command", &compile])
        .output()
        .unwrap();
    assert!(status.status.success(), "fake CLI compilation failed");
    fs::copy(&exe, root.path().join("claude.exe")).unwrap();
    let path = std::env::join_paths(
        std::iter::once(root.path().to_owned())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    for tool in ["0", "1"] {
        let output = crate::platform::worker_command(std::env::current_exe().unwrap())
            .unwrap()
            .args([
                "--exact",
                "core::daily_check::background_tests::background_fake_cli_child",
                "--nocapture",
            ])
            .env("PATH", &path)
            .env("SIDENOTE_DATA_DIR", root.path().join("data"))
            .env(
                "SIDENOTE_FAKE_CALLS",
                root.path().join(format!("calls-{tool}.txt")),
            )
            .env("SIDENOTE_FAKE_CHILD", "1")
            .env("SIDENOTE_FAKE_TOOL", tool)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    }
}

#[test]
fn background_fake_cli_child() {
    if std::env::var("SIDENOTE_FAKE_CHILD").as_deref() != Ok("1") {
        return;
    }
    let work = tempfile::tempdir().unwrap();
    let log = PathBuf::from(std::env::var_os("SIDENOTE_FAKE_CALLS").unwrap());
    let mut codex = daily_runner("Codex Sol", work.path(), work.path()).unwrap();
    let run = |r: &Runner, read_only_background| {
        super::super::runner_process::run_runner_with_timeout(
            r,
            "Synthetic input".into(),
            work.path(),
            Duration::from_secs(10),
            read_only_background,
            |_| {},
        )
    };
    assert_eq!(super::super::runners::kind(&codex).unwrap(), "codex");
    if std::env::var("SIDENOTE_FAKE_TOOL").as_deref() == Ok("1") {
        assert!(
            run(&codex, true).is_err(),
            "daily Codex accepted a file edit"
        );
        assert_eq!(run(&codex, false).unwrap(), "Synthetic answer");
        return;
    }
    assert_eq!(run(&codex, true).unwrap(), "Synthetic answer");
    let calls = fs::read_to_string(&log).unwrap();
    assert!(!calls.contains("claude\n"));
    for flag in [
        "mcp_servers={}",
        "--sandbox\nread-only",
        "approval_policy=\"never\"",
        "features.apps=false",
        "features.plugins=false",
        "features.multi_agent=false",
        "web_search=\"disabled\"",
        "--ephemeral",
        "--json",
    ] {
        assert!(calls.contains(flag), "missing {flag}");
    }
    assert!(!calls.contains("--ignore-user-config"));
    assert!(!calls.contains("features.hooks"));
    assert!(!codex.args.iter().any(|a| a == "--add-dir"));
    let config = super::super::state_dir().join("runners.json");
    let mut presets: Vec<Runner> = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    let preset = presets.iter_mut().find(|r| r.name == "Codex Sol").unwrap();
    let native = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .next()
        .unwrap()
        .join("codex.exe")
        .to_string_lossy()
        .into_owned();
    preset.program = native.clone();
    preset.model = Some("synthetic-selected-model".into());
    fs::write(&config, serde_json::to_vec(&presets).unwrap()).unwrap();
    let selected = daily_runner("Codex Sol", work.path(), work.path()).unwrap();
    assert_eq!(selected.program, native);
    assert_eq!(selected.model.as_deref(), Some("synthetic-selected-model"));
    assert_eq!(run(&selected, true).unwrap(), "Synthetic answer");
    let claude = daily_runner("Claude Sonnet", work.path(), work.path()).unwrap();
    for flag in ["--tools", "--allowedTools"] {
        assert!(
            claude
                .args
                .windows(2)
                .any(|a| a == [flag, "Read,Grep,Glob"])
        );
    }
    assert!(claude.args.iter().any(|a| a == "--strict-mcp-config"));
    assert_eq!(run(&claude, true).unwrap(), "Synthetic answer");
    assert!(fs::read_to_string(&log).unwrap().contains("claude\n"));
    let sandbox = codex.args.iter().position(|a| a == "read-only").unwrap();
    codex.args[sandbox] = "workspace-write".into();
    assert!(super::super::runners::validate(&codex).is_err());
}
