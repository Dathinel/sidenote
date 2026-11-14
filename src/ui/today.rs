//! The Today strip at the top of Home. It shows only what needs a look right now, from data the
//! app already holds, with no calendar:
//!
//! - a meeting that is still being made into notes, with its quick title;
//! - to-dos due today and overdue, when Tasks is on;
//! - the newest daily check, when Daily check is on.
//!
//! A line with nothing to say is left out, and the whole strip goes when no line is left.
use super::{
    State, check, live, roles,
    tasks::View,
    theme::Palette,
    widgets::{dot, hairline, pressable, spaced_caps},
};
use rune::{Color, El, col, row};
use sidenote::core::{self, DueBucket};

/// One line of the strip: a coloured dot, the words, and where pressing it goes.
struct Item {
    key: &'static str,
    ink: Color,
    mark: Color,
    lead: Option<String>,
    words: String,
    action: &'static str,
    go: fn(&mut State),
}

/// The to-dos that are late and the ones due today, among those still open and not cleared.
fn due_counts(s: &State) -> (usize, usize) {
    let t = &s.tasks;
    let count = |bucket: DueBucket| {
        t.items
            .iter()
            .filter(|x| {
                !x.done && !core::is_cleared(x, t.cutoff) && core::due_bucket(x, t.today) == bucket
            })
            .count()
    };
    (count(DueBucket::Overdue), count(DueBucket::Today))
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn items(s: &State, p: Palette) -> Vec<Item> {
    let mut out = Vec::new();
    if let Some(l) = s.live.as_ref().filter(|l| l.phase == live::Phase::Making) {
        let heading = live::summary_skip(s).map_or("Writing the summary", |skip| skip.heading());
        let (lead, words) = match l.quick_title() {
            Some(title) => (heading.to_string(), title),
            None => (
                heading.to_string(),
                format!("for the {} meeting", l.clock()),
            ),
        };
        out.push(Item {
            key: "today-making",
            ink: p.ink,
            mark: p.accent,
            lead: Some(lead),
            words,
            action: "Open",
            go: |s| s.open_live(),
        });
    }
    if s.config.features.tasks {
        let (late, today) = due_counts(s);
        if late + today > 0 {
            let mut parts = Vec::new();
            if today > 0 {
                parts.push(format!("{} due today", plural(today, "task", "tasks")));
            }
            if late > 0 {
                parts.push(format!("{late} overdue"));
            }
            out.push(Item {
                key: "today-tasks",
                ink: if late > 0 { p.warn } else { p.ink },
                mark: if late > 0 { p.warn } else { p.accent },
                lead: None,
                words: parts.join(", "),
                action: "Open Tasks",
                go: |s| {
                    s.set_view(View::Tasks);
                    s.nav_done();
                },
            });
        }
    }
    if let Some((words, high)) = check::last_line(s) {
        out.push(Item {
            key: "today-check",
            ink: if high { p.warn } else { p.ink },
            mark: if high { p.warn } else { p.ok },
            lead: None,
            words,
            action: "Open",
            go: |s| {
                s.open_check();
                s.nav_done();
            },
        });
    }
    out
}

/// The strip, or None when there is nothing to show. It is for the whole meeting list only: a
/// project filter or a search has its own job.
pub fn strip(s: &State, p: Palette) -> Option<El> {
    if s.filter != super::nav::Filter::All || !s.search.text.trim().is_empty() {
        return None;
    }
    let items = items(s, p);
    if items.is_empty() {
        return None;
    }
    let mut card = col()
        .key("today")
        .w_full()
        .mb(16.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.line)
        .child(spaced_caps("Today", p).px(14.0).pt(10.0).pb(2.0));
    for (n, item) in items.into_iter().enumerate() {
        if n > 0 {
            card = card.child(row().w_full().px(14.0).child(hairline(p)));
        }
        let mut words = row()
            .grow(1.0)
            .min_w(0.0)
            .gap(8.0)
            .items_center()
            .child(dot(8.0, item.mark));
        if let Some(lead) = &item.lead {
            words = words.child(roles::SMALL.bold().wrap(lead, item.ink));
        }
        words = words.child(
            roles::SMALL
                .wrap(
                    &item.words,
                    if item.lead.is_some() {
                        p.soft
                    } else {
                        item.ink
                    },
                )
                .grow(1.0)
                .min_w(0.0),
        );
        let said = match &item.lead {
            Some(lead) => format!("{lead} {}", item.words),
            None => item.words.clone(),
        };
        card = card.child(
            pressable(
                row()
                    .w_full()
                    .gap(10.0)
                    .items_center()
                    .px(14.0)
                    .py(9.0)
                    .child(words)
                    .child(roles::CONTROL.wrap(item.action, p.accent)),
            )
            .key(item.key)
            .label(format!("{said}. {}", item.action))
            .w_full()
            .radius(10.0)
            .hover(p.chip)
            .on_press(item.go),
        );
    }
    Some(card)
}
