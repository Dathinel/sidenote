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
   Console.WriteLine(Array.IndexOf(args, "json") >= 0 ? "{\"type\":\"result\",\"result\":\"Interactive answer\"}" : "Interactive answer");
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
        for test in [
            "core::daily_check::background_tests::background_fake_cli_child",
            "core::notes::helper::tests::background_fake_helper_child",
        ] {
            let output = crate::platform::worker_command(std::env::current_exe().unwrap())
                .unwrap()
                .args(["--exact", test, "--nocapture"])
                .env("PATH", &path)
                .env("SIDENOTE_DATA_DIR", root.path().join("data"))
                .env("SIDENOTE_FAKE_CALLS", root.path().join("calls.txt"))
                .env("SIDENOTE_FAKE_CHILD", "1")
                .env("SIDENOTE_FAKE_TOOL", tool)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
    }
}

#[test]
fn background_fake_cli_child() {
    if std::env::var("SIDENOTE_FAKE_CHILD").as_deref() != Ok("1") {
        return;
    }
    let work = tempfile::tempdir().unwrap();
    let log = PathBuf::from(std::env::var_os("SIDENOTE_FAKE_CALLS").unwrap());
    let mut daily = daily_runner(DEFAULT_RUNNER, work.path(), work.path()).unwrap();
    assert_eq!(super::super::runners::kind(&daily).unwrap(), "codex");
    let run = |r: &Runner| {
        super::super::runner_process::run_runner_with_timeout(
            r,
            "Synthetic input".into(),
            work.path(),
            Duration::from_secs(10),
            |_| {},
        )
    };
    if std::env::var("SIDENOTE_FAKE_TOOL").as_deref() == Ok("1") {
        assert!(run(&daily).is_err(), "daily check accepted a file edit");
        let answer_only = super::super::runners::background_codex("gpt-6.1-sol", false);
        assert!(
            run(&answer_only).is_err(),
            "answer-only runner accepted a tool"
        );
        return;
    }
    assert_eq!(run(&daily).unwrap(), "Synthetic answer");
    assert!(daily_runner("Claude Sonnet", work.path(), work.path()).is_err());
    assert!(
        super::super::notes::test_helper(super::super::notes::Helper::Codex, "gpt-6.1-sol")
            .unwrap_err()
            .contains("invalid JSON")
    );
    let migration = MemoryStore::new(super::super::state_dir())
        .prepare_migration(true)
        .unwrap();
    assert_eq!(
        migration.runner.as_deref(),
        Some("Memory migration (Codex)")
    );
    let calls = fs::read_to_string(&log).unwrap();
    assert!(!calls.contains("claude\n"));
    for flag in [
        "--ignore-user-config",
        "features.hooks=false",
        "mcp_servers={}",
        "--sandbox\nread-only",
        "features.shell_tool=false",
    ] {
        assert!(calls.contains(flag), "missing {flag}");
    }
    let index = daily.args.iter().position(|a| a == "read-only").unwrap();
    daily.args[index] = "workspace-write".into();
    assert!(super::super::runners::validate(&daily).is_err());
    let interactive = super::super::list_runners()
        .unwrap()
        .into_iter()
        .find(|r| r.name == "Claude Sonnet (answer only)")
        .unwrap();
    assert_eq!(run(&interactive).unwrap(), "Interactive answer");
    assert!(fs::read_to_string(&log).unwrap().contains("claude\n"));
}
