//! The Diary: one entry per day, newest first. Each entry is built from what the app already
//! keeps, with no model call: the day's meetings with their decisions and the tasks they gave
//! you, the tasks you ticked that day, memory changes, and the daily check. The newest check's
//! open findings sit in its day as "Needs your attention" and open the same panel as before
//! (Apply, Dismiss, Snooze). Days with nothing are left out. The full report of any day stays
//! one press away.
use super::{
    State, check,
    list::text_title,
    meeting::COLUMN,
    memory::Tab,
    roles,
    theme::Palette,
    widgets::{dot, label, link, pressable},
};
use chrono::{DateTime, Datelike, Local, NaiveDate};
use rune::{El, col, el, row};
use rune_kit::{Kit, Menu, MenuItem, Placement};
use sidenote::core::{self, TaskItem, memory::ProposalStatus};
use std::collections::{BTreeSet, HashMap};

/// Days added by "Show earlier days".
pub const PAGE: usize = 10;
/// Items shown under one heading before "and N more".
const MOST: usize = 3;

/// One day with something in it.
struct Day<'a> {
    date: NaiveDate,
    meetings: Vec<&'a core::Meeting>,
    done: Vec<&'a TaskItem>,
    /// Memory changes: the event, the record id and its text when it still has one.
    memory: Vec<(&'static str, String, Option<String>)>,
    proposals: usize,
    check: bool,
}

fn local_day(at: &str) -> Option<NaiveDate> {
    DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|t| t.with_timezone(&Local).date_naive())
}

fn memory_verb(op: &str) -> Option<&'static str> {
    Some(match op {
        "accept_proposal" => "Added",
        "edit" => "Edited",
        "confirm" => "Confirmed",
        "retire" => "Retired",
        "forget" => "Forgot",
        _ => return None,
    })
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
    }
    if f.memory {
        dates.extend(s.memory.events.iter().filter_map(|e| {
            memory_verb(&e.operation)?;
            local_day(&e.at)
        }));
        dates.extend(
            s.memory
                .proposals
                .iter()
                .filter(|p| p.status == ProposalStatus::Pending)
                .filter_map(|p| local_day(&p.created_at)),
        );
    }
    dates.extend(s.check.dates.iter().copied());
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
    let mut done: Vec<&TaskItem> = if f.tasks {
        s.check
            .done_on
            .iter()
            .filter(|(_, d)| **d == date)
            .filter_map(|((id, index), _)| {
                s.tasks
                    .all
                    .iter()
                    .find(|t| t.done && !t.manual && &t.meeting_id == id && t.index == *index)
            })
            .collect()
    } else {
        Vec::new()
    };
    done.sort_by(|a, b| (&a.meeting_id, a.index).cmp(&(&b.meeting_id, b.index)));
    let mut memory = Vec::new();
    let mut proposals = 0;
    if f.memory {
        for e in s.memory.events.iter().rev() {
            let Some(verb) = memory_verb(&e.operation) else {
                continue;
            };
            if local_day(&e.at) != Some(date) {
                continue;
            }
            let text = s
                .memory
                .records
                .iter()
                .find(|r| r.id == e.record_id)
                .and_then(|r| r.text.clone());
            memory.push((verb, e.record_id.clone(), text));
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
        done,
        memory,
        proposals,
        check: s.check.dates.contains(&date),
    }
}

fn mine<'a>(s: &'a State, m: &core::Meeting) -> Vec<&'a TaskItem> {
    s.tasks
        .all
        .iter()
        .filter(|t| !t.manual && t.meeting_id == m.id && t.mine == Some(true))
        .collect()
}

fn decided(m: &core::Meeting) -> Vec<&core::Point> {
    m.points().filter(|pt| pt.kind == "decided").collect()
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// "Ana, Tom and 2 others".
fn names(people: &[String]) -> String {
    match people {
        [] => String::new(),
        [a] => a.clone(),
        [a, b] => format!("{a} and {b}"),
        [a, b, c] => format!("{a}, {b} and {c}"),
        [a, b, rest @ ..] => format!("{a}, {b} and {} others", rest.len()),
    }
}

/// "a, b and c".
fn and_list(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// The day in two plain sentences, from counts the entry shows below it.
fn summary(s: &State, d: &Day) -> String {
    let mut out = Vec::new();
    if !d.meetings.is_empty() {
        let mut people: Vec<String> = Vec::new();
        for m in &d.meetings {
            for who in &m.summary.people {
                if !people.contains(who) {
                    people.push(who.clone());
                }
            }
        }
        let n = count(d.meetings.len(), "meeting", "meetings");
        out.push(if people.is_empty() {
            format!("{n}.")
        } else {
            format!("{n} with {}.", names(&people))
        });
    }
    let decisions: usize = d.meetings.iter().map(|m| decided(m).len()).sum();
    let new_mine: usize = d.meetings.iter().map(|m| mine(s, m).len()).sum();
    let mut facts = Vec::new();
    if decisions > 0 {
        facts.push(count(decisions, "decision", "decisions"));
    }
    if new_mine > 0 {
        facts.push(count(new_mine, "new task for you", "new tasks for you"));
    }
    if !d.done.is_empty() {
        facts.push(format!("{} done", count(d.done.len(), "task", "tasks")));
    }
    if !d.memory.is_empty() {
        facts.push(count(d.memory.len(), "memory change", "memory changes"));
    }
    if !facts.is_empty() {
        let line = and_list(&facts);
        let mut chars = line.chars();
        let first = chars.next().map(|c| c.to_uppercase().to_string());
        out.push(format!("{}{}.", first.unwrap_or_default(), chars.as_str()));
    }
    if out.is_empty() && d.check {
        out.push("No meetings. The daily check ran.".into());
    }
    out.join(" ")
}

// ---- pieces of an entry --------------------------------------------------------------------

/// A small caption over a group inside the entry.
fn caption(s: &str, p: Palette) -> El {
    roles::META.medium().wrap(s, p.faint).mt(6.0)
}

fn bullet_row(mark: El, words: El) -> El {
    row()
        .w_full()
        .gap(8.0)
        .items_start()
        .child(col().w(14.0).pt(1.0).items_center().child(mark))
        .child(words.grow(1.0).min_w(0.0))
}

fn ring(done: bool, p: Palette) -> El {
    if done {
        label("\u{2713}", 12.0, 700.0, p.ok).mt(2.0)
    } else {
        el().size(9.0, 9.0).radius(5.0).border(1.5, p.soft).mt(6.0)
    }
}

fn more(n: usize, what: &str, p: Palette) -> El {
    roles::META
        .wrap(format!("and {n} more {what}"), p.faint)
        .ml(22.0)
}

fn task_line(t: &TaskItem, p: Palette) -> El {
    let ink = if t.done { p.soft } else { p.ink };
    bullet_row(ring(t.done, p), roles::SMALL.wrap(t.task.clone(), ink))
}

/// One meeting: its time, its title as a link, its one-line context, what it decided and the
/// tasks it gave you.
fn meeting_block(s: &State, m: &core::Meeting, date: NaiveDate, narrow: bool, p: Palette) -> El {
    let time = m
        .started
        .map(|t| t.format("%H:%M").to_string())
        .unwrap_or_default();
    let id = m.id.clone();
    let origin = format!("diary-{date}");
    let mut facts = Vec::new();
    if let Some(n) = m.minutes.filter(|n| *n > 0) {
        facts.push(format!("{n} min"));
    }
    if narrow {
        facts.insert(0, time.clone());
    }
    let title = pressable(
        col()
            .w_full()
            .gap(roles::META_GAP)
            .child(roles::TITLE.wrap(m.title.clone(), p.ink))
            .child(roles::meta_line(&facts, p)),
    )
    .key(format!("diary-meeting-{}", m.id))
    .label(format!("Open the meeting: {}", m.title))
    .w_full()
    .px(8.0)
    .py(4.0)
    .radius(8.0)
    .hover(p.chip)
    .on_press_tx(move |s: &mut State, tx| s.open_check_citation(&id, origin.clone(), tx));
    // Everything under the title lines up with its text, inside the hover padding.
    let mut body = col().w_full().gap(4.0).px(8.0);
    if let Some(line) = m.summary.context.first().filter(|l| !l.trim().is_empty()) {
        body = body.child(roles::SMALL.wrap(line.trim(), p.soft));
    }
    let points = decided(m);
    if !points.is_empty() {
        body = body.child(caption("Decided", p));
        for pt in points.iter().take(MOST) {
            body = body.child(bullet_row(
                dot(5.0, p.faint).mt(8.0),
                roles::SMALL.wrap(pt.text.clone(), p.ink),
            ));
        }
        if points.len() > MOST {
            body = body.child(more(points.len() - MOST, "in the meeting", p));
        }
    }
    let tasks = mine(s, m);
    if !tasks.is_empty() {
        body = body.child(caption("For you", p));
        for t in tasks.iter().take(MOST) {
            body = body.child(task_line(t, p));
        }
        if tasks.len() > MOST {
            body = body.child(more(tasks.len() - MOST, "on the Tasks page", p));
        }
    }
    let block = col()
        .w_full()
        .gap(2.0)
        .grow(1.0)
        .min_w(0.0)
        .child(title)
        .child(body);
    if narrow {
        return block;
    }
    row()
        .w_full()
        .gap(8.0)
        .items_start()
        .child(roles::META.wrap(time, p.faint).w(44.0).pt(7.0))
        .child(block)
}

fn done_block(s: &State, d: &Day, p: Palette) -> El {
    let mut out = col()
        .w_full()
        .gap(4.0)
        .child(roles::GROUP.wrap("Done", p.ink));
    for t in d.done.iter().take(MOST + 2) {
        let from = s
            .meetings
            .iter()
            .find(|m| m.id == t.meeting_id)
            .map(|m| m.title.clone())
            .unwrap_or_default();
        out = out.child(bullet_row(
            ring(true, p),
            col()
                .w_full()
                .child(roles::SMALL.wrap(t.task.clone(), p.ink))
                .child(roles::meta(from, p)),
        ));
    }
    if d.done.len() > MOST + 2 {
        out = out.child(more(d.done.len() - MOST - 2, "ticked", p));
    }
    out
}

fn memory_block(d: &Day, p: Palette) -> El {
    let mut out = col()
        .w_full()
        .gap(4.0)
        .child(roles::GROUP.wrap("Memory", p.ink));
    for (verb, id, text) in d.memory.iter().take(MOST) {
        let words = text
            .clone()
            .unwrap_or_else(|| "A claim you asked to forget.".into());
        let mut line = row()
            .w_full()
            .gap(8.0)
            .items_start()
            .child(roles::SMALL.medium().wrap(*verb, p.soft).w(78.0))
            .child(roles::SMALL.wrap(words, p.ink).grow(1.0).min_w(0.0));
        if text.is_some() {
            let id = id.clone();
            line = pressable(line)
                .key(format!("diary-memory-{}-{id}", d.date))
                .label("Open this memory claim")
                .radius(6.0)
                .hover(p.chip)
                .on_press(move |s: &mut State| {
                    s.open_memory();
                    s.open_record(&id);
                });
        }
        out = out.child(line);
    }
    if d.memory.len() > MOST {
        out = out.child(roles::META.wrap(
            format!("and {} more changes", d.memory.len() - MOST),
            p.faint,
        ));
    }
    if d.proposals > 0 {
        out = out.child(
            link(
                format!(
                    "{} to review \u{203a}",
                    count(d.proposals, "proposal", "proposals")
                ),
                p,
            )
            .key(format!("diary-proposals-{}", d.date))
            .label("Review the memory proposals")
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

fn entry(s: &State, k: &Kit, d: &Day, newest_check: bool, narrow: bool, p: Palette) -> El {
    let mut out = col()
        .key(format!("diary-{}", d.date))
        .w_full()
        .gap(14.0)
        .child(day_head(d, s.check.today, p));
    let line = summary(s, d);
    if !line.is_empty() {
        out = out.child(roles::READ.wrap(line, p.ink));
    }
    if !d.meetings.is_empty() {
        let mut list = col().w_full().gap(18.0).mt(4.0);
        for m in &d.meetings {
            list = list.child(meeting_block(s, m, d.date, narrow, p));
        }
        out = out.child(list);
    }
    if !d.done.is_empty() {
        out = out.child(done_block(s, d, p).mt(4.0));
    }
    if !d.memory.is_empty() || d.proposals > 0 {
        out = out.child(memory_block(d, p).mt(4.0));
    }
    if d.check {
        let part = if newest_check {
            check::day_attention(s, k, p)
        } else {
            check::day_line(s, d.date, p)
        };
        out = out.child(part.mt(4.0));
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
    doc = doc.child(
        roles::SMALL
            .wrap(
                "Your days, newest first: meetings, what they decided, your tasks, memory and what the daily check wants you to look at.",
                p.soft,
            )
            .mt(8.0),
    );
    let newest_check = s.check.report.as_ref().map(|r| r.date);
    for &date in all.iter().take(s.check.shown) {
        let d = day(s, date);
        doc = doc.child(
            entry(s, k, &d, newest_check == Some(date), narrow, p).mt(roles::SECTION_TOP + 8.0),
        );
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
