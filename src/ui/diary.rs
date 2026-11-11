//! The Diary: one entry per day, newest first. Each entry is a short paragraph about the day's
//! meetings, the meetings as compact links, and folded sections with counts: tasks created,
//! memories created, tasks done and what the daily check wants you to look at. The paragraph
//! is the `day_summary` the daily check wrote in its one run the morning after; a day without
//! one joins the first sentence of each meeting's own summary. Nothing here calls a model.
//! The newest check's findings open the same panel as the full report (Apply, Dismiss,
//! Snooze). Days with nothing are left out.
use super::{
    State, check,
    list::text_title,
    memory::Tab,
    roles,
    tasks::chevron,
    theme::Palette,
    widgets::{label, link, pressable},
};
use chrono::{DateTime, Datelike, Duration, Local, Months, NaiveDate};
use rune::{El, Key, KeyEvent, Tx, col, el, row};
use rune_kit::Kit;
use sidenote::core::{self, TaskItem, memory::ProposalStatus};
use std::collections::{BTreeSet, HashMap};

/// Days added by "Show earlier days".
pub const PAGE: usize = 10;
/// Rows listed in an open section before "and N more".
const MOST: usize = 12;

/// One day with something in it.
struct Day<'a> {
    date: NaiveDate,
    meetings: Vec<&'a core::Meeting>,
    created: Vec<&'a TaskItem>,
    done: Vec<&'a TaskItem>,
    /// Claims accepted into memory that day: record id and text.
    memories: Vec<(String, String)>,
    proposals: usize,
    check: bool,
    decisions: Vec<(String, String)>,
}

/// The folded sections of a day.
#[derive(Clone, Copy)]
pub enum Part {
    Outcomes,
    Summary,
    Tasks,
    Memories,
    Done,
    Attention,
    Low,
}

impl Part {
    fn name(self) -> &'static str {
        match self {
            Self::Outcomes => "outcomes",
            Self::Summary => "summary",
            Self::Tasks => "tasks",
            Self::Memories => "memories",
            Self::Done => "done",
            Self::Attention => "attention",
            Self::Low => "low",
        }
    }
}

/// The key of one day's section in `CheckState::open_parts`.
pub fn part_key(date: NaiveDate, part: Part) -> String {
    format!("{date}:{}", part.name())
}

fn local_day(at: &str) -> Option<NaiveDate> {
    DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|t| t.with_timezone(&Local).date_naive())
}

/// A claim the owner accepted into memory: the event that creates a record.
fn created(op: &str) -> bool {
    op == "accept_proposal"
}

/// Every day with content, newest first.
fn days(s: &State) -> Vec<NaiveDate> {
    let f = &s.config.features;
    let mut dates = BTreeSet::new();
    dates.extend(s.check.diary_selected);
    for m in &s.meetings {
        if let Some(t) = m.started {
            dates.insert(t.date_naive());
        }
    }
    if f.tasks {
        dates.extend(s.check.done_on.values().copied());
        dates.extend(
            s.tasks
                .all
                .iter()
                .filter(|t| t.manual)
                .filter_map(|t| t.meeting_date),
        );
    }
    if f.memory {
        dates.extend(
            s.memory
                .events
                .iter()
                .filter(|e| created(&e.operation))
                .filter_map(|e| local_day(&e.at)),
        );
        dates.extend(
            s.memory
                .proposals
                .iter()
                .filter(|p| p.status == ProposalStatus::Pending)
                .filter_map(|p| local_day(&p.created_at)),
        );
    }
    if f.daily_check {
        dates.extend(s.check.dates.iter().copied());
    }
    dates.into_iter().rev().collect()
}

/// What one day holds. Only the days on screen are gathered.
fn day(s: &State, date: NaiveDate) -> Day<'_> {
    let f = &s.config.features;
    let mut meetings: Vec<&core::Meeting> = s
        .meetings
        .iter()
        .filter(|m| m.started.is_some_and(|t| t.date_naive() == date))
        .collect();
    meetings.sort_by_key(|m| m.started);
    let (mut created_tasks, mut done) = (Vec::new(), Vec::new());
    if f.tasks {
        let ids: BTreeSet<&str> = meetings.iter().map(|m| m.id.as_str()).collect();
        created_tasks = s
            .tasks
            .all
            .iter()
            .filter(|t| {
                if t.manual {
                    t.meeting_date == Some(date)
                } else {
                    ids.contains(t.meeting_id.as_str())
                }
            })
            .collect();
        // Yours first, then in the meeting's order.
        created_tasks.sort_by_key(|t| (t.mine != Some(true), t.meeting_rank, t.index));
        done = s
            .check
            .done_on
            .iter()
            .filter(|(_, d)| **d == date)
            .filter_map(|((id, index), _)| {
                s.tasks
                    .all
                    .iter()
                    .find(|t| t.done && !t.manual && &t.meeting_id == id && t.index == *index)
            })
            .collect();
        done.sort_by(|a, b| (&a.meeting_id, a.index).cmp(&(&b.meeting_id, b.index)));
    }
    let mut memories = Vec::new();
    let mut proposals = 0;
    if f.memory {
        for e in &s.memory.events {
            if !created(&e.operation) || local_day(&e.at) != Some(date) {
                continue;
            }
            if let Some(text) = s
                .memory
                .records
                .iter()
                .find(|r| r.id == e.record_id)
                .and_then(|r| r.text.clone())
            {
                memories.push((e.record_id.clone(), text));
            }
        }
        proposals = s
            .memory
            .proposals
            .iter()
            .filter(|p| {
                p.status == ProposalStatus::Pending && local_day(&p.created_at) == Some(date)
            })
            .count();
    }
    Day {
        date,
        meetings: meetings.clone(),
        created: created_tasks,
        done,
        memories,
        proposals,
        check: f.daily_check && s.check.dates.contains(&date),
        decisions: meetings_decisions(&meetings),
    }
}

fn meetings_decisions(meetings: &[&core::Meeting]) -> Vec<(String, String)> {
    let mut seen = BTreeSet::new();
    meetings
        .iter()
        .flat_map(|m| {
            m.summary.topics.iter().flat_map(move |topic| {
                topic
                    .points
                    .iter()
                    .filter(|point| {
                        matches!(
                            point.kind.trim().to_lowercase().as_str(),
                            "decision" | "decided"
                        )
                    })
                    .map(move |point| {
                        (
                            m.id.clone(),
                            display_text(&point.text, "Review the decision in Meeting summary"),
                        )
                    })
            })
        })
        .filter(|(id, text)| seen.insert((id.clone(), text.to_lowercase())))
        .collect()
}

/// Free-form model copy containing technical evidence is withheld, rather than partially scrubbed.
fn display_text(value: &str, fallback: &str) -> String {
    static INTERNAL: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)(?:\[\[|\b(?:finding|claim|proposal|record|decision)[-_:][a-z0-9_-]+|\b(?:DEC|D)[-_]\d+\b|\b[a-f0-9]{8,}\b|\b\d{13}-\d+\b|\brev(?:ision)?\s*[:#=])").unwrap()
    });
    if INTERNAL.is_match(value) {
        return fallback.into();
    }
    value.trim().replace('\u{2014}', ",")
}

fn short_summary(value: &str) -> (String, bool) {
    let mut end = 0;
    for _ in 0..2 {
        let rest = value[end..].trim_start();
        if rest.is_empty() {
            break;
        }
        let sentence = first_sentence(rest);
        end = (value.len() - rest.len() + sentence.len()).min(value.len());
    }
    // Keep a very long first sentence readable without changing its stored value.
    let limit = value
        .char_indices()
        .nth(280)
        .map(|(i, _)| i)
        .unwrap_or(value.len());
    end = end.min(limit);
    (value[..end].trim().to_string(), end < value.len())
}

/// The first sentence of a line: up to the first full stop, question or exclamation mark that
/// ends a word.
fn first_sentence(line: &str) -> String {
    let line = line.trim();
    let end = line
        .char_indices()
        .find(|&(i, c)| {
            matches!(c, '.' | '?' | '!')
                && line[i + c.len_utf8()..]
                    .chars()
                    .next()
                    .is_none_or(char::is_whitespace)
        })
        .map_or(line.len(), |(i, c)| i + c.len_utf8());
    let mut out = line[..end].to_string();
    if !out.ends_with(['.', '?', '!']) && !out.is_empty() {
        out.push('.');
    }
    out
}

/// The day in a few plain sentences: the check's summary of it, else the first sentence of
/// each meeting's own summary.
fn summary(s: &State, d: &Day) -> String {
    if let Some(text) = s.check.day_summaries.get(&d.date) {
        return text.clone();
    }
    d.meetings
        .iter()
        .filter_map(|m| m.summary.context.first())
        .map(|l| first_sentence(l))
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

// ---- pieces of an entry --------------------------------------------------------------------

/// A meeting as one compact link: its time, its title and its length.
fn meeting_row(m: &core::Meeting, date: NaiveDate, p: Palette) -> El {
    let time = m
        .started
        .map(|t| t.format("%H:%M").to_string())
        .unwrap_or_else(|| "Time unknown".into());
    let minutes = m
        .minutes
        .filter(|n| *n > 0)
        .map(|n| format!("{n} min"))
        .unwrap_or_default();
    let id = m.id.clone();
    let origin = format!("diary-{date}");
    pressable(
        row()
            .w_full()
            .py(14.0)
            .gap(8.0)
            .items_start()
            .child(roles::META.wrap(time, p.faint).w(48.0))
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(roles::META_GAP)
                    .child(
                        roles::NAME
                            .wrap(m.diary_title.replace('\u{2014}', ","), p.ink)
                            .w_full(),
                    )
                    .children((!m.summary.people.is_empty()).then(|| {
                        roles::META
                            .wrap(
                                m.summary.people.join(", ").replace('\u{2014}', ","),
                                p.faint,
                            )
                            .w_full()
                    })),
            )
            .child(roles::META.wrap(minutes, p.faint)),
    )
    .key(format!("diary-meeting-{}", m.id))
    .label(format!(
        "Open the meeting: {}",
        m.diary_title.replace('\u{2014}', ",")
    ))
    .w_full()
    .px(8.0)
    .radius(6.0)
    .hover(p.chip)
    .on_press_tx(move |s: &mut State, tx| s.open_check_citation(&id, origin.clone(), tx))
}

/// A row inside a section, lined up with the section's name.
fn item(mark: El, words: El) -> El {
    row()
        .w_full()
        .gap(8.0)
        .items_start()
        .pl(8.0)
        .py(3.0)
        .child(col().w(12.0).pt(1.0).items_center().child(mark))
        .child(words.grow(1.0).min_w(0.0))
}

fn tick(done: bool, p: Palette) -> El {
    if done {
        label("\u{2713}", 12.0, 700.0, p.ok).mt(1.0)
    } else {
        el().size(9.0, 9.0).radius(5.0).border(1.5, p.soft).mt(6.0)
    }
}

fn more(n: usize, what: &str, p: Palette) -> El {
    roles::META
        .wrap(format!("and {n} more {what}"), p.faint)
        .pl(28.0)
        .py(2.0)
}

fn task_rows(tasks: &[&TaskItem], with_owner: bool, p: Palette) -> El {
    let mut out = col().w_full().pb(6.0);
    for t in tasks.iter().take(MOST) {
        let who = match (t.mine, t.owner.as_deref()) {
            (Some(true), _) => "You".to_string(),
            (_, Some(o)) => o.to_string(),
            _ => String::new(),
        };
        let mut facts = vec![t.meeting_title.clone()];
        if with_owner {
            facts.insert(0, who);
        }
        let id = t.meeting_id.clone();
        let stable = t.stable_id.clone();
        let index = t.index;
        out = out.child(
            pressable(item(
                tick(t.done, p),
                col()
                    .w_full()
                    .child(roles::SMALL.wrap(
                        display_text(
                            if t.title.trim().is_empty() {
                                &t.task
                            } else {
                                &t.title
                            },
                            "Review this task",
                        ),
                        if t.done { p.soft } else { p.ink },
                    ))
                    .child(roles::META.wrap(display_text(&facts.join(roles::DOT), ""), p.faint)),
            ))
            .key(format!(
                "diary-task-{}-{}-{with_owner}",
                t.meeting_id, t.index
            ))
            .label("Open this task")
            .hover(p.chip)
            .on_press(move |s: &mut State| s.diary_task(&id, &stable, index)),
        );
    }
    if tasks.len() > MOST {
        out = out.child(more(tasks.len() - MOST, "on the Tasks page", p));
    }
    out
}

fn memory_rows(d: &Day, p: Palette) -> El {
    let mut out = col().w_full().pb(6.0);
    for (id, text) in d.memories.iter().take(MOST) {
        let id = id.clone();
        out = out.child(
            pressable(item(
                el().size(5.0, 5.0).radius(3.0).fill(p.faint).mt(8.0),
                roles::SMALL.wrap(display_text(text, "Open accepted memory"), p.ink),
            ))
            .key(format!("diary-memory-{}-{id}", d.date))
            .label("Open this memory claim")
            .radius(6.0)
            .hover(p.chip)
            .on_press(move |s: &mut State| {
                s.open_memory();
                s.open_record(&id);
            }),
        );
    }
    if d.memories.len() > MOST {
        out = out.child(more(d.memories.len() - MOST, "in Memory", p));
    }
    if d.proposals > 0 {
        let n = d.proposals;
        out = out.child(
            link(
                format!(
                    "{n} currently pending {} to review \u{203a}",
                    if n == 1 { "proposal" } else { "proposals" }
                ),
                p,
            )
            .key(format!("diary-proposals-{}", d.date))
            .label("Review the memory proposals")
            .ml(24.0)
            .mt(2.0)
            .on_press(|s: &mut State| {
                s.open_memory();
                s.memory.tab = Tab::Review;
            }),
        );
    }
    out
}

fn day_head(d: &Day, today: NaiveDate, p: Palette) -> El {
    let mut name = d.date.format("%A %-d %B").to_string();
    if d.date.year() != today.year() {
        name.push_str(&format!(" {}", d.date.year()));
    }
    let when = if d.date == today {
        Some("Today")
    } else if today.pred_opt() == Some(d.date) {
        Some("Yesterday")
    } else {
        None
    };
    let mut head = row().w_full().items_center().flex_wrap().gap(12.0);
    if let Some(w) = when {
        head = head.child(roles::SMALL.wrap(w, p.faint));
    }
    head.child(roles::SECTION.wrap(name, p.ink))
}

fn entry(s: &State, _k: &Kit, d: &Day, _newest_check: bool, p: Palette) -> El {
    let mut out = col()
        .key(format!("diary-{}", d.date))
        .w_full()
        .gap(12.0)
        .child(day_head(d, s.check.today, p));
    let line = display_text(&summary(s, d), "");
    if !line.is_empty() {
        let (short, more) = short_summary(&line);
        let key = part_key(d.date, Part::Summary);
        let on = s.check.open_parts.contains(&key);
        out = out.child(
            roles::READ
                .wrap(if on { line } else { short }, p.soft)
                .max_w(720.0),
        );
        if more {
            out = out.child(disclosure(
                key,
                if on { "Show less" } else { "Show more" },
                on,
                p,
            ));
        }
    } else if d.meetings.is_empty() {
        out = out.child(roles::SMALL.wrap("No meetings", p.soft));
    }
    let mut list = col().w_full();
    for m in &d.meetings {
        list = list.child(meeting_row(m, d.date, p));
    }
    out = out.child(list);
    let mut counts = Vec::new();
    for (n, singular, plural) in [
        (d.created.len(), "task", "tasks"),
        (d.decisions.len(), "decision", "decisions"),
        (d.memories.len(), "memory accepted", "memories accepted"),
        (d.proposals, "memory proposal", "memory proposals"),
        (d.done.len(), "completed task", "completed tasks"),
    ] {
        if n > 0 {
            counts.push(format!("{n} {}", if n == 1 { singular } else { plural }));
        }
    }
    if !counts.is_empty() || d.check {
        let key = part_key(d.date, Part::Outcomes);
        let on = s.check.open_parts.contains(&key);
        out = out.child(disclosure(
            key,
            &if counts.is_empty() {
                "Daily check".into()
            } else {
                counts.join(roles::DOT)
            },
            on,
            p,
        ));
        if on {
            let mut detail = col()
                .w_full()
                .gap(8.0)
                .pl(8.0)
                .key(format!("diary-outcomes-{}", d.date));
            if !d.created.is_empty() {
                detail = detail
                    .child(roles::GROUP.wrap("Tasks from this day", p.soft))
                    .child(task_rows(&d.created, true, p));
            }
            if !d.decisions.is_empty() {
                detail = detail.child(roles::GROUP.wrap("Decisions", p.soft));
                for (n, (id, text)) in d.decisions.iter().enumerate() {
                    let id = id.clone();
                    let origin = format!("diary-{}", d.date);
                    detail = detail.child(
                        pressable(roles::SMALL.wrap(text, p.ink).py(6.0).w_full())
                            .key(format!("diary-decision-{}-{n}", d.date))
                            .label("Open decision in Meeting summary")
                            .hover(p.chip)
                            .on_press_tx(move |s: &mut State, tx| {
                                s.open_check_citation(&id, origin.clone(), tx)
                            }),
                    );
                }
            }
            if !d.memories.is_empty() || d.proposals > 0 {
                detail = detail
                    .child(roles::GROUP.wrap("Memory accepted and currently pending", p.soft))
                    .child(memory_rows(d, p));
            }
            if !d.done.is_empty() {
                detail = detail
                    .child(roles::GROUP.wrap("Completed", p.soft))
                    .child(task_rows(&d.done, false, p));
            }
            if d.check {
                detail = detail.child(check::day_line(s, d.date, p));
            }
            out = out.child(detail);
        }
    }
    out
}

fn disclosure(key: String, words: &str, open: bool, p: Palette) -> El {
    let toggle = key.clone();
    pressable(
        row()
            .w_full()
            .items_center()
            .gap(8.0)
            .child(chevron(open, p))
            .child(roles::META.wrap(words, p.soft).grow(1.0).min_w(0.0)),
    )
    .key(format!("diary-fold-{key}"))
    .label(words)
    .expanded(open)
    .py(8.0)
    .radius(6.0)
    .hover(p.chip)
    .on_press(move |s: &mut State| {
        if !s.check.open_parts.remove(&toggle) {
            s.check.open_parts.insert(toggle.clone());
        }
    })
}

// ---- the page ------------------------------------------------------------------------------

fn month_start(date: NaiveDate) -> NaiveDate {
    date.with_day(1).unwrap_or(date)
}

fn calendar(s: &State, p: Palette) -> El {
    use super::widgets::{ButtonRole, button};
    let selected = s.check.diary_selected.unwrap_or(s.check.today);
    let month = month_start(s.check.diary_month.unwrap_or(selected));
    let mut out = col()
        .w_full()
        .gap(8.0)
        .key("diary-calendar")
        .role("group")
        .label("Calendar. Dots mark days with meetings");
    out = out.child(
        row()
            .w_full()
            .items_center()
            .justify_between()
            .child(roles::GROUP.wrap(month.format("%B %Y").to_string(), p.ink))
            .child(
                row()
                    .gap(2.0)
                    .child(
                        button("\u{2039}", "diary-month-prev", ButtonRole::Ghost, p)
                            .label("Previous month")
                            .on_press(move |s: &mut State| {
                                s.check.diary_month = month.checked_sub_months(Months::new(1))
                            }),
                    )
                    .child(
                        button("\u{203a}", "diary-month-next", ButtonRole::Ghost, p)
                            .label("Next month")
                            .on_press(move |s: &mut State| {
                                s.check.diary_month = month.checked_add_months(Months::new(1))
                            }),
                    ),
            ),
    );
    let mut weekdays = row().w_full();
    for day in ["M", "T", "W", "T", "F", "S", "S"] {
        weekdays = weekdays.child(
            col()
                .grow(1.0)
                .items_center()
                .child(roles::META.wrap(day, p.faint)),
        );
    }
    out = out.child(weekdays);
    let meeting_days: BTreeSet<_> = s
        .meetings
        .iter()
        .filter_map(|m| m.started.map(|t| t.date_naive()))
        .collect();
    let offset = month.weekday().num_days_from_monday() as i64;
    for week in 0..6 {
        let mut line = row().w_full();
        let mut any = false;
        for weekday in 0..7 {
            let date = month + Duration::days(week * 7 + weekday - offset);
            let cell = if date.month() == month.month() {
                any = true;
                let dot = meeting_days.contains(&date);
                let action = pressable(
                    col()
                        .w_full()
                        .items_center()
                        .py(4.0)
                        .child(roles::META.wrap(
                            date.day().to_string(),
                            if selected == date { p.bg } else { p.ink },
                        ))
                        .child(el().size(3.0, 3.0).radius(2.0).fill(if dot {
                            if selected == date { p.bg } else { p.faint }
                        } else {
                            rune::Color::TRANSPARENT
                        })),
                )
                .key(format!("diary-date-{date}"))
                .label(format!(
                    "{}{}",
                    date.format("%A %-d %B %Y"),
                    if dot { ", meetings" } else { ", no meetings" }
                ))
                .checked(selected == date)
                .radius(6.0)
                .hover(p.chip)
                .on_key(calendar_key)
                .on_press(move |s: &mut State| s.diary_jump(date));
                if selected == date {
                    action.fill(p.ink)
                } else {
                    action
                }
            } else {
                el()
            };
            line = line.child(col().grow(1.0).min_w(0.0).child(cell));
        }
        if any {
            out = out.child(line);
        }
    }
    out.child(
        row()
            .w_full()
            .justify_between()
            .items_center()
            .child(roles::META.wrap("\u{2022} Meetings", p.faint))
            .child(
                button("Today", "diary-today", ButtonRole::Ghost, p)
                    .on_press(|s: &mut State| s.diary_jump(s.check.today)),
            ),
    )
}

fn calendar_key(s: &mut State, e: &KeyEvent, tx: &mut Tx) -> bool {
    let delta = match e.key {
        Key::Enter => {
            s.diary_jump(s.check.diary_selected.unwrap_or(s.check.today));
            return true;
        }
        Key::Left => -1,
        Key::Right => 1,
        Key::Up => -7,
        Key::Down => 7,
        Key::Escape => {
            tx.blur();
            tx.scroll_into_view("check-top");
            return true;
        }
        _ => return false,
    };
    let date = s.check.diary_selected.unwrap_or(s.check.today) + Duration::days(delta);
    s.check.diary_selected = Some(date);
    s.check.diary_month = Some(month_start(date));
    tx.scroll_into_view(format!("diary-date-{date}"));
    true
}

impl State {
    /// Dates each meeting task was ticked, from the tick journal. Read when the Diary opens.
    /// Ticks from before the journal have no time, so they never show under Done.
    pub fn refresh_diary(&mut self) {
        let mut times: HashMap<String, _> = HashMap::new();
        let mut done_on = HashMap::new();
        for t in self.tasks.all.iter().filter(|t| t.done && !t.manual) {
            let stamps = times
                .entry(t.meeting_id.clone())
                .or_insert_with(|| self.store.tick_times(&t.meeting_id).unwrap_or_default());
            let original = self
                .meetings
                .iter()
                .find(|m| m.id == t.meeting_id)
                .and_then(|m| m.summary.todo.get(t.index))
                .map(|todo| todo.task.as_str());
            if let Some(at) = original.and_then(|o| stamps.get(o)) {
                done_on.insert(
                    (t.meeting_id.clone(), t.index),
                    at.with_timezone(&Local).date_naive(),
                );
            }
        }
        self.check.done_on = done_on;
    }

    pub(super) fn diary_task(&mut self, id: &str, stable: &str, index: usize) {
        let target = self
            .tasks
            .all
            .iter()
            .find(|t| {
                t.meeting_id == id
                    && if stable.is_empty() {
                        t.index == index
                    } else {
                        t.stable_id == stable
                    }
            })
            .cloned();
        let Some(t) = target else {
            self.check.notice =
                Some("Source unavailable. Open Tasks to browse current work.".into());
            return;
        };
        if !self.close_panel() {
            self.check.notice =
                Some("Your task draft is kept. Save or cancel it in Tasks first.".into());
            return;
        }
        self.remember_diary();
        self.set_view(super::tasks::View::Tasks);
        if !self
            .tasks
            .items
            .iter()
            .any(|row| row.meeting_id == t.meeting_id && row.index == t.index)
        {
            self.tasks.items.push(t.clone());
        }
        self.tasks.diary_rows = None;
        self.tasks.diary_label = None;
        self.open_task(&t.meeting_id, t.index);
    }

    /// The day picker: show enough days to reach it, then scroll to its entry.
    pub(super) fn diary_jump(&mut self, date: NaiveDate) {
        self.check.diary_selected = Some(date);
        self.check.diary_month = Some(month_start(date));
        let index = days(self).iter().position(|d| *d == date).unwrap_or(0);
        // Historical summaries are read only when their date is visited.
        if !self.check.day_summaries.contains_key(&date) {
            for &report_date in &self.check.dates {
                if report_date < date {
                    continue;
                }
                if let Ok(report) = core::load_check(report_date)
                    && report.content.day_summary_date == Some(date)
                {
                    self.check
                        .day_summaries
                        .insert(date, report.content.day_summary);
                    break;
                }
            }
        }
        self.check.picker_open = false;
        self.check.shown = self.check.shown.max(index + 1);
        self.pending_scroll = Some(super::PendingScroll {
            key: format!("diary-{date}"),
            center: false,
            left: 3,
        });
    }
}

fn attention(s: &State) -> Vec<core::diary::Attention> {
    core::diary::aggregate(
        if s.config.features.daily_check {
            &s.check.diary_reports
        } else {
            &[]
        },
        if s.config.features.tasks {
            &s.tasks.all
        } else {
            &[]
        },
        &s.meetings,
        if s.config.features.memory {
            &s.memory.records
        } else {
            &[]
        },
        if s.config.features.memory {
            &s.memory.proposals
        } else {
            &[]
        },
        s.check.today,
    )
    .into_iter()
    .filter(|a| match a.route {
        core::diary::Route::Task { .. } => s.config.features.tasks,
        core::diary::Route::Memory(_) | core::diary::Route::Proposal(_) => s.config.features.memory,
        _ => true,
    })
    .collect()
}

impl State {
    fn remember_diary(&mut self) {
        self.check.diary_return = Some(format!(
            "diary-{}",
            self.check
                .diary_selected
                .unwrap_or_else(|| days(self).first().copied().unwrap_or(self.check.today))
        ));
    }
    fn diary_route(&mut self, route: &core::diary::Route, tx: &mut Tx) {
        use core::diary::Route;
        if self.memory.editing {
            self.check.notice =
                Some("Your Memory draft is kept. Save or cancel it in Memory first.".into());
            return;
        }
        if !self.close_panel() {
            self.check.notice =
                Some("Your task draft is kept. Save or cancel it in Tasks first.".into());
            return;
        }
        self.remember_diary();
        match route {
            Route::Task {
                meeting,
                stable,
                index,
            } => self.diary_task(meeting, stable, *index),
            Route::Memory(id) => {
                if !self.memory.records.iter().any(|r| r.id == *id) {
                    self.check.notice =
                        Some("Source unavailable. Open Memory to browse current claims.".into());
                    return;
                }
                self.open_memory();
                self.memory.tab = Tab::All;
                self.memory.scope = super::memory::Scope::Active;
                self.memory.project = None;
                self.memory.meeting = None;
                self.memory.search = rune::TextEdit::default();
                self.open_record(id);
            }
            Route::Proposal(id) => {
                if !self
                    .memory
                    .proposals
                    .iter()
                    .any(|r| r.proposal.id == *id && r.status == ProposalStatus::Pending)
                {
                    self.check.notice = Some(
                        "Source unavailable. Open Memory Review to browse current proposals."
                            .into(),
                    );
                    return;
                }
                self.open_memory();
                self.memory.tab = Tab::Review;
                self.memory.project = None;
                self.memory.meeting = None;
                self.memory.search = rune::TextEdit::default();
                self.open_record(id);
            }
            Route::Meeting(id) => self.open_check_citation(
                id,
                self.check.diary_return.clone().unwrap_or_default(),
                tx,
            ),
            Route::Review { date, finding } => {
                self.open_report(*date);
                self.open_finding(finding);
            }
        }
    }
    fn diary_tasks_filter(&mut self, label: String, keys: BTreeSet<String>) {
        if !self.close_panel() {
            self.check.notice =
                Some("Your task draft is kept. Save or cancel it in Tasks first.".into());
            return;
        }
        self.remember_diary();
        self.set_view(super::tasks::View::Tasks);
        self.tasks.items = self.tasks.all.clone();
        self.tasks.done = core::DoneFilter::Open;
        self.tasks.who = core::Who::Everyone;
        self.tasks.search = rune::TextEdit::default();
        self.tasks.collapsed.clear();
        self.tasks.grouping = core::TaskGrouping::Due;
        self.tasks.diary_rows = Some(keys);
        self.tasks.diary_label = Some(label);
    }
    pub(super) fn diary_return(&mut self) {
        if self.memory.editing {
            self.memory.notice = Some("Save or cancel your Memory draft before returning.".into());
            return;
        }
        if !self.close_panel() {
            return;
        }
        let key = self.check.diary_return.take();
        self.go_home();
        self.check_open = true;
        self.check.report_open = false;
        self.check.refresh();
        self.pending_scroll = key.map(|key| super::PendingScroll {
            key,
            center: false,
            left: 3,
        });
    }
}

pub(super) fn return_link(s: &State, p: Palette) -> El {
    if s.check.diary_return.is_none() {
        return el();
    }
    link("\u{2039} Back to Diary", p)
        .key("diary-return")
        .px(16.0)
        .py(8.0)
        .on_press(|s: &mut State| s.diary_return())
}

fn source_row(
    title: &str,
    context: &str,
    route: core::diary::Route,
    key: String,
    p: Palette,
) -> El {
    let icon = if matches!(
        route,
        core::diary::Route::Memory(_) | core::diary::Route::Proposal(_)
    ) {
        &super::icons::MEMORY
    } else {
        &super::icons::TASKS
    };
    pressable(
        row()
            .w_full()
            .gap(10.0)
            .items_start()
            .child(super::icons::draw(icon, 18.0, p.faint).mt(2.0))
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(roles::META_GAP)
                    .child(
                        roles::CONTROL
                            .wrap(display_text(title, "Review this follow-up"), p.ink)
                            .w_full(),
                    )
                    .child(
                        roles::META
                            .wrap(display_text(context, "Open source for details"), p.faint)
                            .w_full(),
                    ),
            )
            .child(roles::META.wrap("\u{203a}", p.faint)),
    )
    .key(key)
    .label(display_text(title, "Review this follow-up"))
    .py(10.0)
    .radius(6.0)
    .hover(p.chip)
    .on_press_tx(move |s: &mut State, tx| s.diary_route(&route, tx))
}
fn rail_heading(name: &str, count: usize, action: El, p: Palette) -> El {
    col()
        .w_full()
        .gap(12.0)
        .mt(24.0)
        .child(super::widgets::hairline(p))
        .child(
            row()
                .w_full()
                .items_center()
                .justify_between()
                .gap(8.0)
                .mt(8.0)
                .child(roles::GROUP.wrap(format!("{name} ({count})"), p.ink))
                .child(action),
        )
}
fn rail(s: &State, followups: &[core::diary::Attention], p: Palette) -> El {
    let mut out = col().w_full().pb(24.0).child(calendar(s, p));
    if s.check.diary_read_error {
        out = out
            .child(
                roles::SMALL
                    .wrap("Could not load follow-ups", p.warn)
                    .role("alert")
                    .mt(16.0),
            )
            .child(
                super::widgets::action("Retry", p)
                    .key("diary-retry")
                    .on_press(|s: &mut State| s.check.refresh()),
            );
    }
    if !followups.is_empty() {
        let keys: BTreeSet<_> = followups
            .iter()
            .filter(|a| matches!(a.route, core::diary::Route::Task { .. }))
            .map(|a| a.route.key())
            .collect();
        let only_other = keys.is_empty();
        out = out.child(rail_heading(
            "Needs you",
            followups.len(),
            link("See all", p)
                .key("diary-needs-all")
                .on_press(move |s: &mut State| {
                    if only_other {
                        s.check.diary_review_open = true;
                    } else {
                        s.diary_tasks_filter("Needs you".into(), keys.clone());
                    }
                }),
            p,
        ));
        for (i, a) in followups.iter().take(5).enumerate() {
            out = out.child(source_row(
                &a.title,
                &format!("{}{}{}", a.reason.text(), roles::DOT, a.context),
                a.route.clone(),
                format!("diary-needs-{i}"),
                p,
            ));
        }
        if followups
            .iter()
            .any(|a| !matches!(a.route, core::diary::Route::Task { .. }))
        {
            out = out.child(
                link("Review all follow-ups", p)
                    .key("diary-other-followups")
                    .on_press(|s: &mut State| s.check.diary_review_open = true),
            );
        }
    }
    if s.config.features.tasks {
        for waiting in [false, true] {
            let mut tasks: Vec<_> = s
                .tasks
                .all
                .iter()
                .filter(|t| {
                    if waiting {
                        core::diary::waiting(t)
                    } else {
                        core::diary::due_week(t, s.check.today)
                    }
                })
                .collect();
            tasks.sort_by_key(|t| {
                (
                    t.due_date.unwrap_or(NaiveDate::MAX),
                    core::diary::task_route(t).key(),
                )
            });
            tasks.dedup_by_key(|t| core::diary::task_route(t).key());
            if tasks.is_empty() {
                continue;
            }
            let keys: BTreeSet<_> = tasks
                .iter()
                .map(|t| core::diary::task_route(t).key())
                .collect();
            let title = if waiting {
                "Waiting on others"
            } else {
                "Due this week"
            };
            let label = if waiting {
                title.into()
            } else {
                format!(
                    "Due this week, {} to {}",
                    s.check.today.format("%-d %b"),
                    core::diary::week_end(s.check.today).format("%-d %b")
                )
            };
            out = out.child(rail_heading(
                title,
                tasks.len(),
                link("See all", p)
                    .key(if waiting {
                        "diary-waiting-all"
                    } else {
                        "diary-due-all"
                    })
                    .on_press(move |s: &mut State| {
                        s.diary_tasks_filter(label.clone(), keys.clone())
                    }),
                p,
            ));
            for (i, t) in tasks.into_iter().take(3).enumerate() {
                let context = if waiting {
                    t.waiting_label
                        .clone()
                        .or_else(|| t.waiting_on.as_ref().and_then(|w| w.text.clone()))
                        .unwrap_or_else(|| "Waiting for a task dependency".into())
                } else {
                    format!(
                        "{}{}You",
                        t.due_date.unwrap().format("%a %-d %b"),
                        roles::DOT
                    )
                };
                out = out.child(source_row(
                    super::tasks::shown_title(s, t),
                    &context,
                    core::diary::task_route(t),
                    format!("diary-{}-{i}", if waiting { "waiting" } else { "due" }),
                    p,
                ));
            }
        }
    }
    if s.config.features.memory {
        let proposals: Vec<_> = s
            .memory
            .proposals
            .iter()
            .filter(|p| p.status == ProposalStatus::Pending)
            .collect();
        if !proposals.is_empty() {
            out = out.child(rail_heading(
                "Memory proposals",
                proposals.len(),
                link("Open Memory", p)
                    .key("diary-memory-all")
                    .on_press(|s: &mut State| {
                        if s.memory.editing {
                            s.check.notice = Some("Save or cancel your Memory draft first.".into());
                            return;
                        }
                        s.remember_diary();
                        s.open_memory();
                        s.memory.tab = Tab::Review;
                        s.memory.project = None;
                        s.memory.meeting = None;
                        s.memory.search = rune::TextEdit::default();
                    }),
                p,
            ));
            for (i, proposal) in proposals.into_iter().take(2).enumerate() {
                out = out.child(source_row(
                    &proposal.proposal.text,
                    "Currently pending review",
                    core::diary::Route::Proposal(proposal.proposal.id.clone()),
                    format!("diary-proposal-{i}"),
                    p,
                ));
            }
        }
    }
    out
}

/// The real app shell owns navigation and the top-right New meeting button.
pub fn feed(s: &State, k: &Kit, p: Palette, narrow: bool, _layer: &mut Vec<El>) -> El {
    let all = days(s);
    let followups = attention(s);
    let mut timeline = col().w_full().pb(56.0).key("diary-timeline");
    if s.check.diary_review_open {
        timeline = timeline
            .child(roles::SECTION.wrap(format!("Follow-ups ({})", followups.len()), p.ink))
            .child(
                super::widgets::action("Back to days", p)
                    .key("diary-review-close")
                    .on_press(|s: &mut State| s.check.diary_review_open = false),
            );
        for (i, a) in followups.iter().enumerate() {
            timeline = timeline.child(source_row(
                &a.title,
                &format!("{}{}{}", a.reason.text(), roles::DOT, a.context),
                a.route.clone(),
                format!("diary-review-{i}"),
                p,
            ));
        }
    } else {
        if all.is_empty() {
            timeline = timeline
                .child(roles::SECTION.wrap("No meetings yet", p.ink).mt(24.0))
                .child(roles::SMALL.wrap("Record a meeting to begin your Diary.", p.soft));
        }
        let newest_check = s.check.report.as_ref().map(|r| r.date);
        for (n, &date) in all.iter().take(s.check.shown).enumerate() {
            timeline = timeline.child(
                entry(s, k, &day(s, date), newest_check == Some(date), p).mt(if n == 0 {
                    24.0
                } else {
                    roles::SECTION_TOP + 8.0
                }),
            );
        }
        if all.len() > s.check.shown {
            timeline = timeline.child(
                link("Browse earlier days", p)
                    .key("diary-more")
                    .mt(roles::SECTION_TOP)
                    .on_press(|s: &mut State| s.check.shown += PAGE),
            );
        }
    }
    let mut page = col()
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .w_full()
        .max_w(1536.0)
        .px(if narrow { 16.0 } else { 40.0 })
        .pt(if narrow { 8.0 } else { 18.0 })
        .child(
            text_title("Diary", if narrow { 26.0 } else { 30.0 }, p)
                .key("check-top")
                .focusable(true),
        );
    if s.config.features.memory {
        page = page.child(super::memory::refresh_status(&s.memory, p));
    }
    if s.app_w.get() >= 1100.0 {
        page.child(
            row()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .items_start()
                .gap(48.0)
                .child(super::widgets::scroll(timeline, "check-scroll"))
                .child(
                    col().w(320.0).h_full().min_h(0.0).pt(24.0).child(
                        super::widgets::scroll(rail(s, &followups, p), "diary-rail-scroll")
                            .focusable(true)
                            .role("region")
                            .label("This week and follow-ups")
                            .on_key(rail_key),
                    ),
                ),
        )
    } else {
        page.child(super::widgets::scroll(
            col()
                .w_full()
                .child(rail(s, &followups, p).mt(16.0))
                .child(timeline),
            "check-scroll",
        ))
    }
}
fn rail_key(_s: &mut State, event: &KeyEvent, tx: &mut Tx) -> bool {
    match event.key {
        Key::PageDown | Key::Down => tx.scroll_page("diary-rail-scroll", 1),
        Key::PageUp | Key::Up => tx.scroll_page("diary-rail-scroll", -1),
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn display_copy_and_summary_boundaries() {
        assert_eq!(
            display_text("Decision DEC-12 at revision: abcdef12", "Review the update"),
            "Review the update"
        );
        assert_eq!(display_text("Paper workshop", ""), "Paper workshop");
        assert_eq!(display_text("Two\u{2014}parts", ""), "Two,parts");
        assert_eq!(
            short_summary("One. Two. Three."),
            ("One. Two.".into(), true)
        );
        assert_eq!(short_summary("Short"), ("Short".into(), false));
        assert_eq!(
            month_start(NaiveDate::from_ymd_opt(2027, 1, 31).unwrap())
                .checked_sub_months(Months::new(1))
                .unwrap()
                .to_string(),
            "2026-12-01"
        );
    }
}
