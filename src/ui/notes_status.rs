//! Plain, bounded status copy shared by the list, meeting page and live page.
use super::{State, area::button, roles, theme::Palette, widgets::{self, BannerKind}};
use rune::{El, col, row};
use serde::Deserialize;
use sidenote::core::{
    self, Meeting,
    notes::{Helper, JobState, Request},
};
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Debug, Deserialize)]
pub struct Job {
    #[serde(flatten)]
    pub state: JobState,
    #[serde(default)]
    pub helper: Option<Helper>,
    #[serde(default)]
    pub updated_at: i64,
}
impl Job {
    pub fn queued() -> Self {
        Self {
            state: JobState::Queued,
            helper: None,
            updated_at: 0,
        }
    }
}
pub fn read(root: &Path, meetings: &[Meeting]) -> BTreeMap<String, Job> {
    meetings
        .iter()
        .filter(|m| m.device.is_none())
        .filter_map(|m| {
            if !m.id.bytes().all(|b| b.is_ascii_digit() || b == b'-') {
                return None;
            }
            let path = root.join("local/notes-jobs").join(format!("{}.json", m.id));
            core::file_reads::text(&path, core::file_reads::JOURNAL_MAX)
                .ok()
                .and_then(|raw| serde_json::from_str(&raw).ok())
                .map(|j| (m.id.clone(), j))
        })
        .collect()
}
pub fn is_failure(state: &JobState) -> bool {
    matches!(
        state,
        JobState::HelperMissing
            | JobState::BackendError { .. }
            | JobState::TimedOut
            | JobState::BadOutput
            | JobState::TooLong { .. }
    )
}
pub fn message(job: &Job, selected: Option<Helper>) -> String {
    let helper = job.helper.or(selected);
    let name = helper.map(Helper::name).unwrap_or("The AI helper");
    match &job.state {
        JobState::NoHelper => "No notes yet. Sidenote needs an AI helper to write notes. Your transcript is ready below.".into(),
        JobState::NeedsClick => "Notes were not made for this meeting because it ended before AI notes were turned on, or automatic notes were off.".into(),
        JobState::Queued => "This meeting is waiting to have its notes written.".into(),
        JobState::Running => format!("Writing notes with {name}. This can take a few minutes."),
        JobState::HelperMissing => format!("{name} was not found on this PC. Install it again, then choose it in Settings. Your transcript is saved."),
        JobState::BackendError { next_try_ms, .. } => {
            let command = match helper { Some(Helper::Claude) => "claude", Some(Helper::Codex) => "codex", Some(Helper::OpenCode) => "opencode", None => "your AI helper" };
            let retry = next_try_ms.and_then(chrono::DateTime::from_timestamp_millis).map(|t| format!(" Sidenote tries again at {}.", t.with_timezone(&chrono::Local).format("%H:%M"))).unwrap_or_else(|| " Press Try again when you are ready.".into());
            format!("{name} did not finish. This often means you need to sign in again. Open a terminal, type {command}, and sign in.{retry} Your transcript is saved.")
        }
        JobState::TimedOut => format!("{name} took more than 15 minutes and was stopped. Try again in a few minutes. Your transcript is saved."),
        JobState::BadOutput => format!("{name} replied, but Sidenote could not turn the reply into notes. Try again. If it keeps happening, reset your instructions in Settings. Your transcript is saved."),
        JobState::Done { fallback: true, .. } => "These notes used the default instructions, because notes made with your instructions could not be read.".into(),
        JobState::Done { .. } => String::new(),
        JobState::TooLong { minutes } => format!("This meeting ran {} hours{} . Notes can cover the first 4 hours. Choose the first 4 hours below. Your transcript is saved.", minutes / 60, if minutes % 60 == 0 { String::new() } else { format!(" {} minutes", minutes % 60) }).replace("hours .", "hours.").replace("minutes .", "minutes."),
        JobState::Empty => "Hex heard no speech in this meeting, so there are no notes to write.".into(),
    }
}
pub fn label(state: &JobState) -> &'static str {
    match state {
        JobState::NoHelper => "No notes yet",
        JobState::NeedsClick => "Ready to make notes",
        JobState::Queued => "Notes queued",
        JobState::Running => "Writing notes",
        JobState::HelperMissing => "AI helper not found",
        JobState::BackendError { .. } => "AI helper stopped with an error",
        JobState::TimedOut => "AI helper took too long",
        JobState::BadOutput => "AI answer could not be read",
        JobState::TooLong { .. } => "Meeting too long for notes",
        JobState::Empty => "No speech heard",
        JobState::Done { fallback: true, .. } => "Notes used default instructions",
        JobState::Done {
            tasks_failed: true, ..
        } => "Tasks were not sorted",
        JobState::Done { .. } => "Notes ready",
    }
}
fn after_setup(state: JobState, configured: bool) -> JobState {
    if configured && state == JobState::NoHelper {
        JobState::NeedsClick
    } else {
        state
    }
}
pub fn job(s: &State, m: &Meeting) -> Option<Job> {
    if m.device.is_some() {
        return None;
    }
    let configured = s
        .ai_notes
        .settings
        .as_ref()
        .is_some_and(|n| n.engine == "app" && n.helper.is_some());
    s.ai_notes
        .jobs
        .get(&m.id)
        .cloned()
        .or_else(|| {
            (!m.has_summary()).then_some(Job {
                state: if configured {
                    JobState::NeedsClick
                } else {
                    JobState::NoHelper
                },
                helper: None,
                updated_at: 0,
            })
        })
        .map(|mut job| {
            job.state = after_setup(job.state, configured && !m.has_summary());
            job
        })
}
pub fn list_label(s: &State, m: &Meeting) -> Option<&'static str> {
    job(s, m).and_then(|j| {
        if matches!(
            j.state,
            JobState::Done {
                fallback: false,
                tasks_failed: false
            }
        ) {
            None
        } else {
            Some(label(&j.state))
        }
    })
}
fn request_button(title: &str, id: &str, mode: Request, key: &str, s: &State, p: Palette) -> El {
    let id = id.to_string();
    let b = button(title, key, p);
    if s.ai_notes.pending.is_some() {
        b.dim(0.5)
    } else {
        b.on_press(move |s: &mut State| s.request_notes(id.clone(), mode))
    }
}
pub fn banner(s: &State, m: &Meeting, p: Palette) -> Option<El> {
    let job = job(s, m)?;
    banner_for(s, &m.id, &job, p)
}
pub fn banner_for(s: &State, id: &str, job: &Job, p: Palette) -> Option<El> {
    if matches!(
        job.state,
        JobState::Done {
            fallback: false,
            ..
        }
    ) || (matches!(job.state, JobState::Done { .. }) && s.ai_notes.dismissed.contains(id))
    {
        return None;
    }
    let selected = s.ai_notes.settings.as_ref().and_then(|n| n.helper);
    let kind = if is_failure(&job.state) { BannerKind::Problem } else if matches!(job.state, JobState::Running | JobState::Queued) { BannerKind::Working } else { BannerKind::Info };
    let mut c = widgets::banner(label(&job.state), message(job, selected), kind, p)
        .key("notes-state-banner")
        ;
    let mut buttons = row().w_full().flex_wrap().gaps(8.0, 8.0);
    match job.state {
        JobState::NoHelper => {
            buttons = buttons.child(
                button("Set up AI notes", "notes-state-setup", p)
                    .on_press(|s: &mut State| s.open_ai_notes()),
            )
        }
        JobState::NeedsClick => {
            buttons = buttons.child(request_button(
                "Make notes",
                id,
                Request::MakeNotes,
                "notes-state-make",
                s,
                p,
            ))
        }
        JobState::HelperMissing => {
            buttons = buttons.child(
                button("Open Settings", "notes-state-settings", p)
                    .on_press(|s: &mut State| s.open_ai_notes()),
            )
        }
        JobState::BackendError { .. } => {
            buttons = buttons
                .child(request_button(
                    "Try again",
                    id,
                    Request::TryAgain,
                    "notes-state-retry",
                    s,
                    p,
                ))
                .child(
                    button("Open Settings", "notes-state-settings", p)
                        .on_press(|s: &mut State| s.open_ai_notes()),
                )
        }
        JobState::TimedOut | JobState::BadOutput => {
            buttons = buttons.child(request_button(
                "Try again",
                id,
                Request::TryAgain,
                "notes-state-retry",
                s,
                p,
            ));
            if job.state == JobState::BadOutput {
                buttons = buttons.child(edit_button(p));
            }
        }
        JobState::TooLong { .. } => {
            buttons = buttons.child(request_button(
                "Make notes from the first 4 hours",
                id,
                Request::FirstPart,
                "notes-state-first-part",
                s,
                p,
            ))
        }
        JobState::Done { fallback: true, .. } => {
            let id = id.to_string();
            buttons = buttons.child(edit_button(p)).child(
                button("Dismiss", "notes-state-dismiss", p).on_press(move |s: &mut State| {
                    s.ai_notes.dismissed.insert(id.clone());
                }),
            );
        }
        _ => {}
    }
    c = c.child(buttons);
    if let JobState::BackendError { exit: Some(code), .. } = job.state {
        let key = id.to_string();
        c = c.child(button("Details", "notes-error-details", p).self_start().on_press(move |s: &mut State| {
            if !s.ai_notes.details.remove(&key) { s.ai_notes.details.insert(key.clone()); }
        }));
        if s.ai_notes.details.contains(id) {
            c = c.child(roles::META.wrap(format!("The helper ended with exit code {code}."), p.soft));
        }
    }
    if let Some(pending) = s.ai_notes.pending {
        c = c.child(roles::META.wrap(pending, p.soft).role("status"));
    }
    if let Some(error) = &s.ai_notes.error {
        c = c.child(roles::SMALL.wrap(error, p.warn).role("alert"));
    }
    Some(c)
}
fn edit_button(p: Palette) -> El {
    button("Edit instructions", "notes-state-edit", p)
        .on_press(|s: &mut State| {
            s.open_ai_notes();
            super::scene_scroll(s, "agent-instructions");
        })
}
pub fn tasks_notice(s: &State, m: &Meeting, p: Palette) -> Option<El> {
    let j = job(s, m)?;
    matches!(
        j.state,
        JobState::Done {
            tasks_failed: true,
            ..
        }
    )
    .then(|| {
        col()
            .w_full()
            .gap(8.0)
            .mt(12.0)
            .child(roles::SMALL.wrap("These tasks were not sorted into projects.", p.soft))
            .children(
                s.ai_notes
                    .error
                    .as_ref()
                    .map(|error| roles::SMALL.wrap(error, p.warn).role("alert")),
            )
            .children(
                s.ai_notes
                    .pending
                    .map(|text| roles::META.wrap(text, p.soft).role("status")),
            )
            .child(
                request_button(
                    "Sort again",
                    &m.id,
                    Request::SortAgain,
                    "notes-sort-again",
                    s,
                    p,
                )
                .self_start(),
            )
    })
}
pub fn provenance(m: &Meeting, p: Palette) -> Option<El> {
    let raw: serde_json::Value = serde_json::from_str(&m.summary_raw).ok()?;
    let by = raw.get("made_by")?;
    let helper = by.get("helper")?.as_str()?;
    let helper = match helper {
        "claude" | "Claude Code" => "Claude Code",
        "codex" | "Codex" => "Codex",
        "open_code" | "OpenCode" => "OpenCode",
        _ => return None,
    };
    let model = by
        .get("model")
        .and_then(|m| m.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("configured model");
    Some(
        roles::META
            .wrap(format!("Notes by {helper} ({model})"), p.soft)
            .key("notes-provenance")
            .mt(24.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn meetings_that_previously_needed_setup_offer_a_manual_request_after_setup() {
        let job: Job = serde_json::from_str(r#"{"state":"no_helper","updated_at":1}"#).unwrap();
        assert_eq!(after_setup(job.state.clone(), false), JobState::NoHelper);
        assert_eq!(after_setup(job.state, true), JobState::NeedsClick);
        assert_eq!(after_setup(JobState::Empty, true), JobState::Empty);
        assert_eq!(after_setup(JobState::Running, true), JobState::Running);
    }

    #[test]
    fn every_failure_has_plain_copy_without_raw_helper_output() {
        for (name, state) in [
            ("no-helper", JobState::NoHelper),
            ("needs-click", JobState::NeedsClick),
            ("queued", JobState::Queued),
            ("running", JobState::Running),
            ("helper-missing", JobState::HelperMissing),
            (
                "backend-error",
                JobState::BackendError {
                    next_try_ms: Some(1794305400000),
                    exit: Some(1),
                },
            ),
            (
                "backend-manual",
                JobState::BackendError {
                    next_try_ms: None,
                    exit: None,
                },
            ),
            ("timeout", JobState::TimedOut),
            ("bad-output", JobState::BadOutput),
            (
                "fallback",
                JobState::Done {
                    fallback: true,
                    tasks_failed: false,
                },
            ),
            ("too-long", JobState::TooLong { minutes: 360 }),
            ("empty", JobState::Empty),
        ] {
            let job = Job {
                state,
                helper: Some(Helper::Codex),
                updated_at: 0,
            };
            let text = message(&job, Some(Helper::Claude));
            assert!(!text.is_empty(), "{name}");
            assert!(!label(&job.state).is_empty());
            assert!(!text.contains('\u{2014}'));
            assert!(!text.contains("Claude Code"));
        }
    }
    #[test]
    fn helper_name_command_exit_and_retry_are_specific_to_the_job() {
        let mut job = Job {
            state: JobState::BackendError {
                next_try_ms: None,
                exit: Some(7),
            },
            helper: Some(Helper::OpenCode),
            updated_at: 0,
        };
        let text = message(&job, Some(Helper::Claude));
        assert!(text.contains("OpenCode did not finish"));
        assert!(!text.contains("exit code"));
        assert!(text.contains("type opencode"));
        assert!(text.contains("Press Try again"));
        assert!(text.contains("Your transcript is saved."));
        job.state = JobState::BackendError {
            next_try_ms: Some(1794305400000),
            exit: None,
        };
        assert!(message(&job, None).contains("tries again at"));
    }
    #[test]
    fn job_state_ignores_untrusted_message_text() {
        let raw = r#"{"state":"backend_error","next_try_ms":null,"exit":1,"helper":"codex","message":"PRIVATE MEETING TEXT","updated_at":2}"#;
        let job: Job = serde_json::from_str(raw).unwrap();
        assert!(!message(&job, None).contains("PRIVATE"));
        assert_eq!(job.helper, Some(Helper::Codex));
    }
}
