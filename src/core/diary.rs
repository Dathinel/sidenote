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
pub fn task_route(t: &TaskItem) -> Route {
    Route::Task {
        meeting: t.meeting_id.clone(),
        stable: t.stable_id.clone(),
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
            || t.waiting_on.is_some())
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
        for (index, f) in report.content.findings.iter().enumerate() {
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
            let key = route.key();
            if !seen.insert(key.clone()) || !seen.insert(format!("finding:{finding}")) {
                continue;
            }
            if !snapshot.open.contains(&index)
                || task.is_some_and(|t| t.done)
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
}
