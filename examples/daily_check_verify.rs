//! Assert-based storage/action smoke check. Run only with a private APPDATA directory.
use chrono::NaiveDate;
use serde_json::json;
use sidenote::core::{self, daily_check::*};
use std::{fs, path::PathBuf};

fn main() {
    // Native runner fixture exercises the real hidden process/JSON validation path without a model call.
    if let Some(response) = std::env::var_os("HEX_DAILY_CHECK_FIXTURE") {
        assert!(
            PathBuf::from(std::env::var_os("APPDATA").unwrap()).ends_with("daily-check-selfcheck")
        );
        let executable = std::env::current_exe().unwrap();
        if executable.file_stem().unwrap() == "git" {
            std::process::exit(1);
        }
        let result = fs::read_to_string(response).unwrap();
        println!(
            "{}",
            json!({"type":"result","result":result,"is_error":false})
        );
        return;
    }
    let root = PathBuf::from(std::env::var_os("APPDATA").expect("private APPDATA required"));
    assert!(
        root.ends_with("daily-check-selfcheck"),
        "refusing non-fixture APPDATA"
    );
    let state = core::state_dir();
    let id = "123-456";
    let meeting = core::meetings_dir().join(id);
    fs::create_dir_all(&meeting).unwrap();
    fs::create_dir_all(state.join("checks")).unwrap();
    fs::create_dir_all(state.join("memory")).unwrap();
    fs::write(
        meeting.join("english-summary.json"),
        json!({"title":"Fixture","todo":[{"task":"Check shipment","owner":null}]}).to_string(),
    )
    .unwrap();
    assert!(list_checks().unwrap().is_empty());
    assert!(latest_check().unwrap().is_none());
    assert!(company_memory().unwrap().is_empty());
    let date = NaiveDate::from_ymd_opt(2026, 11, 6).unwrap();
    let content = CheckContent {
        findings: vec![],
        story: "Fixture".into(),
        memory_update: "Fixture memory".into(),
        summary: "One\nTwo\nThree".into(),
        ..CheckContent::default()
    };
    let report = DailyCheck {
        schema_version: 1,
        date,
        runner: "fixture".into(),
        prompt_bytes: 100,
        elapsed_seconds: 0.0,
        meeting_count: 1,
        task_count: 1,
        kb: KbSnapshot {
            revision: "fixture".into(),
            captured_at: "fixture".into(),
            path: root.join("kb"),
            stale: false,
        },
        warnings: vec![],
        memory_updated: true,
        content,
    };
    fs::write(
        state.join(format!("checks/{date}.json")),
        serde_json::to_vec(&report).unwrap(),
    )
    .unwrap();
    fs::write(state.join("checks/not-a-date.json"), "{}").unwrap();
    fs::write(state.join("memory/company.md"), "Fixture memory").unwrap();
    assert_eq!(list_checks().unwrap(), vec![date]);
    assert_eq!(load_check(date).unwrap().prompt_bytes, 100);
    assert_eq!(latest_check().unwrap().unwrap().date, date);
    assert_eq!(company_memory().unwrap(), "Fixture memory");
    let action = |kind, value| CheckAction {
        kind,
        meeting_id: id.into(),
        todo_index: 0,
        expected_task: "Check shipment".into(),
        value,
    };
    core::TaskStore::default().add_project("Delivery").unwrap();
    apply_action(&action(ActionKind::SetProject, json!("Delivery"))).unwrap();
    apply_action(&action(ActionKind::SetOwner, json!("Owner"))).unwrap();
    apply_action(&action(ActionKind::SetDue, json!("2026-11-08"))).unwrap();
    apply_action(&action(ActionKind::MarkDone, json!(true))).unwrap();
    apply_action(&action(
        ActionKind::Note,
        json!("Source-backed review note"),
    ))
    .unwrap();
    let edits = core::TaskStore::default().load_overrides().unwrap();
    let edit = edits.get(id, 0).unwrap();
    assert_eq!(edit.project.as_deref(), Some("Delivery"));
    assert_eq!(edit.owner.as_deref(), Some("Owner"));
    assert_eq!(edit.due.as_deref(), Some("2026-11-08"));
    let saved = core::StateStore::default().load(id).unwrap();
    assert!(saved.checked.contains("Check shipment"));
    assert_eq!(saved.messages[0].role, "alignment-note");
    assert!(apply_action(&action(ActionKind::SetDue, json!("tomorrow"))).is_err());
    assert!(apply_action(&action(ActionKind::MarkDone, json!("yes"))).is_err());
    let mut stale = action(ActionKind::SetOwner, json!("Wrong owner"));
    stale.expected_task = "Changed task".into();
    assert!(apply_action(&stale).is_err());
    stale.expected_task = "Check shipment".into();
    stale.todo_index = 99;
    assert!(apply_action(&stale).is_err());
    stale.meeting_id = "../escape".into();
    assert!(apply_action(&stale).is_err());
    println!("storage reads, all five actions, stale/index/path/value rejection: passed");
}
