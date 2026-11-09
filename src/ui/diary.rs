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
    meeting::COLUMN,
    memory::Tab,
    roles,
    tasks::chevron,
    theme::Palette,
    widgets::{label, link, pressable},
};
use chrono::{DateTime, Datelike, Local, NaiveDate};
use rune::{El, col, el, row};
use rune_kit::{Kit, Menu, MenuItem, Placement};
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
}

/// The folded sections of a day.
#[derive(Clone, Copy)]
pub enum Part {
    Tasks,
    Memories,
    Done,
    Attention,
    Low,
}

impl Part {
    fn name(self) -> &'static str {
        match self {
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
        meetings,
        created: created_tasks,
        done,
        memories,
        proposals,
        check: f.daily_check && s.check.dates.contains(&date),
    }
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
        .unwrap_or_default();
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
            .h(32.0)
            .gap(8.0)
            .items_center()
            .child(roles::META.wrap(time, p.faint).w(44.0))
            .child(roles::CONTROL.line(m.title.clone(), p.ink).shrink())
            .child(el().grow(1.0))
            .child(roles::META.wrap(minutes, p.faint)),
    )
    .key(format!("diary-meeting-{}", m.id))
    .label(format!("Open the meeting: {}", m.title))
    .w_full()
    .px(8.0)
    .radius(6.0)
    .hover(p.chip)
    .on_press_tx(move |s: &mut State, tx| s.open_check_citation(&id, origin.clone(), tx))
}

/// A folded section's header: a turn arrow, the name and its count. Pressing it opens or
/// folds the section.
pub fn fold_head(key: String, name: &str, n: usize, open: bool, p: Palette) -> El {
    let toggle = key.clone();
    pressable(
        row()
            .w_full()
            .h(34.0)
            .gap(8.0)
            .items_center()
            .child(chevron(open, p))
            .child(roles::GROUP.wrap(name, p.ink))
            .child(roles::count(n, p)),
    )
    .key(format!("diary-fold-{key}"))
    .label(format!(
        "{name}, {n}. {}",
        if open { "Fold" } else { "Open" }
    ))
    .w_full()
    .px(8.0)
    .radius(6.0)
    .hover(p.chip)
    .on_press(move |s: &mut State| {
        if !s.check.open_parts.remove(&toggle) {
            s.check.open_parts.insert(toggle.clone());
        }
    })
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
        out = out.child(item(
            tick(t.done, p),
            col()
                .w_full()
                .child(roles::SMALL.wrap(t.task.clone(), if t.done { p.soft } else { p.ink }))
                .child(roles::meta_line(&facts, p)),
        ));
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
                roles::SMALL.wrap(text.clone(), p.ink),
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
                    "{n} {} to review \u{203a}",
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
    let mut head = col().w_full().gap(roles::META_GAP);
    if let Some(w) = when {
        head = head.child(roles::CAPTION.wrap(w.to_uppercase(), p.accent));
    }
    head.child(roles::SECTION.wrap(name, p.ink))
}

fn entry(s: &State, k: &Kit, d: &Day, newest_check: bool, p: Palette) -> El {
    let mut out = col()
        .key(format!("diary-{}", d.date))
        .w_full()
        .gap(12.0)
        .child(day_head(d, s.check.today, p));
    let line = summary(s, d);
    if !line.is_empty() {
        out = out.child(roles::READ.wrap(line, p.ink));
    } else if d.meetings.is_empty() {
        out = out.child(roles::SMALL.wrap("No meetings this day.", p.soft));
    }
    if !d.meetings.is_empty() {
        let mut list = col().w_full();
        for m in &d.meetings {
            list = list.child(meeting_row(m, d.date, p));
        }
        out = out.child(list);
    }
    let open = |part: Part| s.check.open_parts.contains(&part_key(d.date, part));
    let mut parts = col().w_full().gap(2.0);
    let mut any = false;
    if !d.created.is_empty() {
        any = true;
        let on = open(Part::Tasks);
        parts = parts.child(fold_head(
            part_key(d.date, Part::Tasks),
            "Tasks created",
            d.created.len(),
            on,
            p,
        ));
        if on {
            parts = parts.child(task_rows(&d.created, true, p));
        }
    }
    if !d.memories.is_empty() || d.proposals > 0 {
        any = true;
        let on = open(Part::Memories);
        // A day with proposals but nothing accepted says so, rather than "created 0".
        let (name, n) = if d.memories.is_empty() {
            ("Memory proposals", d.proposals)
        } else {
            ("Memories created", d.memories.len())
        };
        parts = parts.child(fold_head(part_key(d.date, Part::Memories), name, n, on, p));
        if on {
            parts = parts.child(memory_rows(d, p));
        }
    }
    if !d.done.is_empty() {
        any = true;
        let on = open(Part::Done);
        parts = parts.child(fold_head(
            part_key(d.date, Part::Done),
            "Done",
            d.done.len(),
            on,
            p,
        ));
        if on {
            parts = parts.child(task_rows(&d.done, false, p));
        }
    }
    if d.check && newest_check {
        any = true;
        parts = parts.child(check::day_attention(s, k, d.date, p));
    }
    if any {
        out = out.child(parts);
    }
    if d.check && !newest_check {
        out = out.child(check::day_line(s, d.date, p).pl(8.0));
    }
    out
}

// ---- the page ------------------------------------------------------------------------------

fn picker(s: &State, k: &Kit, all: &[NaiveDate], p: Palette, layer: &mut Vec<El>) -> El {
    if all.len() < 2 {
        return el();
    }
    let open = s.check.picker_open;
    let trigger = pressable(
        row()
            .gap(4.0)
            .items_center()
            .child(roles::CONTROL.wrap("Earlier days", p.soft))
            .child(
                rune::icon(
                    if open {
                        k.glyph.chevron_up
                    } else {
                        k.glyph.chevron_down
                    },
                    13.0,
                )
                .color(p.faint),
            ),
    )
    .key("check-picker")
    .role("combobox")
    .label("Go to a day")
    .px(12.0)
    .py(5.0)
    .radius(999.0)
    .border(1.0, p.line)
    .hover(p.chip)
    .on_press(|s: &mut State| s.check.picker_open = !s.check.picker_open);
    let mut menu = Menu::new("diary-days")
        .trigger(trigger)
        .open(open)
        .placement(Placement::BottomEnd)
        .min_width(220.0)
        .on_close(|s: &mut State| s.check.picker_open = false);
    for (n, d) in all.iter().take(90).enumerate() {
        let d = *d;
        menu = menu.item(
            MenuItem::new(d.format("%a %-d %b %Y").to_string())
                .on_select(move |s: &mut State| s.diary_jump(d, n)),
        );
    }
    menu.build(k, layer)
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

    /// The day picker: show enough days to reach it, then scroll to its entry.
    fn diary_jump(&mut self, date: NaiveDate, index: usize) {
        self.check.picker_open = false;
        self.check.shown = self.check.shown.max(index + 1);
        self.pending_scroll = Some(super::PendingScroll {
            key: format!("diary-{date}"),
            center: false,
            left: 3,
        });
    }
}

/// The Diary's reading column.
pub fn feed(s: &State, k: &Kit, p: Palette, narrow: bool, layer: &mut Vec<El>) -> El {
    let all = days(s);
    let mut doc = col()
        .key("check-top")
        .w_full()
        .max_w(COLUMN)
        .px(if narrow { 16.0 } else { 40.0 })
        .pt(if narrow { 8.0 } else { 18.0 })
        .pb(56.0);
    doc = doc.child(
        row()
            .w_full()
            .items_end()
            .justify_between()
            .gap(12.0)
            .flex_wrap()
            .child(text_title("Diary", if narrow { 26.0 } else { 30.0 }, p))
            .child(picker(s, k, &all, p, layer).pb(4.0)),
    );
    if all.is_empty() {
        return doc.child(check::empty(s, p));
    }
    let newest_check = s.check.report.as_ref().map(|r| r.date);
    for (n, &date) in all.iter().take(s.check.shown).enumerate() {
        let d = day(s, date);
        let top = if n == 0 {
            24.0
        } else {
            roles::SECTION_TOP + 8.0
        };
        doc = doc.child(entry(s, k, &d, newest_check == Some(date), p).mt(top));
    }
    if all.len() > s.check.shown {
        doc = doc.child(
            link("Show earlier days", p)
                .key("diary-more")
                .label("Show ten earlier days")
                .mt(roles::SECTION_TOP)
                .on_press(|s: &mut State| s.check.shown += PAGE),
        );
    }
    doc
}
