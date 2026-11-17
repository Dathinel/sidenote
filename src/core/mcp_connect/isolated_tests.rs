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
        self.0.path().join(client.name()).join("config.json")
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
#[should_panic(expected = "config path escaped temporary home")]
fn config_paths_outside_test_home_fail_before_read() {
    let _sandbox = Sandbox::new();
    let _ = super::json_config::has_entry(Client::Cursor, Path::new("C:/owner/.cursor/mcp.json"));
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
