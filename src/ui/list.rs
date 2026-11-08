//! The home page: every summarised meeting in one calm column, grouped by day, with the
//! sidebar's project and device filters narrowing it.
use super::{
    ListItem, State, ask,
    nav::Filter,
    project,
    theme::{Palette, display},
    widgets::{highlighted, keycap, label, line, link, notice_bar, page_icon, pressable, scroll},
};
use hex_meetings::core::{Meeting, SearchHit, SearchSource};
use rune::{El, Key, TextEdit, col, row};
use rune_kit::{Kit, TextInput};

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Who was there, else what kind of meeting it was, then what came out of it.
pub(super) fn people_line(m: &Meeting) -> String {
    let who = match m.summary.people.as_slice() {
        [] => m.summary.kind.clone(),
        [a] => a.clone(),
        [a, b] => format!("{a}, {b}"),
        [a, b, rest @ ..] => format!("{a}, {b} and {} more", rest.len()),
    };
    let decided = m.points().filter(|p| p.kind == "decided").count();
    let mut parts = vec![who];
    if let Some(device) = &m.device {
        parts.push(format!("from {device}"));
    }
    if decided > 0 {
        parts.push(plural(decided, "decision", "decisions"));
    }
    if !m.summary.todo.is_empty() {
        parts.push(format!("{} to do", m.summary.todo.len()));
    }
    parts.retain(|p| !p.is_empty());
    parts.join(" \u{b7} ")
}

fn source_name(source: SearchSource) -> &'static str {
    match source {
        SearchSource::Title => "Title",
        SearchSource::Summary => "Summary",
        SearchSource::Transcript => "Transcript",
    }
}

pub(super) fn snippet(hit: &SearchHit, p: Palette) -> El {
    col()
        .mt(4.0)
        .gap(1.0)
        .child(label(source_name(hit.source), 11.0, 600.0, p.faint))
        .child(highlighted(
            &hit.snippet.text,
            &hit.snippet.matches,
            13.0,
            p.soft,
            p.hit,
        ))
}

fn search_key(s: &mut State, e: &rune::KeyEvent, tx: &mut rune::Tx) -> bool {
    match e.key {
        Key::Up => s.move_cursor(-1, tx),
        Key::Down => s.move_cursor(1, tx),
        Key::Enter => s.open_cursor(tx),
        Key::Escape => {
            s.search = TextEdit::new("");
            s.on_search_changed();
            tx.blur();
        }
        _ => return false,
    }
    true
}

fn search_box(s: &State, k: &Kit, height: f32, placeholder: &str, p: Palette) -> El {
    TextInput::new(&s.search)
        .plain()
        .placeholder(placeholder)
        .on_edit(|s: &mut State| &mut s.search)
        .on_change(|s: &mut State, _| s.on_search_changed())
        .on_key(search_key)
        .height(height)
        // Ctrl+K opens the palette from anywhere; the cap tells people so (UX-12).
        .trailing(keycap("Ctrl K", p))
        .build(k)
        .key("search")
        .border(1.0, if s.search.focused { p.accent } else { p.line })
        .radius(10.0)
        .fill(p.card)
        .w_full()
        .min_w(0.0)
}

fn empty_text(s: &State) -> &'static str {
    if s.meetings.is_empty() {
        "Meetings with an English summary appear here on their own."
    } else if s.filter != Filter::All && s.search.text.trim().is_empty() {
        "No meetings here yet."
    } else {
        "No meeting mentions that."
    }
}

// ---- home --------------------------------------------------------------------------------

fn home_row(
    s: &State,
    k: &Kit,
    item: &ListItem,
    position: usize,
    narrow: bool,
    p: Palette,
    layer: &mut Vec<El>,
) -> El {
    let m = &s.meetings[item.index];
    let at = m
        .started
        .map(|t| t.format("%H:%M").to_string())
        .unwrap_or_default();
    let mut text_col = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(1.0)
        .child(line(&m.title, 15.5, 600.0, p.ink))
        .child(line(people_line(m), 13.5, 400.0, p.soft));
    if let Some(hit) = &item.hit {
        text_col = text_col.child(snippet(hit, p));
    }
    let id = m.id.clone();
    let main = pressable(
        row()
            .gap(14.0)
            .items_center()
            .child(page_icon(p))
            .child(text_col),
    )
    .key(format!("meeting-{}", m.id))
    .label(&m.title)
    .grow(1.0)
    .min_w(0.0)
    .on_press_tx(move |s: &mut State, tx| s.select(&id, tx));
    let mut r = row()
        .w_full()
        .gap(12.0)
        .items_center()
        .px(10.0)
        .py(9.0)
        .radius(10.0)
        .hover(p.chip)
        .child(main);
    if !narrow && let Some(chip) = project::chip(s, k, &m.id, p, layer) {
        r = r.child(chip);
    }
    r = r.child(
        row()
            .w(44.0)
            .justify_end()
            .child(label(at, 13.0, 400.0, p.faint)),
    );
    r = r.child(super::more::menu(s, k, m, "row", p, layer));
    let keyboard = (s.keyboard || !s.search.text.trim().is_empty()) && s.cursor == position;
    if keyboard {
        r = r.fill(p.chip);
    }
    let mut out = col().w_full().child(r);
    if let Some(form) = project::form(s, k, &m.id, p) {
        out = out.child(form.ml(56.0).mb(6.0));
    }
    out
}

/// The home page: every summarised meeting, newest day first, with the Ask bar floating over
/// it. Ask here covers all of them.
pub fn home(s: &State, k: &Kit, p: Palette, narrow: bool, layer: &mut Vec<El>) -> El {
    let side = if narrow { 16.0 } else { 40.0 };
    let (title, count) = match &s.filter {
        Filter::All => (
            "Meetings".to_string(),
            plural(
                s.meetings.len(),
                "meeting summarised",
                "meetings summarised",
            ),
        ),
        f => {
            let n = s.meetings.iter().filter(|m| s.matches_filter(m)).count();
            let name = match f {
                Filter::Project(name) => name.clone(),
                _ => "From other devices".into(),
            };
            (name, plural(n, "meeting", "meetings"))
        }
    };
    let mut doc = col()
        .key("home")
        .w_full()
        .max_w(860.0)
        .px(side)
        .pt(if narrow { 8.0 } else { 20.0 })
        .pb(s.overlay_h.get() + 32.0)
        .child(
            row()
                .w_full()
                .items_end()
                .justify_between()
                .gap(12.0)
                .mb(18.0)
                .child(
                    text_title(&title, if narrow { 28.0 } else { 34.0 }, p)
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(label(count, 13.0, 400.0, p.faint).pb(6.0)),
        )
        .children((s.filter != Filter::All).then(|| {
            link("\u{2039} Show all meetings", p)
                .key("filter-clear")
                .self_start()
                .mb(10.0)
                .on_press(|s: &mut State| s.go_meetings())
        }))
        .children(super::today::strip(s, p))
        .child(search_box(
            s,
            k,
            42.0,
            "Search titles, summaries, to-dos and transcripts",
            p,
        ));
    let mut position = 0;
    for group in &s.groups {
        doc = doc.child(
            label(&group.label, 13.5, 600.0, p.soft)
                .mt(28.0)
                .mb(4.0)
                .px(10.0),
        );
        for item in &group.items {
            doc = doc.child(home_row(s, k, item, position, narrow, p, layer));
            position += 1;
        }
    }
    if position == 0 {
        doc = doc.child(label(empty_text(s), 15.0, 400.0, p.soft).mt(28.0).px(10.0));
    }
    if !s.scan_errors.is_empty() {
        doc = doc.child(
            label(s.scan_errors.join(" "), 12.5, 400.0, p.warn)
                .mt(20.0)
                .px(10.0),
        );
    }
    // "meeting-pane" is the key the Ask thread measures to size itself.
    let mut pane = col()
        .key("meeting-pane")
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .w_full()
        .fill(p.bg);
    if let Some(note) = &s.notice {
        pane = pane.child(notice_bar(note, p));
    }
    pane.child(scroll(
        col().w_full().items_center().child(doc),
        "list-scroll",
    ))
    .child(ask::overlay(s, k, p, narrow, layer))
}

pub fn text_title(s: &str, size: f32, p: Palette) -> El {
    rune::text(s.to_string())
        .font(display(size))
        .color(p.ink)
        .wrap()
}
