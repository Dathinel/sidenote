//! Plain, bounded status copy shared by the list, meeting page and live page.
use super::{
    State,
    area::button,
    roles,
    theme::Palette,
    widgets::{self, BannerKind},
};
use rune::{El, row};
use sidenote::core::{
    self, Meeting,
    notes::{Helper, JobState, Request},
};
use std::{collections::BTreeMap, path::Path};

pub use sidenote::core::notes::StatusDetail as Job;
pub fn read(root: &Path, meetings: &[Meeting]) -> BTreeMap<String, Job> {
    core::notes::status_records(
        root,
        meetings
            .iter()
            .filter(|m| m.device.is_none())
            .map(|m| m.id.as_str()),
    )
}
pub fn is_failure(state: &JobState) -> bool {
    matches!(
        state,
        JobState::HelperUnavailableOnLinux
            | JobState::HelperMissing
            | JobState::BackendError { .. }
            | JobState::HelperLimit { .. }
            | JobState::HelperAuth { .. }
            | JobState::TimedOut
            | JobState::BadOutput
            | JobState::TooLong { .. }
            | JobState::ContextTooLong { .. }
    )
}
pub fn running_message(name: &str, started_at: i64) -> String {
    let minutes = chrono::Utc::now()
        .timestamp_millis()
        .saturating_sub(started_at)
        .max(0)
        / 60_000;
    format!(
        "Writing summary with {name}. This can take up to 15 minutes. Started {minutes} minutes ago."
    )
}
pub fn message(job: &Job, selected: Option<Helper>) -> String {
    let helper = job.helper.or(selected);
    let name = helper.map(Helper::name).unwrap_or("The agent");
    let standard: String = match &job.state {
        JobState::NoHelper => "Choose Claude Code in Settings, then press Write summary. Your transcript is saved.".into(),
        JobState::NeedsClick => "Sidenote only writes summaries automatically for meetings that end after you turn on Meeting summary. You can write this one now.".into(),
        JobState::Queued => "Waiting for another summary to finish.".into(),
        JobState::Running => running_message(name, job.updated_at),
        JobState::HelperUnavailableOnLinux => core::notes::LINUX_HELPER_UNAVAILABLE.into(),
        JobState::HelperMissing => format!("{name} was not found on this PC. Install {name}, then choose it in Settings. Your transcript is saved."),
        JobState::BackendError { next_try_ms, .. } => {
            let command = match helper { Some(Helper::Claude) => "claude", Some(Helper::Codex) => "codex", Some(Helper::OpenCode) => "opencode", None => "your agent" };
            let retry = next_try_ms.and_then(chrono::DateTime::from_timestamp_millis).map(|t| format!(" Sidenote tries again at {}.", t.with_timezone(&chrono::Local).format("%H:%M"))).unwrap_or_else(|| " Press Try again when you are ready.".into());
            format!("{name} did not finish. This often means you need to sign in again. Open a terminal, type {command}, and sign in.{retry} Your transcript is saved.")
        }
        JobState::HelperLimit { .. } => format!("{name} has hit its usage limit. Your transcript is saved."),
        JobState::HelperAuth { .. } => format!("{name} needs you to sign in again. Your transcript is saved."),
        JobState::TimedOut => format!("{name} took more than 15 minutes and was stopped. Try again in a few minutes. Your transcript is saved."),
        JobState::BadOutput => format!("{name} replied, but Sidenote could not turn the reply into a summary. Try again. If it keeps happening, press Open instructions to review or reset them. Your transcript is saved."),
        JobState::Done { fallback: true, .. } => "This summary used the default instructions because your own instructions did not work. Edit your instructions before trying again.".into(),
        JobState::Done { .. } => String::new(),
        JobState::TooLong { minutes } => {
            let length = if minutes % 60 == 0 { format!("{} hours", minutes / 60) } else { format!("{} hours {} minutes", minutes / 60, minutes % 60) };
            format!("This meeting ran {length}. Summaries can cover the first 4 hours. Use the button below to summarise the first 4 hours. Your transcript is saved.")
        },
        JobState::ContextTooLong { typed_notes, glossary } => {
            let input = match (typed_notes, glossary) {
                (true, true) => "Typed notes and glossary are each",
                (true, false) => "Typed notes are",
                _ => "The glossary is",
            };
            format!("{input} over the 16,000 character limit. Shorten the input, then press Try again. Your transcript is saved.")
        }
        JobState::Empty => "Sidenote heard no speech in this meeting, so there is nothing to summarise.".into(),
    };
    let routing = job.routing_message();
    if routing.is_empty() {
        standard
    } else if standard.is_empty() {
        routing
    } else {
        format!("{routing} {standard}")
    }
}

fn label_for(job: &Job) -> &'static str {
    if !job.routing_message().is_empty()
        && matches!(
            job.state,
            JobState::Done {
                tasks_failed: false,
                ..
            }
        )
    {
        "Summary used Hex coverage"
    } else {
        label(&job.state)
    }
}

pub fn visible_copy(
    job: &Job,
    selected: Option<Helper>,
    dismissed: bool,
) -> Option<(&'static str, String)> {
    if matches!(job.state, JobState::Done { .. })
        && (dismissed
            || (matches!(
                job.state,
                JobState::Done {
                    fallback: false,
                    ..
                }
            ) && job.routing_message().is_empty()))
    {
        return None;
    }
    Some((label_for(job), message(job, selected)))
}

pub fn label(state: &JobState) -> &'static str {
    match state {
        JobState::NoHelper => "No summary yet",
        JobState::NeedsClick => "Ready to write a summary",
        JobState::Queued => "Summary queued",
        JobState::Running => "Writing summary",
        JobState::HelperUnavailableOnLinux => "Summaries not available on Linux yet",
        JobState::HelperMissing => "Agent not found",
        JobState::BackendError {
            next_try_ms: Some(_),
            ..
        } => "Agent failed, retry scheduled",
        JobState::BackendError { .. } => "Agent failed, retry needed",
        JobState::HelperLimit { .. } => "Agent usage limit reached",
        JobState::HelperAuth { .. } => "Agent sign-in needed",
        JobState::TimedOut => "Agent took too long",
        JobState::BadOutput => "Agent reply could not be read",
        JobState::TooLong { .. } => "Meeting too long for a summary",
        JobState::ContextTooLong { .. } => "Shorten the summary input",
        JobState::Empty => "No speech heard",
        JobState::Done { fallback: true, .. } => "Summary used default instructions",
        JobState::Done {
            tasks_failed: true, ..
        } => "Tasks not added to a project",
        JobState::Done { .. } => "Summary ready",
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
    if m.device.is_some() || (!m.has_summary() && !m.draft_raw.is_empty()) {
        return None;
    }
    let configured = s
        .ai_notes
        .settings
        .as_ref()
        .is_some_and(|n| n.engine == "app" && n.effective_helper().is_some());
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
                request: None,
                transcript_revision: None,
                routing_fallbacks: Vec::new(),
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
        ) && j.routing_message().is_empty()
        {
            None
        } else {
            Some(label_for(&j))
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
pub(super) fn can_use_codex(job: &Job, found: &[(Helper, bool)]) -> bool {
    matches!(job.state, JobState::HelperLimit { .. })
        && job.helper == Some(Helper::Claude)
        && found.contains(&(Helper::Codex, true))
}
pub(super) fn use_codex_button(
    s: &State,
    id: &str,
    job: &Job,
    key: &str,
    p: Palette,
) -> Option<El> {
    if !can_use_codex(job, &s.ai_notes.found) {
        return None;
    }
    let id = id.to_string();
    Some(
        button("Use Codex for summaries", key, p)
            .disabled(s.ai_notes.pending.is_some())
            .on_press(move |s: &mut State| s.use_codex_for_summary(id.clone())),
    )
}
pub fn banner_for(s: &State, id: &str, job: &Job, p: Palette) -> Option<El> {
    let selected = s
        .ai_notes
        .settings
        .as_ref()
        .and_then(|n| n.effective_helper());
    let (title, copy) = visible_copy(job, selected, s.ai_notes.dismissed.contains(id))?;
    let kind = if is_failure(&job.state) {
        BannerKind::Problem
    } else if matches!(job.state, JobState::Running | JobState::Queued) {
        BannerKind::Working
    } else {
        BannerKind::Info
    };
    let mut c = widgets::banner(title, copy, kind, p).key("notes-state-banner");
    let mut buttons = row().w_full().flex_wrap().gaps(8.0, 8.0);
    match job.state {
        JobState::NoHelper => {
            buttons = buttons.child(
                button("Choose an agent", "notes-state-setup", p)
                    .on_press(|s: &mut State| s.open_ai_notes()),
            )
        }
        JobState::NeedsClick => {
            let title = selected
                .map(|h| format!("Write summary with {}", h.name()))
                .unwrap_or_else(|| "Write summary".into());
            buttons = buttons.child(request_button(
                &title,
                id,
                Request::MakeNotes,
                "notes-state-make",
                s,
                p,
            ))
        }
        JobState::HelperMissing => {
            buttons = buttons.child(
                button("Choose an agent", "notes-state-settings", p)
                    .on_press(|s: &mut State| s.open_ai_notes()),
            )
        }
        JobState::BackendError { .. }
        | JobState::HelperAuth { .. }
        | JobState::HelperLimit { .. } => {
            buttons = buttons
                .child(request_button(
                    "Try again",
                    id,
                    job.retry_request(),
                    "notes-state-retry",
                    s,
                    p,
                ))
                .child(
                    button("Choose an agent", "notes-state-settings", p)
                        .on_press(|s: &mut State| s.open_ai_notes()),
                )
        }
        JobState::TimedOut | JobState::BadOutput | JobState::ContextTooLong { .. } => {
            buttons = buttons.child(request_button(
                "Try again",
                id,
                job.retry_request(),
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
                "Write summary from the first 4 hours",
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
    if let JobState::BackendError { exit: Some(_), .. } = job.state {
        let key = id.to_string();
        buttons = buttons.child(
            button("Details", "notes-error-details", p)
                .self_start()
                .on_press(move |s: &mut State| {
                    if !s.ai_notes.details.remove(&key) {
                        s.ai_notes.details.insert(key.clone());
                    }
                }),
        );
    }
    c = c.child(buttons.children(use_codex_button(s, id, job, "notes-use-codex", p)));
    if let JobState::BackendError {
        exit: Some(code), ..
    } = job.state
        && s.ai_notes.details.contains(id)
    {
        c = c.child(roles::META.wrap(format!("The agent ended with exit code {code}."), p.soft));
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
    widgets::pressable(
        row()
            .gap(6.0)
            .items_center()
            .child(super::icons::draw(&super::icons::PROMPTS, 16.0, p.soft))
            .child(roles::CONTROL.wrap("Open\u{a0}instructions", p.soft)),
    )
    .key("notes-state-edit")
    .px(10.0)
    .py(6.0)
    .radius(8.0)
    .border(1.0, p.line)
    .hover(p.chip)
    .label("Open instructions")
    .on_press(|s: &mut State| {
        s.open_hex();
        s.hex.section = super::hex::Section::PromptLibrary;
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
        widgets::banner(
            "Tasks need another try",
            "Tasks could not be added to projects. Press Try again. Your transcript is saved.",
            BannerKind::Problem,
            p,
        )
        .mt(12.0)
        .mb(16.0)
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
                "Try again",
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
            .wrap(format!("Summary by {helper} ({model})"), p.soft)
            .key("notes-provenance")
            .mt(24.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires production corruption notes-job receipts"]
    fn production_corruption_status_remains_visible_after_success() {
        let root =
            std::path::PathBuf::from(std::env::var_os("SIDENOTE_ROUTING_R1_STATUS_OUT").unwrap());
        assert!(core::is_proof_path(
            &root,
            "sidenote-routing-r1-status-public"
        ));
        for (case, reason) in [
            ("missing", "missing or damaged"),
            ("truncated", "missing or damaged"),
            ("checksum", "missing or damaged"),
            ("pending", "request record was damaged"),
        ] {
            let root = root.join(case);
            let folder = std::fs::read_dir(root.join("meetings"))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            let meeting = Meeting::load_pending(&folder).unwrap();
            let jobs = read(&root, &[meeting]);
            assert_eq!(jobs.len(), 1);
            let job = jobs.values().next().unwrap();
            assert!(matches!(
                job.state,
                JobState::Done {
                    fallback: false,
                    ..
                }
            ));
            let (label, text) =
                visible_copy(job, None, false).expect("routing warning must remain visible");
            assert_eq!(label, "Summary used Hex coverage");
            assert!(text.contains(reason), "{case}: {text}");
            assert!(!text.contains("default instructions"));
        }
    }

    #[test]
    fn linux_unavailable_has_specific_copy_in_both_shared_banners() {
        let job = Job {
            state: JobState::HelperUnavailableOnLinux,
            helper: Some(Helper::Codex),
            updated_at: 0,
            request: None,
            transcript_revision: None,
            routing_fallbacks: Vec::new(),
        };
        assert!(is_failure(&job.state));
        assert!(message(&job, None).contains("not available on Linux yet"));
        assert_eq!(label(&job.state), "Summaries not available on Linux yet");
    }
    #[test]
    fn limit_recovery_requires_detected_codex_and_preserves_the_failed_request() {
        let mut job = Job {
            state: JobState::HelperLimit { exit: Some(1) },
            helper: Some(Helper::Claude),
            updated_at: 0,
            request: Some(Request::FirstPart),
            transcript_revision: None,
            routing_fallbacks: Vec::new(),
        };
        assert_eq!(job.retry_request(), Request::FirstPart);
        assert_eq!(
            message(&job, None),
            "Claude Code has hit its usage limit. Your transcript is saved."
        );
        assert!(can_use_codex(&job, &[(Helper::Codex, true)]));
        assert!(!can_use_codex(&job, &[(Helper::Codex, false)]));
        job.helper = Some(Helper::Codex);
        assert!(!can_use_codex(&job, &[(Helper::Codex, true)]));
        job.helper = Some(Helper::Claude);
        job.state = JobState::HelperAuth { exit: Some(1) };
        assert!(!can_use_codex(&job, &[(Helper::Codex, true)]));
    }
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
                request: None,
                transcript_revision: None,
                routing_fallbacks: Vec::new(),
            };
            let text = message(&job, Some(Helper::Claude));
            assert!(!text.is_empty(), "{name}");
            assert!(!label(&job.state).is_empty());
            assert!(!text.contains('\u{2014}'));
            if matches!(
                job.state,
                JobState::Running
                    | JobState::HelperMissing
                    | JobState::BackendError { .. }
                    | JobState::TimedOut
                    | JobState::BadOutput
            ) {
                assert!(!text.contains("Claude Code"));
            }
            if is_failure(&job.state) {
                assert!(text.contains("Your transcript is saved."), "{name}");
            }
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
            request: None,
            transcript_revision: None,
            routing_fallbacks: Vec::new(),
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
    fn context_retry_preserves_first_part_and_task_requests() {
        for request in [Request::FirstPart, Request::RedoTasks, Request::SortAgain] {
            let raw = serde_json::json!({"state":"context_too_long","typed_notes":true,"glossary":false,"request":request});
            let job: Job = serde_json::from_value(raw).unwrap();
            assert_eq!(job.retry_request(), request);
        }
    }
    #[test]
    fn context_overflow_names_the_input_and_offers_shortening() {
        for (typed_notes, glossary, name) in [
            (true, false, "Typed notes"),
            (false, true, "glossary"),
            (true, true, "Typed notes and glossary"),
        ] {
            let job = Job {
                state: JobState::ContextTooLong {
                    typed_notes,
                    glossary,
                },
                helper: Some(Helper::Claude),
                updated_at: 0,
                request: None,
                transcript_revision: None,
                routing_fallbacks: Vec::new(),
            };
            let text = message(&job, None);
            assert!(text.contains(name));
            assert!(text.contains("16,000"));
            assert!(text.contains("Try again"));
            assert!(!text.contains("first 4 hours"));
        }
    }
    #[test]
    fn job_state_ignores_untrusted_message_text() {
        let raw = r#"{"state":"backend_error","next_try_ms":null,"exit":1,"helper":"codex","message":"PRIVATE MEETING TEXT","updated_at":2}"#;
        let job: Job = serde_json::from_str(raw).unwrap();
        assert!(!message(&job, None).contains("PRIVATE"));
        assert_eq!(job.helper, Some(Helper::Codex));
    }
}
