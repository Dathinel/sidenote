//! One meeting as a single reading column, with the Ask bar floating at the bottom.
use super::{
    State, ask,
    list::text_title,
    project,
    theme::Palette,
    todo,
    widgets::{chip, heading, label, link, notice_bar, point_row, pressable, scroll, spaced_caps},
};
use hex_meetings::core::{Meeting, Point, short};
use rune::{El, Span, TextStyle, col, row, text};
use rune_kit::{Kit, TextInput};

/// The widest the reading column gets; lines stay near 80 characters.
pub const COLUMN: f32 = 760.0;

fn chips(s: &State, k: &Kit, m: &Meeting, p: Palette, layer: &mut Vec<El>) -> El {
    let mut r = row().flex_wrap().items_center().gaps(6.0, 6.0);
    let when = m
        .started
        .map(|t| t.format("%a %-d %b %Y, %H:%M").to_string());
    let people = match m.summary.people.len() {
        0 => None,
        1..=3 => Some(m.summary.people.join(", ")),
        n => Some(format!(
            "{}, {} and {} more",
            m.summary.people[0],
            m.summary.people[1],
            n - 2
        )),
    };
    for c in [
        when,
        m.minutes.map(|n| format!("{n} min")),
        Some(m.summary.kind.clone()),
        people,
        m.device.as_ref().map(|device| format!("from {device}")),
    ]
    .into_iter()
    .flatten()
    .filter(|c| !c.is_empty())
    {
        r = r.child(chip(c, p));
    }
    if let Some(c) = project::chip(s, k, &m.id, p, layer) {
        r = r.child(c);
    }
    r
}

/// "6 decisions · 9 to do · 3 still open", each part a jump to its section.
fn counts(m: &Meeting, p: Palette) -> El {
    let decided = m.points().filter(|pt| pt.kind == "decided").count();
    let parts = [
        (
            decided,
            if decided == 1 {
                "decision"
            } else {
                "decisions"
            },
            p.ok,
            "sec-decisions",
        ),
        (m.summary.todo.len(), "to do", p.accent, "sec-todo"),
        (
            m.summary.to_settle.len(),
            "still open",
            p.open,
            "sec-settle",
        ),
    ];
    let mut r = row().flex_wrap().items_center().gaps(4.0, 4.0).mt(14.0);
    for (i, (n, name, ink, target)) in parts.into_iter().enumerate() {
        if i > 0 {
            r = r.child(label("\u{b7}", 14.0, 400.0, p.faint));
        }
        let content = format!("{n} {name}");
        let number_end = n.to_string().len();
        let body = text(content.clone())
            .font(TextStyle::new(14.0, 400.0).lh(22.0))
            .color(p.soft)
            .spans(vec![
                Span::new(0, number_end)
                    .weight(650.0)
                    .color(if n == 0 { p.faint } else { ink }),
            ]);
        // A zero has no section to jump to, so it stays plain text.
        let part = if n == 0 {
            body.key(format!("card-{target}")).px(4.0)
        } else {
            pressable(body)
                .key(format!("card-{target}"))
                .label(content)
                .px(4.0)
                .radius(5.0)
                .hover(p.chip)
                .on_press_tx(move |_s: &mut State, tx| tx.scroll_key_to(target, 12.0))
        };
        r = r.child(part);
    }
    r
}

fn plain_point(kind: &str, text: &str, who: Option<String>) -> Point {
    Point {
        kind: kind.into(),
        text: text.into(),
        who,
        at: Vec::new(),
    }
}

fn points(rows: Vec<El>) -> El {
    col().w_full().gap(2.0).children(rows)
}

fn topic(s: &State, ti: usize, title: &str, pts: &[Point], p: Palette) -> El {
    let mut c = col()
        .key(format!("topic-{}", ti + 1))
        .w_full()
        .gap(2.0)
        .child(label(title, 17.0, 600.0, p.ink).mt(26.0).mb(4.0));
    for (pi, pt) in pts.iter().enumerate() {
        let key = format!("pt-{ti}-{pi}");
        let from = s.back.as_deref() == Some(key.as_str());
        c = c.child(point_row(pt, key, None, from, p));
    }
    c
}

fn transcript(s: &State, k: &Kit, m: &Meeting, p: Palette) -> El {
    let mut out = col().w_full().child(heading(
        "Transcript",
        (!m.lines.is_empty()).then_some(m.lines.len()),
        "sec-transcript",
        p,
    ));
    if m.lines.is_empty() {
        return out.child(label("No English transcript yet.", 15.0, 400.0, p.soft));
    }
    let toggle = format!(
        "{}  {} the full English transcript",
        if s.transcript_open {
            "\u{25be}"
        } else {
            "\u{25b8}"
        },
        if s.transcript_open { "Hide" } else { "Show" },
    );
    out = out.child(
        link(toggle, p)
            .key("transcript-toggle")
            .self_start()
            .py(4.0)
            .on_press(|s: &mut State| s.transcript_open = !s.transcript_open),
    );
    if !s.transcript_open {
        return out;
    }
    out = out.child(
        TextInput::new(&s.transcript_filter)
            .plain()
            .placeholder("Filter lines")
            .on_edit(|s: &mut State| &mut s.transcript_filter)
            .build(k)
            .key("transcript-filter")
            .border(
                1.0,
                if s.transcript_filter.focused {
                    p.accent
                } else {
                    p.line
                },
            )
            .radius(10.0)
            .fill(p.card)
            .w_full()
            .my(10.0),
    );
    let filter = s.transcript_filter.text.trim().to_lowercase();
    // Imported meetings have no times and may name no speakers. Columns a meeting never uses
    // are left out, so its lines start at the left edge.
    let has_time = m.timed();
    let has_who = m.lines.iter().any(|l| !l.who.is_empty());
    let mut shown = 0;
    for (i, line) in m.lines.iter().enumerate() {
        if !filter.is_empty()
            && !format!("{} {}", line.who, line.text)
                .to_lowercase()
                .contains(&filter)
        {
            continue;
        }
        shown += 1;
        let hit = s.highlight == Some(i);
        let near = s.highlight.is_some_and(|h| h.abs_diff(i) == 1);
        let who_ink = if line.who == "You" { p.accent } else { p.soft };
        let mut r = row()
            .key(format!("line-{i}"))
            .gap(12.0)
            .px(8.0)
            .py(6.0)
            .radius(6.0)
            .fill(if hit {
                p.hit
            } else if near {
                p.chip
            } else {
                p.bg
            });
        if has_time {
            r = r.child(
                text(short(&line.at).to_string())
                    .font(TextStyle::new(12.5, 400.0).mono())
                    .color(p.faint)
                    .w(52.0)
                    .pt(4.0),
            );
        }
        if has_who {
            r = r.child(
                text(line.who.clone())
                    .font(TextStyle::new(14.0, 600.0))
                    .color(who_ink)
                    .truncate()
                    .w(84.0)
                    .pt(2.0),
            );
        }
        out = out.child(
            r.child(
                label(&line.text, 15.5, 400.0, p.ink)
                    .font(TextStyle::new(15.5, 400.0).lh(25.0))
                    .selectable(true)
                    .grow(1.0)
                    .min_w(0.0),
            ),
        );
    }
    if shown == 0 {
        out = out.child(label("No transcript lines match.", 14.5, 400.0, p.soft));
    }
    out
}

fn document(s: &State, k: &Kit, m: &Meeting, narrow: bool, p: Palette, layer: &mut Vec<El>) -> El {
    let decided: Vec<&Point> = m.points().filter(|pt| pt.kind == "decided").collect();
    let mut body = col()
        .key("summary-top")
        .w_full()
        .max_w(COLUMN)
        .px(if narrow { 20.0 } else { 40.0 })
        .pt(if narrow { 8.0 } else { 18.0 })
        // Room under the last line for the floating Ask bar.
        .pb(s.overlay_h.get() + 32.0);
    body = body
        .child(text_title(&m.title, if narrow { 28.0 } else { 36.0 }, p).mb(16.0))
        .child(chips(s, k, m, p, layer));
    if let Some(form) = project::form(s, k, &m.id, p) {
        body = body.child(form);
    }
    body = body.child(counts(m, p));

    // The user's own notes come first: they are the most important input to the AI notes.
    if !m.notes.is_empty() {
        body = body.child(
            super::live::your_notes(&m.notes, p)
                .key("sec-your-notes")
                .mt(24.0),
        );
    }

    if !m.summary.context.is_empty() {
        body = body.child(heading("Context", None, "sec-context", p));
        body = body.child(points(
            m.summary
                .context
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    point_row(
                        &plain_point("fact", c, None),
                        format!("ctx-{i}"),
                        None,
                        false,
                        p,
                    )
                })
                .collect(),
        ));
    }

    if !decided.is_empty() {
        body = body.child(heading(
            "Decisions",
            Some(decided.len()),
            "sec-decisions",
            p,
        ));
        body = body.child(points(
            decided
                .iter()
                .enumerate()
                .map(|(i, pt)| {
                    let key = format!("dec-{i}");
                    let from = s.back.as_deref() == Some(key.as_str());
                    point_row(pt, key, Some(i + 1), from, p)
                })
                .collect(),
        ));
    }

    if !m.summary.todo.is_empty() {
        body = body
            .child(heading("To do", Some(m.summary.todo.len()), "sec-todo", p))
            .child(todo::section(s, m, p));
    }

    if !m.summary.to_settle.is_empty() {
        body = body.child(heading(
            "Still to settle",
            Some(m.summary.to_settle.len()),
            "sec-settle",
            p,
        ));
        body = body.child(points(
            m.summary
                .to_settle
                .iter()
                .enumerate()
                .map(|(i, q)| {
                    let pt = plain_point(
                        "open",
                        &q.question,
                        q.waits_on.as_ref().map(|w| format!("waits on {w}")),
                    );
                    point_row(&pt, format!("settle-{i}"), None, false, p)
                })
                .collect(),
        ));
    }

    if !m.summary.topics.is_empty() {
        body = body.child(spaced_caps("Discussion", p).key("sec-discussion").mt(48.0));
    }
    for (ti, t) in m.summary.topics.iter().enumerate() {
        body = body.child(topic(s, ti, &t.title, &t.points, p));
    }

    if !m.summary.verify.is_empty() {
        body = body.child(heading("Check before acting", None, "sec-verify", p));
        body = body.child(points(
            m.summary
                .verify
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    point_row(
                        &plain_point("concern", v, None),
                        format!("verify-{i}"),
                        None,
                        false,
                        p,
                    )
                })
                .collect(),
        ));
    }
    body.child(transcript(s, k, m, p))
}

pub fn pane(s: &State, k: &Kit, p: Palette, narrow: bool, layer: &mut Vec<El>) -> El {
    let mut pane = col()
        .key("meeting-pane")
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .w_full()
        .fill(p.bg);
    let Some(m) = s.current() else {
        return pane;
    };
    if let Some(note) = &s.notice {
        pane = pane.child(notice_bar(note, p));
    }
    let doc = col()
        .w_full()
        .items_center()
        .child(document(s, k, m, narrow, p, layer));
    pane.child(scroll(doc, "meeting-scroll"))
        .child(ask::overlay(s, k, p, narrow, layer))
}
