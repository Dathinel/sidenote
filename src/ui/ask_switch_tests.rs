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

#[test]
fn the_model_picker_shows_on_a_meeting_the_live_screen_and_a_new_chat() {
    if in_parent("the_model_picker_shows_on_a_meeting_the_live_screen_and_a_new_chat") {
        return;
    }
    let mut rt = page();
    rt.update(|s, _| scene_agents(s, ALL_AGENTS));
    rt.frame(0.0);
    assert!(
        rt.state.selected.is_some() && rt.find("ask-model").is_some(),
        "meeting"
    );
    rt.update(|s, _| s.selected = None);
    rt.frame(0.0);
    assert!(rt.find("ask-model").is_some(), "new chat");
    rt.update(|s, _| scene_live(s, "", &[], false));
    rt.frame(0.0);
    assert!(
        rt.state.on_live() && rt.find("ask-model").is_some(),
        "live screen"
    );
}

#[test]
fn a_pick_applies_to_the_next_question_and_default_follows_settings() {
    if in_parent("a_pick_applies_to_the_next_question_and_default_follows_settings") {
        return;
    }
    let mut rt = page();
    rt.update(|s, _| scene_agents(s, ALL_AGENTS));
    let groups = ask_settings::picker_groups(&rt.state);
    assert!(
        groups.len() >= 2 && groups.iter().all(|g| !g.2.is_empty()),
        "{groups:?}"
    );
    let before = rt.state.ask_runner_name().map(str::to_owned);
    rt.update(|s, _| s.choose_ask_pick("Codex".into(), Some("gpt-test".into())));
    assert_eq!(rt.state.ask_runner_name(), Some("Codex"));
    assert_eq!(rt.state.ask_model().as_deref(), Some("gpt-test"));
    assert!(ask::picker_label(&rt.state).contains("Codex"));
    rt.update(|s, _| s.choose_ask_default());
    assert_eq!(rt.state.ask_runner_name().map(str::to_owned), before);
}

// The helper stand-in (`core::ask_session::STAND_IN`) only exists with debug assertions, so
// everything that installs it is compiled the same way. A release test build then has no way
// to reach a real helper and no unresolved symbol.
#[cfg(debug_assertions)]
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};

/// Opened by the test to let helper A answer.
#[cfg(debug_assertions)]
static GATE: AtomicBool = AtomicBool::new(false);
/// Set by helper A as soon as it is running: the test switches only after this.
#[cfg(debug_assertions)]
static ENTERED: AtomicBool = AtomicBool::new(false);
#[cfg(debug_assertions)]
static CALLS: std::sync::Mutex<Vec<(String, Option<String>)>> = std::sync::Mutex::new(Vec::new());
/// How long the fake helper and the test wait for each other.
#[cfg(debug_assertions)]
const WAIT: Duration = Duration::from_secs(20);

/// The fake helper: records what it was started with. Helper A (Codex) says it has started, then
/// holds its answer until the gate opens, so the test can switch while it is still answering.
#[cfg(debug_assertions)]
fn fake_helper(runner: &core::Runner, _prompt: &str) -> Result<String, String> {
    CALLS
        .lock()
        .unwrap()
        .push((runner.name.clone(), runner.model.clone()));
    if runner.name == "Codex" {
        ENTERED.store(true, SeqCst);
        let end = Instant::now() + WAIT;
        while !GATE.load(SeqCst) && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(20));
        }
        if !GATE.load(SeqCst) {
            return Err("the gate never opened".into());
        }
    }
    Ok(format!("answer from {}", runner.name))
}

#[cfg(debug_assertions)]
fn wait_until_entered() {
    let end = Instant::now() + WAIT;
    while !ENTERED.load(SeqCst) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(ENTERED.load(SeqCst), "helper A never started");
}

#[cfg(debug_assertions)]
fn first_model(s: &State, agent: &str) -> (String, Option<String>) {
    let group = ask_settings::picker_groups(s)
        .into_iter()
        .find(|g| g.0 == agent)
        .unwrap();
    (group.1, group.2.into_iter().find_map(|m| m.1))
}

/// Applies events until the answer for the open meeting has been saved.
#[cfg(debug_assertions)]
fn finish_answer(rt: &mut Runtime<State>) {
    while rt.state.asks.contains_key(MEETING) {
        let event = rt
            .state
            .events_rx
            .recv_timeout(Duration::from_secs(25))
            .expect("no answer");
        rt.update(|s, _| s.apply_event(event));
    }
}

#[cfg(debug_assertions)]
#[test]
fn switching_helper_mid_answer_labels_each_answer_and_runs_the_next_on_the_new_one() {
    if in_parent("switching_helper_mid_answer_labels_each_answer_and_runs_the_next_on_the_new_one")
    {
        return;
    }
    *core::ask_session::STAND_IN.lock().unwrap() = Some(fake_helper);
    let mut rt = page();
    rt.update(|s, _| scene_agents(s, ALL_AGENTS));
    let (a, a_model) = first_model(&rt.state, "Codex");
    let (b, b_model) = first_model(&rt.state, "OpenCode");
    rt.update(|s, _| s.choose_ask_pick(a.clone(), a_model.clone()));
    rt.update(|s, _| s.send("What do I need to do next?"));
    let run = &rt.state.asks[MEETING];
    assert_eq!((run.runner.as_str(), &run.model), (a.as_str(), &a_model));
    // Switch only once A has really started and is holding its answer.
    wait_until_entered();
    assert!(!GATE.load(SeqCst), "A must still be answering");
    assert_eq!(*CALLS.lock().unwrap(), vec![(a.clone(), a_model.clone())]);
    rt.update(|s, _| s.choose_ask_pick(b.clone(), b_model.clone()));
    assert!(ask::picker_label(&rt.state).starts_with("Next answer: "));
    assert_eq!(rt.state.asks[MEETING].runner, a);
    GATE.store(true, SeqCst);
    finish_answer(&mut rt);
    let first = rt.state.saved.messages.last().unwrap().clone();
    assert_eq!(first.text, format!("answer from {a}"));
    assert_eq!(
        (first.runner.as_deref(), &first.model),
        (Some(a.as_str()), &a_model)
    );
    assert!(
        ask::answer_label(&rt.state, &first)
            .unwrap()
            .starts_with("Codex")
    );
    rt.update(|s, _| s.send("And after that?"));
    assert_eq!(rt.state.asks[MEETING].runner, b);
    finish_answer(&mut rt);
    let second = rt.state.saved.messages.last().unwrap().clone();
    assert_eq!(second.text, format!("answer from {b}"));
    assert_eq!(
        (second.runner.as_deref(), &second.model),
        (Some(b.as_str()), &b_model)
    );
    assert!(
        ask::answer_label(&rt.state, &second)
            .unwrap()
            .starts_with("OpenCode")
    );
    assert!(rt.state.ask_settings.pick.is_none());
    let calls = CALLS.lock().unwrap().clone();
    assert_eq!(calls, vec![(a, a_model), (b, b_model)]);
}

#[cfg(debug_assertions)]
#[test]
fn a_one_question_pick_leaves_the_saved_default_alone_after_submit_and_restart() {
    if in_parent("a_one_question_pick_leaves_the_saved_default_alone_after_submit_and_restart") {
        return;
    }
    *core::ask_session::STAND_IN.lock().unwrap() = Some(fake_helper);
    GATE.store(true, SeqCst);
    let mut rt = page();
    rt.update(|s, _| scene_agents(s, ALL_AGENTS));
    // The Settings default is Claude Sonnet, saved the way the Settings control saves it.
    rt.update(|s, _| s.choose_normal_ask_runner("Claude Sonnet".into()));
    let saved = || core::last_used_runner().unwrap();
    assert_eq!(saved().as_deref(), Some("Claude Sonnet"));
    // Pick Codex for one question and send it through the real Ask path.
    let (codex, codex_model) = first_model(&rt.state, "Codex");
    rt.update(|s, _| s.choose_ask_pick(codex.clone(), codex_model.clone()));
    rt.update(|s, _| s.send("What do I need to do next?"));
    finish_answer(&mut rt);
    assert_eq!(*CALLS.lock().unwrap(), vec![(codex, codex_model)]);
    assert_eq!(rt.state.runner.as_deref(), Some("Claude Sonnet"));
    assert!(rt.state.ask_settings.pick.is_none());
    // After submit, and as a restart reads it, the default is still the Settings choice.
    assert_eq!(saved().as_deref(), Some("Claude Sonnet"));
    let restarted = default_runner(&rt.state.runners, &rt.state.runner_found, saved());
    assert_eq!(restarted.as_deref(), Some("Claude Sonnet"));
    // The next question with no pick goes to the default, not to Codex.
    rt.update(|s, _| s.send("And after that?"));
    assert_eq!(rt.state.asks[MEETING].runner, "Claude Sonnet");
    finish_answer(&mut rt);
    assert_eq!(saved().as_deref(), Some("Claude Sonnet"));
}
