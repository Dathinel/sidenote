//! Dispatch inside the replica owner; never include input or backend errors in diagnostics.
use serde_json::{Value, json};
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
static REQUESTS: AtomicU64 = AtomicU64::new(0);
static FAILURES: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "linux")]
pub(super) fn metrics() -> (u64, u64) {
    (
        REQUESTS.load(Ordering::Relaxed),
        FAILURES.load(Ordering::Relaxed),
    )
}

fn read_tool(name: &str) -> bool {
    matches!(
        name,
        "list_meetings"
            | "search_meetings"
            | "get_meeting"
            | "list_tasks"
            | "list_teams"
            | "get_memory"
            | "list_projects"
            | "get_daily_check"
    )
}
pub(super) fn dispatch(method: &str, params: Value, read_only: bool) -> io::Result<Value> {
    // MCP reads inherit --mcp semantics inside the owner; explicit local proposals may write.
    let _read = crate::core::ReadOnlyScope::begin(
        !(method == "call" && params["name"] == "propose_memory"),
    );
    REQUESTS.fetch_add(1, Ordering::Relaxed);
    let result = dispatch_inner(method, params, read_only);
    if result.is_err() {
        FAILURES.fetch_add(1, Ordering::Relaxed);
    }
    result
}
fn dispatch_inner(method: &str, params: Value, read_only: bool) -> io::Result<Value> {
    match method {
        "tools" => Ok(serde_json::to_value(
            crate::mcp::tools()
                .into_iter()
                .filter(|t| !read_only || read_tool(&t.name))
                .collect::<Vec<_>>(),
        )?),
        "call" => {
            let name = params["name"].as_str().ok_or_else(super::invalid)?;
            if read_only && !read_tool(name) {
                return Err(super::invalid());
            }
            let arguments = params["arguments"].clone();
            if !arguments.is_object() || !crate::mcp::tools().iter().any(|t| t.name == name) {
                return Err(super::invalid());
            }
            crate::core::agent_tools::call(name, arguments)
                .map_err(|_| io::Error::other("MCP_DATA_UNAVAILABLE"))
        }
        "resource" => {
            let uri = params["uri"].as_str().ok_or_else(super::invalid)?;
            let text = crate::core::agent_tools::meeting_resource(uri)
                .map_err(|_| io::Error::other("MCP_DATA_UNAVAILABLE"))?;
            Ok(json!(text))
        }
        _ => Err(super::invalid()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owner_memory_reads_do_not_publish_or_repair_exports_but_local_proposals_persist() {
        use crate::core::memory::*;
        use std::{fs, process::Command};
        const CHILD: &str = "SIDENOTE_TEST_OWNER_MEMORY";
        let Ok(root) = std::env::var(CHILD) else {
            let root = tempfile::tempdir().unwrap();
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "sync_owner::mcp::tests::owner_memory_reads_do_not_publish_or_repair_exports_but_local_proposals_persist", "--nocapture"])
                .env(CHILD, root.path())
                .output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            return;
        };
        let root = std::path::PathBuf::from(root);
        crate::core::configure_paths(&[
            "--data-dir".into(),
            root.clone().into_os_string(),
            "--meetings-dir".into(),
            root.join("meetings").into_os_string(),
            "--peers-dir".into(),
            root.join("peers").into_os_string(),
        ])
        .unwrap();
        fs::write(
            root.join("config.json"),
            r#"{"version":1,"features":{"memory":true}}"#,
        )
        .unwrap();
        for dir in ["memory", "checks", "local/memory"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        let file = MemoryFile {
            migration: Some(MigrationInfo {
                version: 1,
                source_path: "synthetic".into(),
                fingerprint: "synthetic".into(),
                accepted_at: "2026-11-13T00:00:00Z".into(),
            }),
            ..Default::default()
        };
        fs::write(
            root.join("memory/records.json"),
            serde_json::to_vec(&file).unwrap(),
        )
        .unwrap();
        let mut proposal = MemoryProposal {
            id: String::new(),
            op: ProposalOp::Add,
            target_id: None,
            text: "Synthetic report proposal".into(),
            record_type: RecordType::Story,
            project: None,
            people: vec![],
            sources: vec![MemorySource::Meeting {
                reference: "[[1800000000000-1]]".into(),
            }],
            rationale: "Synthetic regression".into(),
            target_updated_at: None,
        };
        proposal.id = proposal_id(&proposal);
        fs::write(root.join("checks/2026-11-13.json"), json!({
            "date":"2026-11-13", "runner":"synthetic", "prompt_bytes":0,
            "elapsed_seconds":0, "meeting_count":0, "task_count":0,
            "kb":{"revision":"synthetic","captured_at":"synthetic","path":"synthetic","stale":false},
            "warnings":[], "memory_updated":false, "findings":[], "memory_proposals":[proposal]
        }).to_string()).unwrap();
        let export = root.join("local/memory/company.md");
        let control = std::sync::Mutex::new(super::super::Controller {
            root: root.clone(),
            pair: None,
            state: Default::default(),
        });
        let call = |name: &str, arguments: Value| {
            let bytes = super::super::commands::respond(&control, &serde_json::to_vec(
                &json!({"op":"mcp","method":"call","params":{"name":name,"arguments":arguments}})
            ).unwrap()).unwrap();
            let response: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(response["ok"], true, "{response}");
            response["value"].clone()
        };
        let generation = crate::core::mutation_generation();
        for stale in [false, true] {
            if stale {
                fs::write(&export, "synthetic stale export").unwrap();
            }
            let result = call("get_memory", json!({"scope":"proposals"}));
            assert!(result.to_string().contains("Synthetic report proposal"));
            assert_eq!(crate::core::mutation_generation(), generation);
            assert!(!root.join("state").exists());
            if stale {
                assert_eq!(
                    fs::read_to_string(&export).unwrap(),
                    "synthetic stale export"
                );
            } else {
                assert!(!export.exists());
            }
        }
        call(
            "propose_memory",
            json!({
                "text":"Synthetic explicit proposal", "reason":"Owner requested review"
            }),
        );
        assert!(
            MemoryStore::default()
                .load()
                .unwrap()
                .proposals
                .iter()
                .any(|p| p.proposal.text == "Synthetic explicit proposal")
        );
        assert!(crate::core::mutation_generation() > generation);
    }
    #[test]
    fn rejects_unknown_methods_and_malformed_calls_without_echoing_input() {
        for (method, params) in [
            ("configure", json!({"secret":"synthetic-canary"})),
            ("call", json!({"arguments": {}})),
            ("resource", json!({"uri": 42})),
        ] {
            let error = dispatch(method, params, false).unwrap_err().to_string();
            assert_eq!(error, "SYNC_CONTROL_INVALID_REQUEST");
            assert!(!error.contains("synthetic-canary"));
        }
    }
    #[test]
    fn remote_scope_is_a_closed_read_allowlist() {
        for name in ["propose_memory", "configure", "future_write"] {
            assert!(!read_tool(name));
            assert!(dispatch("call", json!({"name":name,"arguments":{}}), true).is_err());
        }
        assert!(read_tool("get_meeting"));
    }
}
