//! Read-only Diary follow-ups. Display copy is derived from typed sources, never raw briefs.
use super::{
    Meeting, TaskItem, TaskStatus,
    check_v2::FindingChange,
    daily_check::{DailyCheck, Finding, FindingKind, Severity},
    memory::{MemoryRecord, PendingProposal, ProposalStatus, RecordStatus},
};
use chrono::{Datelike, Duration, NaiveDate};
use std::collections::{BTreeSet, HashMap};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    Task {
        meeting: String,
        stable: String,
        index: usize,
    },
    Memory(String),
    Proposal(String),
    Meeting(String),
    Review {
        date: NaiveDate,
        finding: String,
    },
}
impl Route {
    pub fn key(&self) -> String {
        match self {
            Self::Task {
                meeting,
                stable,
                index,
            } => format!(
                "task:{meeting}:{}",
                if stable.is_empty() {
                    index.to_string()
                } else {
                    stable.clone()
                }
            ),
            Self::Memory(id) => format!("memory:{id}"),
            Self::Proposal(id) => format!("proposal:{id}"),
            Self::Meeting(id) => format!("meeting:{id}"),
            Self::Review { date, finding } => format!("review:{date}:{finding}"),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    DueToday,
    Due(NaiveDate),
    Overdue,
    MissingOwner,
    ConflictingDecisions,
    ReviewUpdate,
    FollowUp,
    Unavailable,
}
impl Reason {
    pub fn text(&self) -> String {
        match self {
            Self::DueToday => "Due today".into(),
            Self::Due(date) => format!("Due {}", date.format("%a %-d %b")),
            Self::Overdue => "Overdue".into(),
            Self::MissingOwner => "An owner is missing".into(),
            Self::ConflictingDecisions => "Two decisions need review".into(),
            Self::ReviewUpdate => "Review the latest update".into(),
            Self::FollowUp => "A follow-up is still open".into(),
            Self::Unavailable => "Source unavailable. Review the original check".into(),
        }
    }
}
#[derive(Clone)]
pub struct Report {
    pub report: DailyCheck,
    pub open: BTreeSet<usize>,
}
#[derive(Clone, Debug)]
pub struct Attention {
    pub route: Route,
    pub title: String,
    pub context: String,
    pub reason: Reason,
    pub rank: u8,
    pub due: Option<NaiveDate>,
}
pub fn task_identity(t: &TaskItem) -> String {
    if t.stable_id.is_empty() {
        format!("text:{}", super::store_io::fingerprint([t.task.as_str()]))
    } else {
        t.stable_id.clone()
    }
}
pub fn task_route(t: &TaskItem) -> Route {
    Route::Task {
        meeting: t.meeting_id.clone(),
        stable: task_identity(t),
        index: t.index,
    }
}
pub fn owned(t: &TaskItem) -> bool {
    t.mine == Some(true) && !t.done
}
pub fn week_end(today: NaiveDate) -> NaiveDate {
    today + Duration::days(6 - today.weekday().num_days_from_monday() as i64)
}
pub fn due_week(t: &TaskItem, today: NaiveDate) -> bool {
    owned(t)
        && t.due_date
            .is_some_and(|d| d >= today && d <= week_end(today))
}
pub fn waiting(t: &TaskItem) -> bool {
    owned(t)
        && t.status == TaskStatus::Waiting
        && (t
            .waiting_label
            .as_ref()
            .is_some_and(|v| !v.trim().is_empty())
            || t.waiting_on.as_ref().is_some_and(|w| {
                w.task_id.as_ref().is_some_and(|v| !v.trim().is_empty())
                    || w.text.as_ref().is_some_and(|v| !v.trim().is_empty())
            }))
}

#[derive(Clone)]
pub enum TaskSelection {
    Needs(BTreeSet<String>),
    Keys(BTreeSet<String>),
    Due { start: NaiveDate, end: NaiveDate },
    Waiting,
}
impl TaskSelection {
    pub fn matches(&self, t: &TaskItem) -> bool {
        match self {
            Self::Needs(keys) => !t.done && keys.contains(&task_route(t).key()),
            Self::Keys(keys) => keys.contains(&task_route(t).key()),
            Self::Due { start, end } => {
                owned(t) && t.due_date.is_some_and(|d| d >= *start && d <= *end)
            }
            Self::Waiting => waiting(t),
        }
    }
}

fn task_subject<'a>(
    f: &Finding,
    tasks: &'a [TaskItem],
    meetings: &[Meeting],
) -> Option<&'a TaskItem> {
    for subject in &f.subject_refs {
        let Some((meeting, index)) = subject
            .strip_prefix("task:")
            .and_then(|s| s.rsplit_once(':'))
        else {
            continue;
        };
        // The source fingerprint survives summary reordering. A reused index is never enough.
        let fingerprint = f
            .subject_keys
            .iter()
            .find_map(|key| key.strip_prefix(&format!("{subject}@")));
        if let Some(fingerprint) = fingerprint {
            let m = meetings.iter().find(|m| m.id == meeting)?;
            let matched: Vec<_> = m
                .summary
                .todo
                .iter()
                .enumerate()
                .filter(|(_, todo)| {
                    super::store_io::fingerprint([todo.task.as_str()]) == fingerprint
                })
                .collect();
            if matched.len() == 1 {
                return tasks
                    .iter()
                    .find(|t| t.meeting_id == meeting && t.index == matched[0].0);
            }
        } else if let Some(action) = &f.action
            && action.meeting_id == meeting
            && index.parse::<usize>().ok() == Some(action.todo_index)
        {
            let matched: Vec<_> = tasks
                .iter()
                .filter(|t| t.meeting_id == meeting && t.task == action.expected_task)
                .collect();
            if matched.len() == 1 {
                return Some(matched[0]);
            }
        }
    }
    if f.subject_refs.is_empty()
        && let Some(action) = &f.action
    {
        let matched: Vec<_> = tasks
            .iter()
            .filter(|t| t.meeting_id == action.meeting_id && t.task == action.expected_task)
            .collect();
        if matched.len() == 1 {
            return Some(matched[0]);
        }
    }
    None
}
fn finding_key(report: &DailyCheck, index: usize) -> String {
    let f = &report.content.findings[index];
    if report.schema_version == 2 && !f.id.is_empty() {
        f.id.clone()
    } else {
        format!("v1:{index}")
    }
}

pub fn aggregate(
    reports: &[Report],
    tasks: &[TaskItem],
    meetings: &[Meeting],
    records: &[MemoryRecord],
    proposals: &[PendingProposal],
    today: NaiveDate,
) -> Vec<Attention> {
    let mut reports: Vec<_> = reports.iter().collect();
    reports.sort_by_key(|r| std::cmp::Reverse(r.report.date));
    let mut seen = BTreeSet::new();
    let mut resolved = BTreeSet::new();
    let mut out = HashMap::new();
    for snapshot in reports {
        let report = &snapshot.report;
        for id in &report.content.comparison.resolved {
            resolved.insert(id.clone());
        }
        let mut findings: Vec<_> = report.content.findings.iter().enumerate().collect();
        findings.sort_by_key(|(i, f)| {
            (
                match f.severity {
                    Severity::High => 0,
                    Severity::Medium => 1,
                    Severity::Low => 2,
                },
                *i,
            )
        });
        for (index, f) in findings {
            let finding = finding_key(report, index);
            if resolved.contains(&finding) {
                continue;
            }
            let task = task_subject(f, tasks, meetings);
            let memory = f
                .subject_refs
                .iter()
                .find_map(|s| s.strip_prefix("memory:"));
            let proposal = f
                .subject_refs
                .iter()
                .find_map(|s| s.strip_prefix("proposal:"));
            let record = memory.and_then(|id| records.iter().find(|r| r.id == id));
            let pending = proposal.and_then(|id| proposals.iter().find(|p| p.proposal.id == id));
            let citation = f
                .evidence
                .iter()
                .find_map(|s| s.strip_prefix("[[").and_then(|s| s.strip_suffix("]]")))
                .and_then(|s| s.split('@').next());
            let has_task_subject =
                f.subject_refs.iter().any(|s| s.starts_with("task:")) || f.action.is_some();
            let route = if let Some(t) = task {
                task_route(t)
            } else if let Some(id) = memory {
                Route::Memory(id.into())
            } else if let Some(id) = proposal {
                Route::Proposal(id.into())
            } else if !has_task_subject
                && let Some(id) = citation.filter(|id| meetings.iter().any(|m| m.id == *id))
            {
                Route::Meeting(id.into())
            } else {
                Route::Review {
                    date: report.date,
                    finding: finding.clone(),
                }
            };
            let key = if matches!(route, Route::Meeting(_)) && !f.id.is_empty() {
                format!("question:{}", f.id)
            } else if matches!(route, Route::Review { .. }) && !f.subject_keys.is_empty() {
                let mut subjects = f.subject_keys.clone();
                subjects.sort();
                format!("subjects:{}", subjects.join("|"))
            } else {
                route.key()
            };
            if !seen.insert(key.clone())
                || !seen.insert(if report.schema_version == 1 {
                    format!("finding:{}:{finding}", report.date)
                } else {
                    format!("finding:{finding}")
                })
            {
                continue;
            }
            if !snapshot.open.contains(&index)
                || task.is_some_and(|t| {
                    t.done
                        || (matches!(f.kind, FindingKind::TaskWithoutOwner)
                            && (t.mine == Some(true)
                                || t.owner
                                    .as_ref()
                                    .is_some_and(|owner| !owner.trim().is_empty())))
                })
                || record.is_some_and(|r| r.status != RecordStatus::Active)
                || pending.is_some_and(|p| p.status != ProposalStatus::Pending)
            {
                continue;
            }
            let missing = (memory.is_some() && record.is_none())
                || (proposal.is_some() && pending.is_none())
                || (has_task_subject && task.is_none());
            let route = if missing {
                Route::Review {
                    date: report.date,
                    finding,
                }
            } else {
                route
            };
            let due = task.and_then(|t| t.due_date);
            let reason = if missing {
                Reason::Unavailable
            } else if task.is_some_and(|t| t.is_overdue(today)) {
                Reason::Overdue
            } else if due == Some(today) {
                Reason::DueToday
            } else if f.change == FindingChange::Changed {
                Reason::ReviewUpdate
            } else {
                match f.kind {
                    FindingKind::TaskWithoutOwner => Reason::MissingOwner,
                    FindingKind::ConflictingDecisions | FindingKind::KbContradictsMeeting => {
                        Reason::ConflictingDecisions
                    }
                    _ => Reason::FollowUp,
                }
            };
            let title = task
                .map(|t| {
                    if t.title.trim().is_empty() {
                        t.task.clone()
                    } else {
                        t.title.clone()
                    }
                })
                .or_else(|| record.and_then(|r| r.text.clone()))
                .or_else(|| pending.map(|p| p.proposal.text.clone()))
                .unwrap_or_else(|| {
                    if matches!(route, Route::Meeting(_)) {
                        "Review a question in Meeting summary".into()
                    } else {
                        "Review a follow-up".into()
                    }
                });
            let context = task.map(|t| t.meeting_title.clone()).unwrap_or_else(|| {
                if memory.is_some() || proposal.is_some() {
                    "Memory".into()
                } else {
                    "Daily check".into()
                }
            });
            let actionable = !matches!(route, Route::Review { .. });
            let rank = if matches!(f.severity, Severity::High) && actionable {
                0
            } else if task.is_some_and(|t| owned(t) && t.is_overdue(today)) {
                1
            } else if task.is_some_and(|t| due_week(t, today)) {
                2
            } else if f.change == FindingChange::Changed {
                3
            } else {
                4
            };
            out.insert(
                key,
                Attention {
                    route,
                    title,
                    context,
                    reason,
                    rank,
                    due,
                },
            );
        }
    }
    for t in tasks
        .iter()
        .filter(|t| owned(t) && (t.is_overdue(today) || due_week(t, today)))
    {
        let route = task_route(t);
        let key = route.key();
        if seen.contains(&key) {
            continue;
        }
        out.entry(key).or_insert_with(|| Attention {
            route,
            title: if t.title.trim().is_empty() {
                t.task.clone()
            } else {
                t.title.clone()
            },
            context: t.meeting_title.clone(),
            reason: if t.is_overdue(today) {
                Reason::Overdue
            } else if t.due_date == Some(today) {
                Reason::DueToday
            } else {
                Reason::Due(t.due_date.unwrap())
            },
            rank: if t.is_overdue(today) { 1 } else { 2 },
            due: t.due_date,
        });
    }
    let mut result: Vec<_> = out.into_values().collect();
    result.sort_by_key(|a| (a.rank, a.due.unwrap_or(NaiveDate::MAX), a.route.key()));
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn week_boundaries_are_local_monday_to_sunday() {
        for (start, end) in [
            ("2026-11-11", "2026-11-15"),
            ("2027-01-03", "2027-01-03"),
            ("2026-12-28", "2027-01-03"),
        ] {
            assert_eq!(
                week_end(NaiveDate::parse_from_str(start, "%Y-%m-%d").unwrap()).to_string(),
                end
            );
        }
    }
    fn date(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
    }
    fn meeting(n: usize) -> Meeting {
        Meeting {
            folder: Default::default(),
            device: None,
            id: "synthetic-meeting".into(),
            title: "Paper workshop".into(),
            diary_title: "Paper workshop".into(),
            summary: super::super::Summary {
                schema: 2,
                title: "Paper workshop".into(),
                todo: (0..n)
                    .map(|i| super::super::Todo {
                        id: format!("task-{i}"),
                        task: format!("Check paper sample {i}"),
                        title: format!("Check sample {i}"),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            },
            summary_raw: String::new(),
            transcript_raw: String::new(),
            routed: None,
            routed_revision: String::new(),
            notes: String::new(),
            lines: Vec::new(),
            started: None,
            minutes: None,
            export_path: None,
            search: String::new(),
        }
    }
    fn tasks(m: &Meeting) -> Vec<TaskItem> {
        let mut tasks = super::super::build_tasks(
            std::slice::from_ref(m),
            &Default::default(),
            &Default::default(),
            &Default::default(),
            &Default::default(),
        );
        for task in &mut tasks {
            task.mine = Some(true);
        }
        tasks
    }
    fn finding(m: &Meeting, index: usize) -> Finding {
        let subject = format!("task:{}:{index}", m.id);
        serde_json::from_value(serde_json::json!({"id":format!("finding-{index}"), "kind":"missing_followup", "severity":"high", "title":"DEC-123 revision abcdef123456", "explanation":"Internal model evidence", "evidence":[format!("[[{}@00:05]]", m.id)], "subject_refs":[subject], "subject_keys":[format!("{subject}@{}", super::super::store_io::fingerprint([m.summary.todo[index].task.as_str()]))], "evidence_keys":["synthetic"], "material_state":"synthetic"})).unwrap()
    }
    fn report(day: &str, findings: Vec<Finding>) -> Report {
        let open = (0..findings.len()).collect();
        Report {
            report: DailyCheck {
                schema_version: 2,
                date: date(day),
                runner: "Synthetic fixture".into(),
                prompt_bytes: 0,
                elapsed_seconds: 0.0,
                meeting_count: 1,
                task_count: findings.len(),
                kb: super::super::daily_check::KbSnapshot {
                    revision: "synthetic".into(),
                    captured_at: String::new(),
                    path: Default::default(),
                    stale: false,
                },
                warnings: Vec::new(),
                memory_updated: false,
                content: super::super::daily_check::CheckContent {
                    findings,
                    ..Default::default()
                },
            },
            open,
        }
    }
    #[test]
    fn fifty_plus_items_dedupe_across_days_and_sort_stably() {
        let m = meeting(60);
        let tasks = tasks(&m);
        let findings: Vec<_> = (0..60).map(|i| finding(&m, i)).collect();
        let old = report("2026-11-09", findings.clone());
        let new = report("2026-11-11", findings);
        let results = aggregate(&[old, new], &tasks, &[m], &[], &[], date("2026-11-11"));
        assert_eq!(results.len(), 60);
        assert!(results.iter().all(|a| !a.title.contains("DEC-123")));
        let keys: Vec<_> = results.iter().map(|a| a.route.key()).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
    }
    #[test]
    fn latest_triage_done_and_explicit_resolution_suppress_old_findings() {
        let m = meeting(1);
        let mut tasks = tasks(&m);
        let f = finding(&m, 0);
        let old = report("2026-11-09", vec![f.clone()]);
        let mut new = report("2026-11-11", vec![f.clone()]);
        new.open.clear();
        tasks[0].due_date = Some(date("2026-11-10"));
        assert!(
            aggregate(
                &[old.clone(), new],
                &tasks,
                std::slice::from_ref(&m),
                &[],
                &[],
                date("2026-11-11")
            )
            .is_empty()
        );
        tasks[0].done = true;
        assert!(
            aggregate(
                std::slice::from_ref(&old),
                &tasks,
                std::slice::from_ref(&m),
                &[],
                &[],
                date("2026-11-11")
            )
            .is_empty()
        );
        tasks[0].done = false;
        tasks[0].due_date = None;
        let mut resolved = report("2026-11-11", Vec::new());
        resolved.report.content.comparison.resolved.push(f.id);
        assert!(aggregate(&[old, resolved], &tasks, &[m], &[], &[], date("2026-11-11")).is_empty());
    }
    #[test]
    fn regenerated_task_indexes_are_guarded_and_removed_sources_open_review() {
        let mut m = meeting(2);
        let f = finding(&m, 0);
        m.summary.todo.swap(0, 1);
        let tasks = tasks(&m);
        let r = report("2026-11-11", vec![f]);
        let result = aggregate(
            std::slice::from_ref(&r),
            &tasks,
            std::slice::from_ref(&m),
            &[],
            &[],
            date("2026-11-11"),
        );
        assert!(
            matches!(&result[0].route, Route::Task { stable, index: 1, .. } if stable == "task-0")
        );
        let result = aggregate(&[r], &[], &[m], &[], &[], date("2026-11-11"));
        assert!(matches!(result[0].route, Route::Review { .. }));
        assert_eq!(result[0].reason, Reason::Unavailable);
    }
    #[test]
    fn ownership_and_waiting_are_explicit() {
        let m = meeting(1);
        let mut tasks = tasks(&m);
        let t = &mut tasks[0];
        t.due_date = Some(date("2026-11-13"));
        assert!(due_week(t, date("2026-11-11")));
        t.mine = None;
        assert!(!due_week(t, date("2026-11-11")));
        t.mine = Some(true);
        t.status = TaskStatus::Waiting;
        assert!(!waiting(t));
        t.waiting_on = Some(super::super::WaitingOn {
            task_id: None,
            text: Some("Venue reply".into()),
        });
        assert!(waiting(t));
        t.done = true;
        assert!(!waiting(t));
        assert!(!due_week(t, date("2026-11-11")));
    }
    #[test]
    fn memory_and_proposal_routes_respect_current_source_status() {
        let m = meeting(1);
        let mut f = finding(&m, 0);
        f.subject_refs = vec!["memory:claim-sample".into()];
        f.subject_keys = f.subject_refs.clone();
        let mut record: MemoryRecord = serde_json::from_value(serde_json::json!({"id":"claim-sample", "type":"decision", "text":"Use a folded paper format.", "sources":[], "status":"active", "basis":"source_supported", "updated_at":"2026-11-11T08:00:00Z", "source_fingerprint":"synthetic"})).unwrap();
        let r = report("2026-11-11", vec![f.clone()]);
        let result = aggregate(
            std::slice::from_ref(&r),
            &[],
            std::slice::from_ref(&m),
            std::slice::from_ref(&record),
            &[],
            date("2026-11-11"),
        );
        assert!(matches!(&result[0].route, Route::Memory(id) if id == "claim-sample"));
        record.status = RecordStatus::Retired;
        assert!(
            aggregate(
                &[r],
                &[],
                std::slice::from_ref(&m),
                &[record],
                &[],
                date("2026-11-11")
            )
            .is_empty()
        );
        f.subject_refs = vec!["proposal:proposal-sample".into()];
        f.subject_keys = f.subject_refs.clone();
        let mut proposal: PendingProposal = serde_json::from_value(serde_json::json!({"id":"proposal-sample", "op":"add", "text":"Keep exercises short.", "sources":[], "rationale":"Review the outcome", "status":"pending", "created_at":"2026-11-11T08:00:00Z"})).unwrap();
        let r = report("2026-11-11", vec![f]);
        let result = aggregate(
            std::slice::from_ref(&r),
            &[],
            std::slice::from_ref(&m),
            &[],
            std::slice::from_ref(&proposal),
            date("2026-11-11"),
        );
        assert!(matches!(&result[0].route, Route::Proposal(id) if id == "proposal-sample"));
        for status in [ProposalStatus::Accepted, ProposalStatus::Rejected] {
            proposal.status = status;
            assert!(
                aggregate(
                    std::slice::from_ref(&r),
                    &[],
                    std::slice::from_ref(&m),
                    &[],
                    std::slice::from_ref(&proposal),
                    date("2026-11-11")
                )
                .is_empty()
            );
        }
    }
    #[test]
    fn expired_snooze_reenters_the_aggregate() {
        use super::super::check_state::{TriageEntry, TriageFile, TriageStatus};
        let m = meeting(1);
        let tasks = tasks(&m);
        let f = finding(&m, 0);
        let mut triage = TriageFile::default();
        triage.findings.insert(
            f.id.clone(),
            TriageEntry {
                finding_id: f.id.clone(),
                status: TriageStatus::Snoozed,
                updated_at: "2026-11-11T08:00:00Z".into(),
                snooze_until: Some(date("2026-11-12")),
                evidence_fingerprint: super::super::check_v2::evidence_fingerprint(&f),
                evidence_keys: f.evidence_keys.clone(),
                note: None,
            },
        );
        for (today, count) in [("2026-11-11", 0), ("2026-11-12", 1)] {
            let mut r = report(today, vec![f.clone()]);
            if triage.status_for(&f, date(today)) != TriageStatus::Open {
                r.open.clear();
            }
            assert_eq!(
                aggregate(
                    &[r],
                    &tasks,
                    std::slice::from_ref(&m),
                    &[],
                    &[],
                    date(today)
                )
                .len(),
                count
            );
        }
    }
    #[test]
    fn legacy_findings_without_a_shared_source_are_date_scoped() {
        let m = meeting(1);
        let mut f = finding(&m, 0);
        f.id.clear();
        f.subject_refs.clear();
        f.subject_keys.clear();
        f.evidence.clear();
        let mut older = report("2026-11-09", vec![f.clone()]);
        older.report.schema_version = 1;
        let mut newer = report("2026-11-11", vec![f]);
        newer.report.schema_version = 1;
        assert_eq!(
            aggregate(&[older, newer], &[], &[], &[], &[], date("2026-11-11")).len(),
            2
        );
    }
    #[test]
    fn assigning_an_owner_resolves_the_old_owner_gap() {
        let m = meeting(1);
        let mut tasks = tasks(&m);
        let mut f = finding(&m, 0);
        f.kind = FindingKind::TaskWithoutOwner;
        let r = report("2026-11-11", vec![f]);
        assert!(
            aggregate(
                std::slice::from_ref(&r),
                &tasks,
                std::slice::from_ref(&m),
                &[],
                &[],
                date("2026-11-11")
            )
            .is_empty()
        );
        tasks[0].mine = None;
        tasks[0].owner = None;
        let results = aggregate(&[r], &tasks, &[m], &[], &[], date("2026-11-11"));
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].reason, Reason::MissingOwner);
    }
    #[test]
    fn legacy_task_identity_survives_reordering_but_not_replacement() {
        let m = meeting(1);
        let mut t = tasks(&m).remove(0);
        t.stable_id.clear();
        let original = task_route(&t).key();
        t.index = 8;
        assert_eq!(task_route(&t).key(), original);
        t.task = "A different paper sample task".into();
        assert_ne!(task_route(&t).key(), original);
    }
}
