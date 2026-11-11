//! Four provider turns, synthetic text only. Uses the production live Ask path.
use sidenote::core::{
    self, AskProgress, Message,
    ask_options::{Fallback, LiveSource, Options, Thinking},
};
use std::{
    cell::{Cell, RefCell},
    ffi::OsString,
    time::{Duration, Instant},
};

fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let state = root
        .join("target")
        .join(format!("meeting-agent-measure-{}", std::process::id()));
    let meetings = state.join("meetings");
    let folder = meetings.join("1800000000000-1");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("english-transcript.md"), "[00:04] You: We decided to test the paper prototype on Friday.\n[00:09] Them: Maya will prepare it.\n").unwrap();
    core::configure_paths(&[
        OsString::from("--data-dir"),
        state.clone().into_os_string(),
        OsString::from("--meetings-dir"),
        meetings.into_os_string(),
        OsString::from("--peers-dir"),
        state.join("peers").into_os_string(),
    ])
    .unwrap();
    let mut runners = core::list_runners().unwrap();
    let codex = runners
        .iter()
        .find(|r| core::ask_options::agent_name(r) == "Codex")
        .unwrap()
        .clone();
    let normal = runners
        .iter()
        .find(|r| core::ask_options::agent_name(r) != "Codex" && core::runners::installed(r))
        .expect("Installed fallback agent")
        .clone();
    let options = Options {
        thinking: Thinking::Low,
        meeting: true,
        meeting_agent: true,
        live: Some(LiveSource {
            hint: Some("1800000000000-1".into()),
            start_ms: None,
        }),
        ..Default::default()
    };
    let context = core::ask_all::AllContext {
        text: String::new(),
        included: 0,
        left_out: 0,
        from: None,
        to: None,
        today: chrono::Local::now().date_naive(),
    };
    let history = vec![Message {
        role: "user".into(),
        text: "What did we decide, and who will prepare it? Answer in two lines. Do not use tools."
            .into(),
        ..Default::default()
    }];
    let mut rows = Vec::new();
    let output = root.join("docs/meeting-agent-timings.json");
    core::ask_session::recording(true);
    if std::env::args().any(|arg| arg == "--primed-only") {
        let start = Instant::now();
        core::ask::prewarm_meeting(codex.clone(), &options, core::ALL_KEY.into())
            .unwrap()
            .recv_timeout(Duration::from_secs(90))
            .unwrap()
            .unwrap();
        let ready_s = start.elapsed().as_secs_f64();
        let start = Instant::now();
        let first = Cell::new(None);
        let answer =
            core::ask_all::ask_with_options(&context, &history, &codex.name, &options, |event| {
                if matches!(event, AskProgress::Text { ref text } if !text.is_empty())
                    && first.get().is_none()
                {
                    first.set(Some(start.elapsed().as_secs_f64()));
                }
            })
            .unwrap();
        let row = serde_json::json!({"call":"primed-first","warm_up_full_s":ready_s,"first_text_s":first.get(),"full_answer_s":start.elapsed().as_secs_f64(),"answer":answer,"provider_turns":2});
        assert!(core::ask::cancel_all(Duration::from_secs(5)));
        std::fs::write(
            root.join("docs/meeting-agent-primed-timing.json"),
            serde_json::to_vec_pretty(&row).unwrap(),
        )
        .unwrap();
        println!(
            "primed-first ready={} first={} full={}",
            ready_s, row["first_text_s"], row["full_answer_s"]
        );
        return;
    }
    for label in ["cold", "warm", "prewarmed-first", "fallback"] {
        let mut options = options.clone();
        if label == "prewarmed-first" {
            core::ask_session::release();
            core::ask::prewarm(codex.clone(), &options, core::ALL_KEY.into());
            // A synthetic three-second gap between starting a meeting and asking.
            std::thread::sleep(Duration::from_secs(3));
        }
        if label == "fallback" {
            core::ask_session::release();
            runners
                .iter_mut()
                .find(|r| r.name == codex.name)
                .unwrap()
                .program = state
                .join("missing/codex.exe")
                .to_string_lossy()
                .into_owned();
            std::fs::write(
                state.join("runners.json"),
                serde_json::to_vec(&runners).unwrap(),
            )
            .unwrap();
            options.fallback = Some(Fallback {
                runner: normal.name.clone(),
                model: normal.model.clone(),
                thinking: Thinking::Medium,
            });
        }
        let first = Cell::new(None);
        let fallback = RefCell::new(None);
        let start = Instant::now();
        let answer =
            core::ask_all::ask_with_options(&context, &history, &codex.name, &options, |event| {
                match event {
                    AskProgress::Text { text } if !text.is_empty() && first.get().is_none() => {
                        first.set(Some(start.elapsed().as_secs_f64()))
                    }
                    AskProgress::Fallback { runner } => *fallback.borrow_mut() = Some(runner),
                    _ => {}
                }
            })
            .expect("Synthetic answer");
        assert!(
            answer.contains("Friday") && answer.contains("Maya"),
            "Missing synthetic evidence"
        );
        if label == "fallback" {
            assert_eq!(fallback.borrow().as_deref(), Some(normal.name.as_str()));
        }
        rows.push(serde_json::json!({"call":label,"first_text_s":first.get(),"full_answer_s":start.elapsed().as_secs_f64(),"fallback":*fallback.borrow(),"answer":answer}));
        std::fs::write(&output, serde_json::to_vec_pretty(&rows).unwrap()).unwrap();
    }
    core::ask_session::recording(false);
    assert!(core::ask::cancel_all(Duration::from_secs(5)));
    for row in rows {
        println!(
            "{} first={} full={}",
            row["call"], row["first_text_s"], row["full_answer_s"]
        );
    }
}
