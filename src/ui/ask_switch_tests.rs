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
    page_at(DEFAULT_SIZE)
}

fn page_at(size: (f32, f32)) -> Runtime<State> {
    page_with(size, || {})
}

/// `page_at`, with `stored` run after the meeting folder is written and before the app reads its
/// meetings, so a test can leave other records beside it.
fn page_with(size: (f32, f32), stored: impl FnOnce()) -> Runtime<State> {
    crate::test_paths::isolate();
    let folder = core::meetings_dir().join(MEETING);
    std::fs::create_dir_all(&folder).unwrap();
    let summary = serde_json::json!({
        "title": "Beta launch plan",
        "context": [CONTEXT],
        "topics": [{"title": "Launch", "points": [{"type": "decided", "text": POINT}]}],
    });
    std::fs::write(folder.join("english-summary.json"), summary.to_string()).unwrap();
    stored();
    let mut state = State::new();
    state.ai_notes.jobs = notes_status::read(&core::state_dir(), &state.meetings);
    let mut rt = Runtime::with_text(
        state,
        view,
        size,
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

fn page_with_cited_point() -> Runtime<State> {
    page_with(DEFAULT_SIZE, || {
        let folder = core::meetings_dir().join(MEETING);
        let summary = serde_json::json!({
            "title": "Beta launch plan",
            "topics": [{"title": "Launch", "points": [{
                "type": "decided", "text": POINT, "at": ["unknown", "00:27"]
            }]}],
        });
        std::fs::write(folder.join("english-summary.json"), summary.to_string()).unwrap();
        std::fs::write(
            folder.join("english-transcript.md"),
            "[00:00:06.000] Maya (English): Ready?\n[00:00:27.000] Tom (English): Ship the beta on Friday.\n",
        )
        .unwrap();
    })
}

#[test]
fn kb_copy_rows_keep_mono_for_paths_and_commands() {
    if in_parent("kb_copy_rows_keep_mono_for_paths_and_commands") {
        return;
    }
    const VALUES: [&str; 2] = ["docs/v1/api_keys.md", "git show HEAD:docs/v1/api_keys.md"];
    fn fixture(s: &State, _: &mut rune::Cx<State>) -> rune::El {
        let p = theme::palette(false);
        col().children(VALUES.into_iter().enumerate().flat_map(|(i, value)| {
            [
                check::source_row(s, value, format!("copy-{i}"), p),
                rune::text(value)
                    .font(roles::META.style().mono())
                    .self_start(),
            ]
        }))
    }
    crate::test_paths::isolate();
    let mut rt = Runtime::with_text(
        State::new(),
        fixture,
        (640.0, 240.0),
        Env::default(),
        TextSystem::new(theme::fonts(), (theme::SANS, "Inter")),
    );
    rt.frame(0.0);
    let elements = rt.elements();
    for value in VALUES {
        let widths = elements
            .iter()
            .filter(|element| element.text.as_deref() == Some(value))
            .map(|element| element.rect[2])
            .collect::<Vec<_>>();
        assert_eq!(widths.len(), 2, "{value}");
        assert!((widths[0] - widths[1]).abs() < 0.5, "{value}: {widths:?}");
    }
}

#[test]
fn point_timestamp_uses_its_own_figures_and_stays_clickable() {
    if in_parent("point_timestamp_uses_its_own_figures_and_stays_clickable") {
        return;
    }
    const PROSE: &str = "Review on 1 December";
    fn fixture(_: &State, _: &mut rune::Cx<State>) -> rune::El {
        let p = theme::palette(false);
        col().children(
            [
                ("plain", None),
                ("early", Some("11:11")),
                ("late", Some("22:22")),
                ("narrow", Some("11:11")),
            ]
            .into_iter()
            .map(|(key, at)| {
                let point = core::Point {
                    text: PROSE.into(),
                    at: at.into_iter().map(str::to_string).collect(),
                    ..Default::default()
                };
                let row = widgets::point_row(&point, key.into(), None, false, p);
                if key == "narrow" { row.w(150.0) } else { row }
            }),
        )
    }
    let state = page_with(DEFAULT_SIZE, || {
        std::fs::write(
            core::meetings_dir()
                .join(MEETING)
                .join("english-transcript.md"),
            "[00:11:11.000] Maya (English): Review the date.\n",
        )
        .unwrap();
    })
    .state;
    let mut rt = Runtime::with_text(
        state,
        fixture,
        (480.0, 240.0),
        Env::default(),
        TextSystem::new(theme::fonts(), (theme::SANS, "Inter")),
    );
    rt.frame(0.0);
    let elements = rt.elements();
    let prose = elements
        .iter()
        .filter(|element| element.text.as_deref() == Some(PROSE))
        .collect::<Vec<_>>();
    assert_eq!(
        prose.len(),
        4,
        "each point renders prose in its own element"
    );
    for element in &prose[..3] {
        assert!((element.rect[2] - prose[0].rect[2]).abs() < 0.5);
    }
    let time = |key| {
        elements
            .iter()
            .find(|e| e.key.as_deref() == Some(key))
            .unwrap()
            .rect
    };
    let early = time("early-time");
    let late = time("late-time");
    assert!((early[2] - late[2]).abs() < 0.5);
    assert!(early[0] >= prose[1].rect[0] + prose[1].rect[2]);
    assert!(early[1] < prose[1].rect[1] + prose[1].rect[3]);
    let narrow = time("narrow-time");
    assert!(narrow[1] >= prose[3].rect[1] + prose[3].rect[3]);
    assert!(narrow[0] + narrow[2] <= 150.0);
    tap(&mut rt, "early-time");
    assert_eq!(rt.state.highlight, Some(0));
    assert_eq!(rt.state.back.as_deref(), Some("early"));
}

#[test]
fn meeting_source_time_reveals_on_hover_and_focus() {
    if in_parent("meeting_source_time_reveals_on_hover_and_focus") {
        return;
    }
    let mut rt = page_with_cited_point();
    let source = "meeting-source-pt-0-0";
    let row = "meeting-point-pt-0-0";
    assert_eq!(rt.presented(source).unwrap().1, 0.0);
    let (x, y) = rt.find("meeting-point-text-pt-0-0").unwrap();
    rt.pointer_move(x, y);
    rt.frame(0.2);
    assert_source_revealed(&rt, source, row);
    rt.pointer_leave();
    rt.frame(0.2);
    assert_eq!(rt.presented(source).unwrap().1, 0.0);

    rt.update(|_, tx| tx.focus(source, true));
    rt.frame(0.2);
    assert_source_revealed(&rt, source, row);
}

#[test]
fn meeting_source_click_opens_the_cited_transcript_line() {
    if in_parent("meeting_source_click_opens_the_cited_transcript_line") {
        return;
    }
    let mut rt = page_with_cited_point();
    let (x, y) = rt.find("meeting-point-text-pt-0-0").unwrap();
    rt.pointer_move(x, y);
    rt.frame(0.2);
    tap(&mut rt, "meeting-source-pt-0-0");
    assert!(rt.state.transcript_open);
    assert_eq!(rt.state.highlight, Some(1));
    assert!(rt.find("line-1").is_some());
}

fn assert_source_revealed(rt: &Runtime<State>, source: &str, row: &str) {
    let elements = rt.elements();
    let time = elements
        .iter()
        .find(|e| e.key.as_deref() == Some(source))
        .unwrap();
    assert_eq!(time.opacity, 1.0);
    assert_eq!(time.label.as_deref(), Some("Open source at 0:27"));
    let row = elements
        .iter()
        .find(|e| e.key.as_deref() == Some(row))
        .unwrap();
    assert_eq!(row.hover, 1.0);
}

/// A summary with every point kind, in an order that is not the display order.
fn page_with_every_kind() -> Runtime<State> {
    page_with(DEFAULT_SIZE, || {
        let folder = core::meetings_dir().join(MEETING);
        let summary = serde_json::json!({
            "title": "Beta launch plan",
            "context": [CONTEXT, "Two questions stayed open."],
            "topics": [
                {"title": "Launch", "points": [
                    {"type": "fact", "text": "Copy is with Priya.", "at": ["00:27"]},
                    {"type": "open", "text": "Whether to ask for the work email later."},
                    {"type": "decided", "text": POINT},
                    {"type": "response", "text": "Priya will send it Thursday."},
                    {"type": "decided", "text": "Hold the review on Monday."},
                ]},
                {"title": "Survey", "points": [{"type": "fact", "text": "Four in ten skip it."}]},
            ],
            "verify": ["The sample is 48 people."],
        });
        std::fs::write(folder.join("english-summary.json"), summary.to_string()).unwrap();
    })
}

fn rect_of(rt: &Runtime<State>, key: &str) -> [f32; 4] {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("missing {key}"))
        .rect
}

#[test]
fn meeting_notes_body_follows_the_spacing_and_bullet_rules() {
    if in_parent("meeting_notes_body_follows_the_spacing_and_bullet_rules") {
        return;
    }
    let rt = page_with_every_kind();
    let row = |i: usize| rect_of(&rt, &format!("meeting-point-pt-0-{i}"));
    // Bullets sit 27 px apart: a 23 px line plus 2 px of padding above and below.
    let mut ys: Vec<f32> = (0..5).map(|i| row(i)[1]).collect();
    ys.sort_by(f32::total_cmp);
    for i in 1..5 {
        assert_eq!(ys[i] - ys[i - 1], 27.0, "row {i}");
        assert_eq!(row(i)[3], 27.0, "row {i}");
    }
    // Every heading is one role: the same height, 8 px above its first row, 28 px below the
    // last row of the section before.
    let context = rect_of(&rt, "sec-context");
    let launch = rect_of(&rt, "topic-1");
    let survey = rect_of(&rt, "topic-2");
    let verify = rect_of(&rt, "sec-verify");
    assert!(
        [launch[3], survey[3], verify[3]]
            .iter()
            .all(|h| *h == context[3])
    );
    assert_eq!(ys[0] - (launch[1] + launch[3]), 8.0);
    assert_eq!(
        launch[1] - (rect_of(&rt, "ctx-0")[1] + rect_of(&rt, "ctx-0")[3]),
        28.0
    );
    assert_eq!(survey[1] - (ys[4] + 27.0), 28.0);
    // Context is one paragraph: both lines are in ctx-0, and no second block exists.
    assert!(rt.find("ctx-1").is_none());
    let context = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("ctx-0"))
        .and_then(|e| e.text);
    assert_eq!(
        context.as_deref(),
        Some(&*format!("{CONTEXT} Two questions stayed open."))
    );
    // Bullets are 5 px, 11 px in from the row edge; the text starts 26 px in (46 for a reply).
    // A decision has an ink dot, an open point a ring (no fill), the rest a grey dot.
    let elements = rt.elements();
    let by_key = |key: &str| {
        elements
            .iter()
            .find(|e| e.key.as_deref() == Some(key))
            .unwrap_or_else(|| panic!("missing {key}"))
    };
    for (i, text_x) in [(0, 26.0), (1, 26.0), (2, 26.0), (3, 46.0), (4, 26.0)] {
        let (row, dot) = (row(i), by_key(&format!("meeting-bullet-pt-0-{i}")));
        let text = rect_of(&rt, &format!("meeting-point-text-pt-0-{i}"));
        assert_eq!((dot.rect[2], dot.rect[3]), (5.0, 5.0), "dot {i}");
        let dx = if i == 3 { 20.0 } else { 0.0 };
        assert_eq!(dot.rect[0] - row[0], 11.0 + dx, "dot x {i}");
        assert_eq!(text[0] - row[0], text_x, "text x {i}");
        assert_eq!(
            dot.fill.a == 0.0,
            i == 1,
            "only the open point is a ring: {i}"
        );
        // First line is 23 px tall, so its centre is 2 + 11.5 below the row top.
        assert_eq!(dot.rect[1] + 2.5 - row[1], 13.5, "dot y {i}");
    }
    let fill = |i: usize| by_key(&format!("meeting-bullet-pt-0-{i}")).fill;
    assert_eq!(fill(2), fill(4), "decision dots share one colour");
    assert_eq!(fill(0), fill(3), "fact and reply dots share one colour");
    assert_ne!(fill(0), fill(2), "a decision dot differs from a fact dot");
}

#[test]
fn meeting_notes_lead_each_topic_with_its_decisions() {
    if in_parent("meeting_notes_lead_each_topic_with_its_decisions") {
        return;
    }
    let rt = page_with_every_kind();
    // Summary order is fact, open, decided, response, decided: shown as the two decisions
    // (summary rows 2 and 4), then the fact and the reply (0, 3), then the open point (1).
    let mut shown: Vec<(f32, usize)> = (0..5)
        .map(|i| (rect_of(&rt, &format!("meeting-point-pt-0-{i}"))[1], i))
        .collect();
    shown.sort_by(|a, b| a.0.total_cmp(&b.0));
    let shown: Vec<usize> = shown.into_iter().map(|(_, i)| i).collect();
    assert_eq!(shown, [2, 4, 0, 3, 1]);
    // The bold lead block is the decisions only.
    let elements = rt.elements();
    let ink = |i: usize| {
        let key = format!("meeting-point-text-pt-0-{i}");
        elements
            .iter()
            .find(|e| e.key.as_deref() == Some(&key))
            .unwrap()
            .text_color
    };
    assert_eq!(ink(2), ink(4));
    for plain in [0, 1, 3] {
        assert_ne!(
            ink(plain),
            ink(2),
            "only decisions use the ink colour: {plain}"
        );
    }
    // A topic with no decisions keeps the summary's order.
    assert!(rect_of(&rt, "meeting-point-pt-1-0")[1] > rect_of(&rt, "topic-2")[1]);
}

#[test]
fn meeting_source_time_works_by_keyboard_in_the_new_rows() {
    if in_parent("meeting_source_time_works_by_keyboard_in_the_new_rows") {
        return;
    }
    let mut rt = page_with_cited_point();
    // The pill is hidden, 21 px high, 6 px in from the row's right edge, and reachable by Tab.
    let (row, pill) = (
        rect_of(&rt, "meeting-point-pt-0-0"),
        rect_of(&rt, "meeting-source-pt-0-0"),
    );
    assert_eq!(pill[3], 21.0);
    assert_eq!(row[0] + row[2] - (pill[0] + pill[2]), 6.0);
    rt.update(|_, tx| tx.focus("meeting-source-pt-0-0", true));
    rt.frame(0.2);
    rt.key(KeyEvent {
        key: Key::Text(" ".into()),
        mods: rune::Mods::default(),
    });
    rt.frame(0.2);
    assert!(rt.state.transcript_open);
    assert_eq!(rt.state.highlight, Some(1));
}

#[test]
fn meeting_model_hint_reveals_on_hover_and_keyboard_focus() {
    if in_parent("meeting_model_hint_reveals_on_hover_and_keyboard_focus") {
        return;
    }
    let mut rt = page();
    let hint = "help-hint-model";
    let tip = "help-hint-model-tip";
    assert_eq!(rt.presented(tip).unwrap().1, 0.0);
    let (x, y) = rt.find(hint).unwrap();
    rt.pointer_move(x, y);
    rt.frame(0.2);
    assert_eq!(rt.presented(tip).unwrap().1, 1.0);
    rt.pointer_leave();
    rt.frame(0.2);
    assert_eq!(rt.presented(tip).unwrap().1, 0.0);
    rt.update(|_, tx| tx.focus(hint, true));
    rt.frame(0.2);
    assert_eq!(rt.presented(tip).unwrap().1, 1.0);
}

#[test]
fn meeting_meta_facts_and_popover_work_by_keyboard() {
    if in_parent("meeting_meta_facts_and_popover_work_by_keyboard") {
        return;
    }
    assert!(page().find("meeting-facts").is_none());
    let start = 1_793_955_600_000_i64;
    let mut rt = page_with((390.0, 900.0), || {
        let folder = core::meetings_dir().join(MEETING);
        let state = core::state_dir();
        std::fs::create_dir_all(&state).unwrap();
        for (path, value) in [
            (
                folder.join("manifest.json"),
                serde_json::json!({"status":"complete","created_at_ms":start,"duration_ms":1320000}),
            ),
            (
                folder.join("english-summary.json"),
                serde_json::json!({"title":"Beta launch plan","context":[CONTEXT],"people":["Maya","Tom"]}),
            ),
            (
                folder.join("tasks.json"),
                serde_json::json!({"meeting_project":"Atlas 2"}),
            ),
            (
                state.join("config.json"),
                serde_json::json!({"version":1,"features":{"projects":true}}),
            ),
            (
                state.join("projects.json"),
                serde_json::json!({"projects":["Atlas 2"]}),
            ),
        ] {
            std::fs::write(path, value.to_string()).unwrap();
        }
    });
    let project = format!("meeting-project-{MEETING}");
    let date = chrono::DateTime::from_timestamp_millis(start)
        .unwrap()
        .with_timezone(&chrono::Local);
    let facts = format!("Date: {}. Duration: 22 min", date.format("%-d %b"));
    rt.update(|s, tx| {
        let deadline = Instant::now() + Duration::from_secs(2);
        while s.areas.refresh_pending {
            let event = s
                .events_rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("area worker finished");
            s.apply_event(event);
        }
        tx.focus("segment0", true);
    });
    rt.frame(0.0);
    let press = |rt: &mut Runtime<State>, key| {
        rt.key(KeyEvent {
            key,
            mods: rune::Mods::default(),
        });
        rt.frame(0.2);
    };
    for (key, name) in [
        ("meeting-facts", facts.as_str()),
        ("meeting-people", "2 people"),
        (&project, "Suggested project: Atlas 2. Change it"),
        ("meeting-status", "Transcript status: English only"),
    ] {
        press(&mut rt, Key::Tab);
        let item = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key))
            .unwrap_or_else(|| panic!("missing {key}"));
        assert!(item.focusable && item.focused, "{key}");
        assert!(item.focus_visible, "{key}");
        assert_eq!(item.label.as_deref(), Some(name), "{key}");
        assert_eq!(item.rect[3], 28.0, "{key}");
        assert!(
            item.rect[0] >= 0.0 && item.rect[0] + item.rect[2] <= 390.0,
            "{key}"
        );
    }
    let focused = |rt: &Runtime<State>, key| {
        rt.elements()
            .iter()
            .any(|e| e.focused && e.key.as_deref() == Some(key))
    };
    press(&mut rt, Key::Text(" ".into()));
    assert!(rt.find("meeting-english-only").is_some());
    assert!(focused(&rt, "meeting-status-settings"));
    press(&mut rt, Key::Escape);
    assert!(rt.find("meeting-english-only").is_none());
    assert!(focused(&rt, "meeting-status"));
}

/// Three tasks and two open questions: one task with a detail, one with a guessed date and one
/// with neither owner nor date; one question waits on someone.
fn page_with_tasks(size: (f32, f32)) -> Runtime<State> {
    let mut rt = page_with(size, || {
        let folder = core::meetings_dir().join(MEETING);
        let summary = serde_json::json!({
            "title": "Beta launch plan",
            "todo": [
                {"task": "Send the survey results to the team", "title": "Send the survey results",
                 "description": "Send the survey results to the team",
                 "owner": "Maya", "due": "2026-11-27"},
                {"task": "Share the pricing page copy", "title": "Share the pricing page copy",
                 "owner": "Priya", "due": "2026-12-01", "due_default": true},
                {"task": "Book five customer tests", "title": "Book five customer tests"},
            ],
            "to_settle": [
                {"question": "Should the work email move to the end of sign-up?", "waits_on": "Priya"},
                {"question": "Who owns the survey follow-up?"},
            ],
        });
        std::fs::write(folder.join("english-summary.json"), summary.to_string()).unwrap();
        let state = core::state_dir();
        std::fs::create_dir_all(&state).unwrap();
        let config = serde_json::json!({"version":1,"features":{"tasks":true}});
        std::fs::write(state.join("config.json"), config.to_string()).unwrap();
    });
    // The tasks come from the area worker's snapshot.
    rt.update(|s, _| {
        s.refresh_areas();
        let deadline = Instant::now() + Duration::from_secs(2);
        while s.areas.refresh_pending {
            let event = s
                .events_rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("area worker finished");
            s.apply_event(event);
        }
    });
    rt.settle_all();
    rt.frame(0.0);
    rt
}

fn element(rt: &Runtime<State>, key: &str) -> rune::ElementInfo {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("missing {key}"))
}

#[test]
fn meeting_tasks_column_is_plain_rows_that_keep_every_fact() {
    if in_parent("meeting_tasks_column_is_plain_rows_that_keep_every_fact") {
        return;
    }
    let rt = page_with_tasks(DEFAULT_SIZE);
    // No card: the lists have neither a rounded box nor a fill.
    for key in ["meeting-tasks", "meeting-questions"] {
        let list = element(&rt, key);
        assert_eq!((list.radius, list.fill.a), (0.0, 0.0), "{key}");
    }
    // Each row: 8 all round, a 16 px check circle at x 8 (1 below the padding), the title at 34.
    let row = |i: usize| element(&rt, &format!("meeting-task-{i}")).rect;
    for i in 0..3 {
        let tick = element(&rt, &format!("meeting-tick-{i}")).rect;
        let title = element(&rt, &format!("meeting-task-open-{i}")).rect;
        assert_eq!((tick[2], tick[3]), (16.0, 16.0), "check circle {i}");
        assert_eq!(
            (tick[0] - row(i)[0], tick[1] - row(i)[1]),
            (8.0, 9.0),
            "tick {i}"
        );
        assert_eq!(
            (title[0] - row(i)[0], title[1] - row(i)[1]),
            (34.0, 8.0),
            "title {i}"
        );
        // The text column is 300 wide: the row is 316 with 8 of padding each side.
        assert_eq!(row(i)[2], 316.0, "row width {i}");
    }
    // 4 px from the label to the first row, 28 between the lists.
    let label = element(&rt, "meeting-task-count").rect;
    assert_eq!(label[3], 22.0);
    assert_eq!(row(0)[1] - (label[1] + label[3]), 4.0);
    let questions = element(&rt, "meeting-questions").rect;
    let tasks = element(&rt, "meeting-tasks").rect;
    assert_eq!(questions[1] - 4.0 - 22.0 - (tasks[1] + tasks[3]), 28.0);
    assert_eq!(element(&rt, "meeting-question-0").rect[0], row(0)[0]);
    // Every fact is still on the page.
    let elements = rt.elements();
    let texts: Vec<_> = elements.iter().filter_map(|e| e.text.as_deref()).collect();
    for want in [
        "Send the survey results",
        "Send the survey results to the team",
        "Maya \u{b7} 27 Nov",
        "Share the pricing page copy",
        "Priya \u{b7} 1 Dec \u{b7} guessed",
        "Book five customer tests",
        "No owner \u{b7} No date",
        "Should the work email move to the end of sign-up?",
        "Waiting on Priya",
        "Who owns the survey follow-up?",
    ] {
        assert!(texts.contains(&want), "missing {want}");
    }
}

#[test]
fn meeting_tasks_column_fits_a_narrow_window() {
    if in_parent("meeting_tasks_column_fits_a_narrow_window") {
        return;
    }
    let rt = page_with_tasks((390.0, 900.0));
    for i in 0..3 {
        let row = element(&rt, &format!("meeting-task-{i}")).rect;
        assert!(
            row[0] >= 0.0 && row[0] + row[2] <= 390.0,
            "row {i}: {row:?}"
        );
        // The check circle sits where the notes text starts.
        let tick = element(&rt, &format!("meeting-tick-{i}")).rect;
        assert_eq!(tick[0], element(&rt, "meeting-view").rect[0], "tick {i}");
    }
}

fn wait_for_tick(rt: &mut Runtime<State>, index: usize, done: bool) {
    rt.update(|s, _| {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !s
            .tasks
            .all
            .iter()
            .any(|t| t.index == index && t.done == done)
        {
            let event = s
                .events_rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("tick saved");
            s.apply_event(event);
        }
    });
    rt.frame(0.2);
}

#[test]
fn meeting_task_rows_work_by_keyboard_and_checking_one_saves() {
    if in_parent("meeting_task_rows_work_by_keyboard_and_checking_one_saves") {
        return;
    }
    let mut rt = page_with_tasks(DEFAULT_SIZE);
    rt.update(|_, tx| tx.focus("meeting-tick-0", true));
    rt.frame(0.0);
    let tick = element(&rt, "meeting-tick-0");
    assert!(tick.focusable && tick.focused && tick.focus_visible);
    assert_eq!(tick.role, "checkbox");
    assert_eq!(
        tick.label.as_deref(),
        Some("Mark Send the survey results done")
    );
    rt.key(KeyEvent {
        key: Key::Tab,
        mods: rune::Mods::default(),
    });
    rt.frame(0.2);
    let open = element(&rt, "meeting-task-open-0");
    assert!(open.focusable && open.focused && open.focus_visible);
    assert_eq!(open.role, "button");
    assert_eq!(open.text.as_deref(), Some("Send the survey results"));
    // Space on the check circle saves the tick; a click takes it off again.
    rt.update(|_, tx| tx.focus("meeting-tick-0", true));
    rt.frame(0.0);
    rt.key(KeyEvent {
        key: Key::Text(" ".into()),
        mods: rune::Mods::default(),
    });
    rt.frame(0.2);
    wait_for_tick(&mut rt, 0, true);
    assert_eq!(
        element(&rt, "meeting-tick-0").label.as_deref(),
        Some("Mark Send the survey results open")
    );
    tap(&mut rt, "meeting-tick-0");
    wait_for_tick(&mut rt, 0, false);
    assert_eq!(
        element(&rt, "meeting-tick-0").label.as_deref(),
        Some("Mark Send the survey results done")
    );
}

#[test]
fn meeting_tasks_label_names_the_sides_only_when_they_will_has_tasks() {
    if in_parent("meeting_tasks_label_names_the_sides_only_when_they_will_has_tasks") {
        return;
    }
    let mut rt = page_with_tasks(DEFAULT_SIZE);
    let inside = |rt: &Runtime<State>, key: &str| -> Vec<String> {
        let root = element(rt, key).path;
        rt.elements()
            .into_iter()
            .filter(|e| e.path.starts_with(&format!("{root}/")))
            .filter_map(|e| e.text)
            .collect()
    };
    // Everyone is on our side: one label, "Tasks 3", and no side labels.
    assert_eq!(inside(&rt, "meeting-task-count")[..2], ["Tasks", "3"]);
    assert!(rt.find("meeting-ours-task-count").is_none());
    assert!(rt.find("meeting-external-task-count").is_none());
    // Priya owns one task and is on the other side: "Tasks 3" over "Ours 2" and "They will 1".
    rt.update(|s, _| {
        s.people
            .prefs
            .people_sides
            .insert(core::people::key("Priya"), core::people::Side::External);
    });
    rt.settle_all();
    rt.frame(0.0);
    assert_eq!(inside(&rt, "meeting-task-count")[..2], ["Tasks", "3"]);
    assert_eq!(inside(&rt, "meeting-ours-task-count"), ["Ours", "2"]);
    assert_eq!(
        inside(&rt, "meeting-external-task-count"),
        ["They will", "1"]
    );
    // The first side label sits 4 under the Tasks label's row; 28 between the two lists.
    let (tasks, ours, theirs) = (
        element(&rt, "meeting-task-count").rect,
        element(&rt, "meeting-ours-task-count").rect,
        element(&rt, "meeting-external-task-count").rect,
    );
    let list = element(&rt, "meeting-tasks").rect;
    assert_eq!(ours[1] - (tasks[1] + tasks[3]), 4.0);
    assert_eq!(
        theirs[1] - (list[1] + list[3]),
        28.0,
        "theirs {theirs:?}, list {list:?}"
    );
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
    // Asked for a model, the helper reported none: the label must not claim the request.
    let requested_only = Message {
        requested_model: Some("gpt-6.1-sol".into()),
        ..said(Some("Codex"), None, false)
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
        requested_only,
    ];
    rt.update(|s, _| s.saved.messages = messages);
    rt.settle_all();
    rt.frame(0.0);
    let label = |i: usize| ask::answer_label(&rt.state, &rt.state.saved.messages[i]);
    assert_eq!(label(0), None);
    assert!(label(1).unwrap().starts_with("Codex"), "{:?}", label(1));
    assert_eq!(label(2).as_deref(), Some("OpenCode (model not reported)"));
    assert_eq!(
        label(3).as_deref(),
        Some("Claude Code (model not reported) \u{b7} fallback")
    );
    assert_eq!(label(4).as_deref(), Some("Codex (model not reported)"));
    assert!(rt.find("answer-model-0").is_none());
    for i in 1..5 {
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
    run.requested_model = Some("gpt-6.1-sol".into());
    run.fallback_model = Some("claude-sonnet-5-5".into());
    s.asks.insert(MEETING.into(), run);
    let progress = |s: &mut State, event| {
        s.apply_event(Event::Progress {
            id: MEETING.into(),
            generation: 0,
            event,
        })
    };
    progress(
        s,
        AskProgress::Model {
            model: "gpt-6.1-sol-reported".into(),
        },
    );
    assert_eq!(
        s.asks[MEETING].reported_model.as_deref(),
        Some("gpt-6.1-sol-reported")
    );
    progress(
        s,
        AskProgress::Fallback {
            runner: "Claude Sonnet".into(),
        },
    );
    // The first helper's report does not carry over, and the request is now the fallback's.
    assert_eq!(s.asks[MEETING].reported_model, None);
    assert_eq!(
        s.asks[MEETING].requested_model.as_deref(),
        Some("claude-sonnet-5-5")
    );
    progress(
        s,
        AskProgress::Model {
            model: "claude-sonnet-5-5-20261101".into(),
        },
    );
    let mut run = s.asks.remove(MEETING).unwrap();
    let (tx, rx) = mpsc::channel();
    tx.send(Ok("Here is the answer.".to_string())).unwrap();
    run.rx = rx;
    s.finish_ask(MEETING, run, None);
    let saved = s.saved.messages.last().unwrap();
    assert_eq!(saved.runner.as_deref(), Some("Claude Sonnet"));
    assert_eq!(saved.model.as_deref(), Some("claude-sonnet-5-5-20261101"));
    assert_eq!(saved.requested_model.as_deref(), Some("claude-sonnet-5-5"));
    assert!(saved.fallback);
}

#[test]
fn a_connected_endpoint_answer_names_the_model_it_reported_or_says_it_did_not() {
    if in_parent("a_connected_endpoint_answer_names_the_model_it_reported_or_says_it_did_not") {
        return;
    }
    let mut rt = page();
    let s = &mut rt.state;
    for (reported, want) in [
        (
            Some("served-model-1"),
            "Chat completions \u{b7} served-model-1",
        ),
        (None, "Chat completions (model not reported)"),
    ] {
        let mut run = AskRun::stand_in(core::provider::NAME, &[], 0);
        run.requested_model = Some("asked-model".into());
        s.asks.insert(MEETING.into(), run);
        if let Some(model) = reported {
            s.apply_event(Event::Progress {
                id: MEETING.into(),
                generation: 0,
                event: AskProgress::Model {
                    model: model.into(),
                },
            });
        }
        let mut run = s.asks.remove(MEETING).unwrap();
        let (tx, rx) = mpsc::channel();
        tx.send(Ok("An answer.".to_string())).unwrap();
        run.rx = rx;
        s.finish_ask(MEETING, run, None);
        let saved = s.saved.messages.last().unwrap();
        assert_eq!(saved.runner.as_deref(), Some(core::provider::NAME));
        assert_eq!(saved.requested_model.as_deref(), Some("asked-model"));
        assert_eq!(saved.model.as_deref(), reported);
        assert_eq!(ask::answer_label(s, saved).as_deref(), Some(want));
    }
}

/// At 390 wide the Ask panel keeps the page's 16 point gutter, Hide and Clear keep their own
/// room inside it, the caption shrinks to what is left, and the page fades into the bar rather
/// than being cut off at its edge.
#[test]
fn at_390_wide_the_ask_header_keeps_clear_off_the_edge_and_the_page_fades_out() {
    if in_parent("at_390_wide_the_ask_header_keeps_clear_off_the_edge_and_the_page_fades_out") {
        return;
    }
    let mut rt = page_at((390.0, 844.0));
    let said = |role: &str, text: &str| Message {
        role: role.into(),
        text: text.into(),
        ..Default::default()
    };
    rt.update(|s, _| {
        s.saved.messages = vec![
            said("user", "What did we decide about the launch date?"),
            said("assistant", "The beta ships on Friday."),
            said("user", "And who owns the follow up?"),
            said("assistant", "Maya will send the draft."),
        ]
    });
    // The panel rises in on its first frame; let that finish so the rects are the resting ones.
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
    let rect =
        |rt: &Runtime<State>, key: &str| rt.presented(key).unwrap_or_else(|| panic!("{key}")).0;
    let (overlay, thread) = (rect(&rt, "ask-overlay"), rect(&rt, "ask-thread"));
    assert_eq!(
        (
            thread[0] - overlay[0],
            overlay[0] + overlay[2] - (thread[0] + thread[2])
        ),
        (16.0, 16.0),
        "the panel keeps the 16 px gutter on both sides"
    );
    let (caption, hide, clear) = (
        rect(&rt, "ask-caption"),
        rect(&rt, "ask-toggle"),
        rect(&rt, "ask-clear"),
    );
    let panel_right = thread[0] + thread[2];
    assert!(
        clear[0] + clear[2] <= panel_right - 4.0,
        "Clear touches the edge"
    );
    assert!(
        caption[0] + caption[2] <= hide[0] && hide[0] + hide[2] <= clear[0],
        "the caption runs under Hide or Clear"
    );
    let (scroll, fade) = (
        rect(&rt, "meeting-scroll"),
        rect(&rt, "meeting-scroll-fade"),
    );
    assert_eq!(
        fade[1] + fade[3],
        scroll[1] + scroll[3],
        "fade sits on the cut edge"
    );
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

/// Makes the fake Codex fail at once, as an agent that is not installed does.
#[cfg(debug_assertions)]
static CODEX_FAILS: AtomicBool = AtomicBool::new(false);
/// What the fake Claude Sonnet reports as the model that answered.
#[cfg(debug_assertions)]
const SONNET_REPORTS: &str = "claude-sonnet-5-5-20261101";

/// The fake helper: records what it was started with and reports a model the way a real one
/// would: Codex a resolved name that differs from the request, Claude Sonnet a dated name,
/// OpenCode none. Helper A (Codex) says it has started, then holds its answer until the gate
/// opens, so the test can switch while it is still answering.
#[cfg(debug_assertions)]
fn fake_helper(
    runner: &core::Runner,
    _prompt: &str,
    progress: &dyn Fn(AskProgress),
) -> Result<String, String> {
    CALLS
        .lock()
        .unwrap()
        .push((runner.name.clone(), runner.model.clone()));
    if runner.name == "Codex" && CODEX_FAILS.load(SeqCst) {
        return Err("Codex is not available".into());
    }
    match runner.name.as_str() {
        "Codex" => progress(AskProgress::Model {
            model: format!("{}-resolved", runner.model.clone().unwrap_or_default()),
        }),
        "Claude Sonnet" => progress(AskProgress::Model {
            model: SONNET_REPORTS.into(),
        }),
        _ => {}
    }
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
    // Codex reported a resolved name that differs from the request: the answer keeps both.
    let resolved = format!("{}-resolved", a_model.clone().unwrap());
    assert_eq!(
        (
            first.runner.as_deref(),
            first.model.as_deref(),
            &first.requested_model
        ),
        (Some(a.as_str()), Some(resolved.as_str()), &a_model)
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
    // OpenCode reported no model: the request is kept, and the label does not claim it.
    assert_eq!(
        (
            second.runner.as_deref(),
            &second.model,
            &second.requested_model
        ),
        (Some(b.as_str()), &None, &b_model)
    );
    assert_eq!(
        ask::answer_label(&rt.state, &second).as_deref(),
        Some("OpenCode (model not reported)")
    );
    assert!(rt.state.ask_settings.pick.is_none());
    // Reload the saved thread from disk: both answers and their labels are unchanged.
    let before = rt.state.saved.messages.clone();
    rt.update(|s, _| s.load_saved());
    assert_eq!(rt.state.saved.messages, before);
    let labels: Vec<_> = rt
        .state
        .saved
        .messages
        .iter()
        .filter(|m| m.role == "assistant")
        .map(|m| ask::answer_label(&rt.state, m))
        .collect();
    assert!(labels[0].as_ref().unwrap().starts_with("Codex"));
    assert_eq!(labels[1].as_deref(), Some("OpenCode (model not reported)"));
    let calls = CALLS.lock().unwrap().clone();
    assert_eq!(calls, vec![(a, a_model), (b, b_model)]);
}

#[cfg(debug_assertions)]
#[test]
fn a_fallback_answer_through_the_real_ask_names_the_helper_that_answered_and_survives_reload() {
    if in_parent(
        "a_fallback_answer_through_the_real_ask_names_the_helper_that_answered_and_survives_reload",
    ) {
        return;
    }
    *core::ask_session::STAND_IN.lock().unwrap() = Some(fake_helper);
    CODEX_FAILS.store(true, SeqCst);
    let mut rt = page();
    rt.update(|s, _| scene_agents(s, ALL_AGENTS));
    // A live meeting asks Codex first and falls back to the normal helper when it fails.
    rt.update(|s, _| {
        s.ask_settings.recording = true;
        s.meeting_agent_transition();
    });
    assert_eq!(rt.state.ask_runner_name(), Some("Codex"));
    let fallback_request = rt.state.normal_ask_model();
    rt.update(|s, _| s.send("What did we decide?"));
    finish_answer(&mut rt);
    let calls: Vec<_> = CALLS.lock().unwrap().iter().map(|c| c.0.clone()).collect();
    assert_eq!(calls, ["Codex", "Claude Sonnet"]);
    let answer = rt.state.saved.messages.last().unwrap().clone();
    assert_eq!(answer.text, "answer from Claude Sonnet");
    assert_eq!(answer.runner.as_deref(), Some("Claude Sonnet"));
    assert_eq!(answer.model.as_deref(), Some(SONNET_REPORTS));
    assert_eq!(answer.requested_model, fallback_request);
    assert!(answer.fallback);
    // After a reload from disk the label still names the helper that answered.
    rt.update(|s, _| s.load_saved());
    let saved = rt.state.saved.messages.last().unwrap();
    assert_eq!(saved, &answer);
    let label = ask::answer_label(&rt.state, saved).unwrap();
    assert!(
        label.starts_with("Claude Code") && label.ends_with("fallback"),
        "{label}"
    );
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

/// The Ask pill and the suggestion chip start where the notes column starts. The thread above
/// keeps the 16 point gutter the 390 wide test checks; the bar does not follow it.
fn the_bar_starts_at_the_notes_column(name: &str, size: (f32, f32)) {
    if in_parent(name) {
        return;
    }
    let mut rt = page_at(size);
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
    let rect = |key: &str| {
        rt.presented(key)
            .unwrap_or_else(|| panic!("{key} at {size:?}"))
            .0
    };
    let notes = rect("meeting-view");
    let (chip, pill) = (rect("suggest-0"), rect("ask-pill"));
    assert_eq!(chip[0], notes[0], "the chip starts at the notes column");
    assert_eq!(pill[0], notes[0], "the pill starts at the notes column");
}

#[test]
fn the_ask_bar_starts_at_the_notes_column_at_390_wide() {
    the_bar_starts_at_the_notes_column(
        "the_ask_bar_starts_at_the_notes_column_at_390_wide",
        (390.0, 844.0),
    );
}

#[test]
fn the_ask_bar_starts_at_the_notes_column_at_the_default_width() {
    the_bar_starts_at_the_notes_column(
        "the_ask_bar_starts_at_the_notes_column_at_the_default_width",
        DEFAULT_SIZE,
    );
}

/// Writes a summary with this `made_by` record into `folder`.
fn stamped_summary(folder: &std::path::Path, made_by: Option<serde_json::Value>) {
    let path = folder.join("english-summary.json");
    let mut summary: serde_json::Value = match std::fs::read(&path) {
        Ok(raw) => serde_json::from_slice(&raw).unwrap(),
        Err(_) => serde_json::json!({"title": "Beta launch plan", "context": [CONTEXT]}),
    };
    if let Some(by) = made_by {
        summary["made_by"] = by;
    }
    std::fs::create_dir_all(folder).unwrap();
    std::fs::write(path, summary.to_string()).unwrap();
}

/// Whether the saved meeting's Notes view says the transcript was English-only.
fn shows_english_only_note(made_by: Option<serde_json::Value>, stored: impl FnOnce()) -> bool {
    let mut rt = page_with(DEFAULT_SIZE, || {
        stamped_summary(&core::meetings_dir().join(MEETING), made_by);
        stored();
    });
    if rt.find("meeting-status").is_some() {
        tap(&mut rt, "meeting-status");
    }
    rt.find("meeting-english-only").is_some()
}

fn tap(rt: &mut Runtime<State>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing {key}"));
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.2);
}

fn page_with_language_fallback() -> Runtime<State> {
    page_with((390.0, 800.0), || {
        let dir = core::state_dir().join("local/notes-jobs");
        std::fs::create_dir_all(&dir).unwrap();
        let job = serde_json::json!({
            "state": "done", "fallback": false, "tasks_failed": true,
            "transcript_revision": "a".repeat(64),
            "routing_fallbacks": [{"reason": {"cpu_language_models_missing": {"ready": 2}}}],
        });
        std::fs::write(dir.join(format!("{MEETING}.json")), job.to_string()).unwrap();
    })
}

#[test]
fn meeting_status_keeps_language_facts_and_the_settings_link_inside_the_popover() {
    if in_parent("meeting_status_keeps_language_facts_and_the_settings_link_inside_the_popover") {
        return;
    }
    let mut rt = page_with_language_fallback();
    let check = |rt: &Runtime<State>, open| {
        let elements = rt.elements();
        let texts: Vec<_> = elements.iter().filter_map(|e| e.text.as_deref()).collect();
        assert_eq!(
            texts.contains(&"English-only transcript. Hindi or Telugu parts may be missing."),
            open
        );
        assert_eq!(texts.contains(&"Hindi and Telugu models are not installed yet; download them once in Settings, Models (about 567 MB). 2 of 4 ready. Notes use the captured Hex transcript."), open);
    };
    check(&rt, false);
    tap(&mut rt, "meeting-status");
    check(&rt, true);
    tap(&mut rt, "meeting-status-settings");
    assert!(rt.state.hex_open);
    assert!(matches!(rt.state.hex.section, hex::Section::MeetingModels));
}

#[test]
fn meeting_language_status_has_no_task_sorting_title() {
    if in_parent("meeting_language_status_has_no_task_sorting_title") {
        return;
    }
    let rt = page_with_language_fallback();
    let elements = rt.elements();
    let texts: Vec<_> = elements.iter().filter_map(|e| e.text.as_deref()).collect();
    assert!(!texts.contains(&"Tasks not added to a project"));
    assert!(texts.contains(&"Not added to projects yet."));
}

#[test]
fn a_summary_stamped_english_only_shows_the_note() {
    if in_parent("a_summary_stamped_english_only_shows_the_note") {
        return;
    }
    let by = serde_json::json!({"helper": "claude", "language_passes": "english_only"});
    assert!(shows_english_only_note(Some(by), || {}));
}

#[test]
fn a_summary_stamped_routed_shows_nothing() {
    if in_parent("a_summary_stamped_routed_shows_nothing") {
        return;
    }
    let by = serde_json::json!({"helper": "claude", "language_passes": "routed"});
    assert!(!shows_english_only_note(Some(by), || {}));
}

#[test]
fn a_summary_with_no_language_record_shows_the_note() {
    if in_parent("a_summary_with_no_language_record_shows_the_note") {
        return;
    }
    // An older summary: it has a `made_by` record, but no word.
    let by = serde_json::json!({"helper": "claude", "model": "claude-sonnet-5-5"});
    assert!(shows_english_only_note(Some(by), || {}));
}

#[test]
fn the_notes_job_record_cannot_hide_the_note() {
    if in_parent("the_notes_job_record_cannot_hide_the_note") {
        return;
    }
    // The job says it pinned a routed revision with no fallback; the summary says English only.
    let by = serde_json::json!({"language_passes": "english_only"});
    assert!(shows_english_only_note(Some(by), || {
        let dir = core::state_dir().join("local/notes-jobs");
        std::fs::create_dir_all(&dir).unwrap();
        let job = serde_json::json!({
            "state": "done", "fallback": false, "tasks_failed": false, "attempts": 1,
            "transcript_revision": "a".repeat(64), "routing_fallbacks": [],
        });
        std::fs::write(dir.join(format!("{MEETING}.json")), job.to_string()).unwrap();
    }));
}

#[test]
fn a_joined_meeting_shows_its_own_summarys_word() {
    if in_parent("a_joined_meeting_shows_its_own_summarys_word") {
        return;
    }
    // The first recording's own folder says routed; the joined meeting's notes say English only.
    let routed = serde_json::json!({"language_passes": "routed"});
    let english = serde_json::json!({"language_passes": "english_only"});
    assert!(shows_english_only_note(Some(routed), || {
        core::meeting_parts::Meeting::begin(&core::state_dir(), MEETING, 1_800_000_000_000)
            .unwrap();
        let joined = core::meeting_parts::inputs(&core::state_dir()).join(MEETING);
        std::fs::create_dir_all(&joined).unwrap();
        std::fs::write(joined.join("manifest.json"), r#"{"status":"complete"}"#).unwrap();
        stamped_summary(&joined, Some(english));
    }));
}
