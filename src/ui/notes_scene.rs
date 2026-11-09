//! Invented UI states. No helper, Hex call, or worker process runs in these scenes.
use super::{
    State,
    ai_notes::{AiNotes, HELPERS},
    hex,
    notes_status::Job,
};
use rune::TextEdit;
use sidenote::core::{
    agent_instructions,
    notes::{Helper, JobState, NotesSettings},
};

pub fn prepare(s: &mut State) {
    super::sync_scene::settle(s);
    hex::scene(s, false);
    s.ai_notes = AiNotes {
        scene: true,
        settings: Some(NotesSettings {
            helper: Some(Helper::Claude),
            model: Helper::Claude.default_model().into(),
            enabled_at_ms: Some(1),
            ..NotesSettings::default()
        }),
        found: HELPERS.map(|h| (h, true)).into(),
        ..AiNotes::default()
    };
    s.sidebar_open.set(false);
    s.drawer_open.set(false);
    s.ask_open = false;
    s.config.features.daily_check = false;
    s.config.features.memory = false;
}
pub fn settings(s: &mut State, mode: &str) {
    prepare(s);
    s.hex_open = true;
    s.hex.section = hex::Section::AiNotes;
    match mode {
        "off" | "none" => {
            let settings = s.ai_notes.settings.as_mut().unwrap();
            settings.helper = None;
            if mode == "off" {
                settings.engine = "off".into();
            }
        }
        "last-error" => {
            s.ai_notes.jobs.insert(
                "1800000000000-1".into(),
                Job {
                    state: state("backend-error"),
                    helper: Some(Helper::Claude),
                    updated_at: 1,
                },
            );
        }
        "scripts" | "switch" => {
            let settings = s.ai_notes.settings.as_mut().unwrap();
            settings.engine = "scripts".into();
            settings.script = Some(std::env::temp_dir().join("example-notes.ps1"));
            if mode == "switch" {
                s.ai_notes.switch_confirm = Some(Helper::Claude);
            }
        }
        "testing" => {
            s.ai_notes.pending = Some("Testing fixed sample text...");
            s.ai_notes.testing = Some(0);
        }
        "worked" => {
            s.ai_notes
                .test_results
                .insert(0, "Worked in 8 s. Only fixed sample text was sent.".into());
        }
        "test-error" => {
            s.ai_notes
                .test_results
                .insert(0, "The helper test failed; check its sign-in".into());
        }
        "load-error" => {
            s.ai_notes.settings = None;
            s.ai_notes.error = Some("Could not read AI notes settings. Try Reload.".into());
        }
        "loading" => {
            s.ai_notes.settings = None;
            s.ai_notes.pending = Some("Loading AI notes...");
        }
        "helpers" => {
            super::scene_scroll(s, "notes-helper-0");
        }
        "options" => {
            s.ai_notes.settings.as_mut().unwrap().auto = false;
            s.ai_notes.name = TextEdit::new("Alex");
            super::scene_scroll(s, "notes-options");
        }
        "edited" => {
            let e = &mut s.ai_notes.editors[0];
            e.text = TextEdit::new(
                "Keep exact decisions and numbers. Give each to-do a clear owner. Use short sentences.",
            );
            e.saved = e.text.text.clone();
            e.edited_at = Some("2026-11-10T04:00:00Z".into());
        }
        "limit" => {
            s.ai_notes.editors[0].text =
                TextEdit::new("a".repeat(agent_instructions::MAX_CHARS + 1));
        }
        "reset" => s.ai_notes.editors[0].reset_confirm = true,
        "fixed-notes" => s.ai_notes.editors[0].fixed_open = true,
        "fixed-tasks" => s.ai_notes.editors[1].fixed_open = true,
        "save-error" => {
            s.ai_notes.editor_target = Some(0);
            s.ai_notes.error =
                Some("Could not save instructions. Check the length and try again.".into());
        }
        _ => {}
    }
    if mode == "scripts" {
        super::scene_scroll(s, "notes-scripts-card");
    }
    if mode == "none" {
        s.ai_notes.found = HELPERS.map(|h| (h, false)).into();
    }
    if matches!(
        mode,
        "instructions" | "edited" | "limit" | "reset" | "fixed-notes" | "save-error"
    ) {
        super::scene_scroll(s, "instructions-0");
    }
    if matches!(mode, "tasks" | "fixed-tasks") {
        super::scene_scroll(s, "instructions-1");
    }
    if matches!(mode, "worked" | "test-error" | "testing" | "switch") {
        super::scene_scroll(s, "notes-helper-0");
    }
}
pub fn today(s: &mut State, found: bool) {
    prepare(s);
    s.go_home();
    s.ai_notes.settings.as_mut().unwrap().helper = None;
    if !found {
        s.ai_notes.found = HELPERS.map(|h| (h, false)).into();
    }
}
pub fn state(name: &str) -> JobState {
    match name {
        "no-helper" | "no-notes" => JobState::NoHelper,
        "needs-click" => JobState::NeedsClick,
        "queued" => JobState::Queued,
        "running" => JobState::Running,
        "helper-missing" => JobState::HelperMissing,
        "backend-error" => JobState::BackendError {
            next_try_ms: Some(1794305400000),
            exit: Some(1),
        },
        "backend-manual" => JobState::BackendError {
            next_try_ms: None,
            exit: None,
        },
        "timeout" => JobState::TimedOut,
        "bad-output" => JobState::BadOutput,
        "fallback" => JobState::Done {
            fallback: true,
            tasks_failed: false,
        },
        "too-long" => JobState::TooLong { minutes: 360 },
        "empty" => JobState::Empty,
        "tasks-failed" => JobState::Done {
            fallback: false,
            tasks_failed: true,
        },
        _ => JobState::Done {
            fallback: false,
            tasks_failed: false,
        },
    }
}
pub fn meeting(s: &mut State, name: &str) {
    prepare(s);
    let Some(mut m) = s.meetings.first().cloned() else {
        panic!("Notes scenes need a synthetic meeting fixture")
    };
    m.title = "Planning the launch".into();
    m.device = None;
    m.summary_raw = if matches!(name, "fallback" | "tasks-failed" | "done") {
        serde_json::json!({"made_by":{"helper":"claude","model":"claude-sonnet-5-5"}}).to_string()
    } else {
        String::new()
    };
    if !m.has_summary() {
        m.summary = Default::default();
    }
    if name == "empty" {
        m.lines.clear();
        m.transcript_raw.clear();
    }
    if name == "too-long" {
        m.minutes = Some(360);
    }
    m.notes = "Share the revised plan on Friday.".into();
    s.selected = Some(m.id.clone());
    s.ai_notes.jobs.insert(
        m.id.clone(),
        Job {
            state: state(name),
            helper: Some(Helper::Claude),
            updated_at: 1,
        },
    );
    if name == "no-notes" {
        s.ai_notes.jobs.clear();
    }
    if matches!(name, "no-notes" | "no-helper") {
        s.ai_notes.settings.as_mut().unwrap().helper = None;
    }
    s.meetings = vec![m];
    s.rebuild_tasks();
    s.refresh_list();
    if name == "tasks-failed" {
        super::scene_scroll(s, "sec-todo");
    }
}
pub fn live(s: &mut State, name: &str) {
    meeting(s, name);
    let id = s.selected.take().unwrap();
    s.live = Some(
        super::live::Live::scene(super::live::Phase::Making, Some("Planning the launch"))
            .scene_pending(&id),
    );
    s.live_open = true;
    s.live_notes = TextEdit::new("Share the revised plan on Friday.");
}
pub fn list(s: &mut State) {
    prepare(s);
    s.ai_notes.settings.as_mut().unwrap().helper = None;
    let template = s.meetings.first().expect("Synthetic meeting").clone();
    let names = [
        "no-helper",
        "needs-click",
        "queued",
        "running",
        "helper-missing",
        "backend-error",
        "backend-manual",
        "timeout",
        "bad-output",
        "fallback",
        "too-long",
        "empty",
        "tasks-failed",
        "done",
    ];
    s.meetings = names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let mut m = template.clone();
            m.id = format!("1800000000000-{i}");
            m.title = format!("Example meeting {}", i + 1);
            m.device = None;
            s.ai_notes.jobs.insert(
                m.id.clone(),
                Job {
                    state: state(name),
                    helper: Some(Helper::Claude),
                    updated_at: 1,
                },
            );
            m
        })
        .collect();
    s.go_home();
    s.refresh_list();
}
