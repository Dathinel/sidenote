//! The Ask answer label and the selectable summary, through the real pages. Each test runs in
//! its own process on an isolated data folder, with invented meeting text only.
use super::*;
use rune::{Env, Runtime, text::TextSystem};

const MEETING: &str = "1800000000000-1";
const CONTEXT: &str = "The team met to plan the beta launch.";
const POINT: &str = "Ship the beta on Friday.";

/// Relaunches the named test in a child process; true in the parent, which has then checked it.
fn in_parent(name: &str) -> bool {
    const FLAG: &str = "SIDENOTE_ASK_V3_TEST";
    if std::env::var(FLAG).ok().as_deref() == Some(name) {
        return false;
    }
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            &format!("ui::ask_switch_tests::{name}"),
            "--nocapture",
        ])
        .env(FLAG, name);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{name} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

/// The meeting page over one invented meeting that has a structured summary.
fn page() -> Runtime<State> {
    crate::test_paths::isolate();
    let folder = core::meetings_dir().join(MEETING);
    std::fs::create_dir_all(&folder).unwrap();
    let summary = serde_json::json!({
        "title": "Beta launch plan",
        "context": [CONTEXT],
        "topics": [{"title": "Launch", "points": [{"type": "decided", "text": POINT}]}],
    });
    std::fs::write(folder.join("english-summary.json"), summary.to_string()).unwrap();
    let mut rt = Runtime::with_text(
        State::new(),
        view,
        DEFAULT_SIZE,
        Env::default(),
        TextSystem::new(theme::fonts(), (theme::SANS, "Inter")),
    );
    rt.state.selected = Some(MEETING.into());
    rt.state.load_saved();
    rt.settle_all();
    rt.frame(0.0);
    rt
}

#[test]
fn a_context_line_and_a_point_can_be_selected_and_copied() {
    if in_parent("a_context_line_and_a_point_can_be_selected_and_copied") {
        return;
    }
    let mut rt = page();
    for (key, len, want) in [
        ("sec-context", 7, "Context"),
        ("ctx-0", CONTEXT.len(), CONTEXT),
        ("meeting-point-text-pt-0-0", POINT.len(), POINT),
    ] {
        rt.update(|_, tx| tx.select_text(key, 0..len));
        rt.frame(0.0);
        assert_eq!(rt.selected_text().as_deref(), Some(want), "{key}");
    }
}

#[test]
fn an_answer_names_the_helper_and_model_that_wrote_it_and_old_ones_still_load() {
    if in_parent("an_answer_names_the_helper_and_model_that_wrote_it_and_old_ones_still_load") {
        return;
    }
    let mut rt = page();
    let said = |runner: Option<&str>, model: Option<&str>, fallback: bool| Message {
        role: "assistant".into(),
        text: "Done.".into(),
        runner: runner.map(str::to_owned),
        model: model.map(str::to_owned),
        fallback,
        ..Default::default()
    };
    let old: Message =
        serde_json::from_str(r#"{"role":"assistant","text":"Older answer."}"#).unwrap();
    assert!(old.runner.is_none() && old.model.is_none() && !old.fallback);
    assert!(!serde_json::to_string(&old).unwrap().contains("runner"));
    let messages = vec![
        old,
        said(Some("Codex"), Some("gpt-6.1-sol"), false),
        said(Some("OpenCode"), None, false),
        said(Some("Claude Sonnet"), None, true),
    ];
    rt.update(|s, _| s.saved.messages = messages);
    rt.settle_all();
    rt.frame(0.0);
    let label = |i: usize| ask::answer_label(&rt.state, &rt.state.saved.messages[i]);
    assert_eq!(label(0), None);
    assert!(label(1).unwrap().starts_with("Codex"), "{:?}", label(1));
    assert!(
        label(2).unwrap().ends_with("provider default"),
        "{:?}",
        label(2)
    );
    assert!(label(3).unwrap().ends_with("fallback"), "{:?}", label(3));
    assert!(rt.find("answer-model-0").is_none());
    for i in 1..4 {
        assert!(
            rt.find(&format!("answer-model-{i}")).is_some(),
            "answer {i}"
        );
    }
}

#[test]
fn a_fallback_answer_is_saved_with_the_helper_that_wrote_it() {
    if in_parent("a_fallback_answer_is_saved_with_the_helper_that_wrote_it") {
        return;
    }
    let mut rt = page();
    let s = &mut rt.state;
    let mut run = AskRun::stand_in("Codex", &[], 0);
    run.model = Some("gpt-6.1-sol".into());
    run.fallback_model = Some("claude-sonnet-5-5".into());
    s.asks.insert(MEETING.into(), run);
    s.apply_event(Event::Progress {
        id: MEETING.into(),
        generation: 0,
        event: AskProgress::Fallback {
            runner: "Claude Sonnet".into(),
        },
    });
    let mut run = s.asks.remove(MEETING).unwrap();
    let (tx, rx) = mpsc::channel();
    tx.send(Ok("Here is the answer.".to_string())).unwrap();
    run.rx = rx;
    s.finish_ask(MEETING, run, None);
    let saved = s.saved.messages.last().unwrap();
    assert_eq!(saved.runner.as_deref(), Some("Claude Sonnet"));
    assert_eq!(saved.model.as_deref(), Some("claude-sonnet-5-5"));
    assert!(saved.fallback);
}
