//! One meeting as a reading column, with the Ask bar floating at the bottom.
//!
//! Under the title and facts sits a row of tabs: Summary, Your notes (only when `notes.md`
//! exists) and Transcript. Summary and Your notes swap the column. The transcript is a
//! searchable panel beside the column in a wide window and takes the whole page in a narrow
//! one. The "..." menu at the end of the row copies or exports this meeting.
use super::{
    State, ask,
    list::text_title,
    more, project, roles,
    theme::Palette,
    todo,
    widgets::{
        chip, hairline, heading, notice_bar, panel_top, point_row, pressable, quiet_button, scroll,
        spaced_caps, split_geometry,
    },
};
use rune::{El, Key, Span, TextEdit, Tx, col, el, row, text};
use rune_kit::{Kit, TextInput};
use sidenote::core::{Meeting, Point, short};
use std::ops::Range;

/// The main column's two tabs. The transcript has its own switch, `State::transcript_open`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Summary,
    Notes,
}

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
        m.minutes.map(|n| {
            if n >= 60 && n % 60 == 0 {
                format!("{} h", n / 60)
            } else if n >= 60 {
                format!("{} h {} min", n / 60, n % 60)
            } else {
                format!("{n} min")
            }
        }),
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
        (m.summary.todo.len(), "tasks", p.accent, "sec-todo"),
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
            r = r.child(roles::SMALL.wrap("\u{b7}", p.faint));
        }
        let content = format!("{n} {name}");
        let number_end = n.to_string().len();
        let body = text(content.clone())
            .font(roles::SMALL.style())
            .color(p.soft)
            .spans(vec![
                Span::new(0, number_end)
                    .weight(600.0)
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
        .child(roles::TITLE.wrap(title, p.ink).mt(24.0).mb(4.0));
    for (pi, pt) in pts.iter().enumerate() {
        let key = format!("pt-{ti}-{pi}");
        let from = s.back.as_deref() == Some(key.as_str());
        c = c.child(point_row(pt, key, None, from, p));
    }
    c
}

impl State {
    /// True when the transcript would take the whole page, because there is no room for it
    /// beside the summary.
    pub fn transcript_covers(&self) -> bool {
        split_geometry(self, self.narrow.get(), true).2
    }

    /// Shows or hides the transcript, when this meeting has one.
    pub fn toggle_transcript(&mut self) {
        if self.current().is_some_and(|m| !m.lines.is_empty()) {
            self.transcript_open = !self.transcript_open;
        }
    }

    /// Picks Summary or Your notes. Where the transcript covers the page, this also closes it.
    pub fn set_meeting_tab(&mut self, tab: Tab) {
        self.meeting_tab = tab;
        if self.transcript_covers() {
            self.transcript_open = false;
        }
    }

    /// The "Go to" box: lands on the line for a time such as 14:22 or 1:02:30.
    pub fn jump_to_typed(&mut self, tx: &mut Tx) {
        let typed = self.transcript_jump.text.trim().to_string();
        if typed.is_empty() {
            return;
        }
        let Some(line) = self.current().and_then(|m| {
            if self.transcript_original
                && let Some(routed) = &m.routed
            {
                let target = sidenote::core::seconds(&typed)? as f64;
                return routed.segments.iter().position(|segment| {
                    (segment.end > segment.start && segment.start <= target && target < segment.end)
                        || segment
                            .sources
                            .iter()
                            .any(|source| source.chunk.start <= target && target < source.chunk.end)
                });
            }
            m.target(&typed)
        }) else {
            self.notice = Some("Try a time that is in the meeting, such as 14:22.".into());
            return;
        };
        self.notice = None;
        self.transcript_filter = TextEdit::new("");
        self.highlight = Some(line);
        self.pending_scroll = Some(super::PendingScroll {
            key: format!("line-{line}"),
            center: true,
            left: 3,
        });
        tx.scroll_key_to(format!("line-{line}"), self.panel_offset());
    }
}

/// The tab that shows in the main column: Your notes only while there are notes.
fn main_tab(s: &State, m: &Meeting) -> Tab {
    if s.meeting_tab == Tab::Notes && !m.notes.is_empty() {
        Tab::Notes
    } else {
        Tab::Summary
    }
}

/// One tab: its name, an optional quiet count, and a 2-point line under the one that is open.
/// `lit` is the transcript's wide-window state: shown beside the summary, so it gets a soft
/// fill instead of the line.
fn tab_button(
    key: &str,
    name: &str,
    count: Option<usize>,
    open: bool,
    lit: bool,
    p: Palette,
) -> El {
    let ink = if open || lit { p.ink } else { p.soft };
    let mut head = row()
        .gap(6.0)
        .items_center()
        .px(8.0)
        .py(3.0)
        .radius(6.0)
        .child(
            if open {
                roles::CONTROL.bold()
            } else {
                roles::CONTROL
            }
            .wrap(name, ink),
        );
    if let Some(n) = count {
        head = head.child(roles::count(n, p));
    }
    if lit {
        head = head.fill(p.chip);
    }
    pressable(
        col()
            .gap(5.0)
            .child(head)
            .child(el().w_full().h(2.0).radius(1.0).fill(if open {
                p.ink
            } else {
                p.bg.alpha(0.0)
            })),
    )
    .key(key.to_string())
    .label(name.to_string())
    .hover(p.chip)
    .radius(6.0)
}

/// Summary, Your notes and Transcript, then the "..." menu at the far end.
fn tab_row(s: &State, k: &Kit, m: &Meeting, narrow: bool, p: Palette, layer: &mut Vec<El>) -> El {
    let tab = main_tab(s, m);
    let panel_on = s.transcript_open && !m.lines.is_empty();
    let covers = panel_on && split_geometry(s, narrow, true).2;
    let mut tabs = row().gap(14.0).items_end().min_w(0.0).child(
        tab_button(
            "tab-summary",
            "Summary",
            None,
            tab == Tab::Summary && !covers,
            false,
            p,
        )
        .on_press_tx(|s: &mut State, tx| {
            s.set_meeting_tab(Tab::Summary);
            tx.blur();
        }),
    );
    if !m.notes.is_empty() {
        tabs = tabs.child(
            tab_button(
                "tab-notes",
                "Your notes",
                None,
                tab == Tab::Notes && !covers,
                false,
                p,
            )
            .on_press_tx(|s: &mut State, tx| {
                s.set_meeting_tab(Tab::Notes);
                tx.blur();
            }),
        );
    }
    if !m.lines.is_empty() {
        tabs = tabs.child(
            tab_button(
                "tab-transcript",
                "Transcript",
                None,
                covers,
                panel_on && !covers,
                p,
            )
            .help("Show or hide the transcript (T)")
            .on_press_tx(|s: &mut State, tx| {
                s.toggle_transcript();
                tx.blur();
            }),
        );
    }
    col()
        .w_full()
        .child(
            row()
                .w_full()
                .items_end()
                .gap(12.0)
                .child(tabs)
                .child(el().grow(1.0))
                .child(more::menu(s, k, m, "meeting", p, layer).pb(2.0)),
        )
        .child(hairline(p))
}

// ---- the transcript panel ----------------------------------------------------------------

/// Where `needle` (already lower case) occurs in `text`, as byte ranges that are safe to
/// highlight. Case folding can change a text's length; then nothing is highlighted, though the
/// line still matches.
fn find_all(text_: &str, needle: &str) -> Vec<Range<usize>> {
    if needle.is_empty() {
        return Vec::new();
    }
    let lower = text_.to_lowercase();
    if lower.len() != text_.len() {
        return Vec::new();
    }
    lower
        .match_indices(needle)
        .map(|(i, found)| i..i + found.len())
        .filter(|r| text_.is_char_boundary(r.start) && text_.is_char_boundary(r.end))
        .collect()
}

/// Speakers keep one colour each, in the order they first speak. "You" is always the accent.
fn speaker_ink(m: &Meeting, who: &str, p: Palette) -> rune::Color {
    if who == "You" {
        return p.accent;
    }
    let ring = [p.open, p.ok, p.warn, p.soft];
    let mut seen: Vec<&str> = Vec::new();
    for l in &m.lines {
        if l.who != "You" && !l.who.is_empty() && !seen.contains(&l.who.as_str()) {
            seen.push(&l.who);
        }
    }
    let at = seen.iter().position(|w| *w == who).unwrap_or(0);
    ring[at % ring.len()]
}

fn transcript_lines(s: &State, m: &Meeting, p: Palette) -> (El, usize) {
    let originals = m.routed.as_ref().map(|r| r.original_lines());
    let lines = if s.transcript_original {
        originals.as_deref().unwrap_or(&m.lines)
    } else {
        &m.lines
    };
    let filter = s.transcript_filter.text.trim().to_lowercase();
    // Imported meetings have no times and may name no speakers. Parts a meeting never uses are
    // left out, so its lines start at the left edge.
    let has_time = m.timed();
    let mut out = col().w_full().px(6.0).pb(16.0);
    let mut shown = 0;
    for (i, line) in lines.iter().enumerate() {
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
        let mut meta = row().gap(10.0).items_center();
        let show_time = has_time && (!s.transcript_original || line.seconds.is_some());
        if show_time {
            meta = meta.child(
                text(short(&line.at).to_string())
                    .font(roles::META.style().mono())
                    .color(p.faint),
            );
        }
        if !line.who.is_empty() {
            meta = meta.child(
                text(line.who.clone())
                    .font(roles::GROUP.style())
                    .color(speaker_ink(m, &line.who, p))
                    .truncate()
                    .min_w(0.0),
            );
        }
        let spans: Vec<Span> = find_all(&line.text, &filter)
            .into_iter()
            .map(|r| Span::new(r.start, r.end).weight(600.0).bg(p.hit, 1.5))
            .collect();
        let body = roles::SMALL
            .wrap(&line.text, p.ink)
            .font(roles::SMALL.style().lh(22.0))
            .spans(spans)
            .selectable(true)
            .w_full()
            .min_w(0.0);
        let mut item = col()
            .key(format!("line-{i}"))
            .w_full()
            .gap(2.0)
            .px(10.0)
            .py(7.0)
            .radius(8.0)
            .fill(if hit {
                p.hit
            } else if near {
                p.chip
            } else {
                p.card
            });
        if show_time || !line.who.is_empty() {
            item = item.child(meta);
        }
        if s.transcript_original && m.routed.is_some() && !line.language.is_empty() {
            item = item.child(
                roles::META
                    .wrap(&line.language, p.faint)
                    .w_full()
                    .min_w(0.0),
            );
        }
        out = out.child(item.child(body));
    }
    if shown == 0 {
        out = out.child(
            roles::SMALL
                .wrap("No transcript lines match.", p.soft)
                .px(10.0)
                .py(8.0),
        );
    }
    (out, shown)
}

/// The transcript: a search box, a "go to" time, the lines with each match marked, and Copy.
fn transcript_panel(s: &State, k: &Kit, m: &Meeting, overlay: bool, p: Palette) -> El {
    let focused = s.transcript_filter.focused;
    let search = TextInput::new(&s.transcript_filter)
        .plain()
        .placeholder("Search the transcript")
        .label("Search the transcript")
        .height(36.0)
        .on_edit(|s: &mut State| &mut s.transcript_filter)
        .on_key(|_: &mut State, e: &rune::KeyEvent, tx: &mut Tx| {
            // Esc leaves the box, so T and Escape work on the page again.
            if e.key == Key::Escape {
                tx.blur();
                return true;
            }
            false
        })
        .build(k)
        .key("transcript-filter")
        .border(1.0, if focused { p.accent } else { p.line })
        .radius(10.0)
        .fill(p.bg)
        .grow(1.0)
        .min_w(0.0);
    let mut tools = row().w_full().gap(8.0).px(14.0).pb(8.0).child(search);
    if m.routed.is_some() {
        tools = tools.child(
            quiet_button(
                if s.transcript_original {
                    "English view"
                } else {
                    "Original view"
                },
                "transcript-language-view",
                p,
            )
            .on_press(|s: &mut State| {
                s.transcript_original = !s.transcript_original;
                s.highlight = None;
                s.pending_scroll = None;
            }),
        );
    }
    if m.timed() {
        tools = tools.child(
            TextInput::new(&s.transcript_jump)
                .plain()
                .placeholder("Go to 14:22")
                .label("Go to a time")
                .height(36.0)
                .width(Some(104.0))
                .on_edit(|s: &mut State| &mut s.transcript_jump)
                .on_submit(|s: &mut State, tx| s.jump_to_typed(tx))
                .build(k)
                .key("transcript-jump")
                .border(
                    1.0,
                    if s.transcript_jump.focused {
                        p.accent
                    } else {
                        p.line
                    },
                )
                .radius(10.0)
                .fill(p.bg),
        );
    }
    let (lines, shown) = transcript_lines(s, m, p);
    let total = if s.transcript_original {
        m.routed
            .as_ref()
            .map_or(m.lines.len(), |r| r.segments.len())
    } else {
        m.lines.len()
    };
    let count = if s.transcript_filter.text.trim().is_empty() {
        format!("{total} lines")
    } else {
        format!("{shown} of {total} lines match")
    };
    let id = m.id.clone();
    let foot = row()
        .w_full()
        .items_center()
        .gap(8.0)
        .px(10.0)
        .py(8.0)
        .child(
            quiet_button("Copy transcript", "transcript-copy", p)
                .on_press_tx(move |s: &mut State, tx| s.copy_transcript(&id, tx)),
        )
        .child(el().grow(1.0))
        .child(roles::meta(count, p).px(4.0));
    let mut frame = col()
        .key("transcript-panel")
        .h_full()
        .min_w(0.0)
        .min_h(0.0)
        .fill(p.card);
    // Beside the summary the panel has its own title and close button. Covering the page it is
    // a tab, and the page's tab row above it does that work.
    if overlay {
        frame = frame.child(el().h(8.0));
    } else {
        frame = frame.child(panel_top(
            "Transcript",
            "Summary",
            false,
            |s: &mut State| s.transcript_open = false,
            p,
        ));
    }
    frame = frame
        .child(tools)
        .child(hairline(p))
        .child(scroll(lines, "transcript-scroll"))
        .child(hairline(p))
        .child(foot);
    // Covering the page, the floating Ask bar sits over the panel's bottom edge. Keep the foot
    // above it.
    if overlay {
        frame = frame.child(el().h(s.overlay_h.get()));
    }
    frame
}

// ---- the page ----------------------------------------------------------------------------

fn summary_sections(s: &State, m: &Meeting, p: Palette, mut body: El) -> El {
    if !m.has_summary() {
        if !m.lines.is_empty() {
            body = body.child(
                super::widgets::link("Open the Transcript tab", p)
                    .key("notes-read-transcript")
                    .self_start()
                    .mt(24.0)
                    .on_press(|s: &mut State| s.toggle_transcript()),
            );
        }
        return body;
    }

    let decided: Vec<&Point> = m.points().filter(|pt| pt.kind == "decided").collect();
    body = body.child(counts(m, p));

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
            .child(heading("Tasks", Some(m.summary.todo.len()), "sec-todo", p))
            .children(super::notes_status::tasks_notice(s, m, p))
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
                        q.waits_on.as_ref().map(|w| format!("Waiting on: {w}")),
                    );
                    point_row(&pt, format!("settle-{i}"), None, false, p)
                })
                .collect(),
        ));
    }

    if !m.summary.topics.is_empty() {
        body = body.child(spaced_caps("Discussion", p).key("sec-discussion").mt(40.0));
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
    body
}

fn document(
    s: &State,
    k: &Kit,
    m: &Meeting,
    narrow: bool,
    floating_ask: bool,
    p: Palette,
    layer: &mut Vec<El>,
) -> El {
    let mut body = col()
        .key("summary-top")
        .w_full()
        .max_w(COLUMN)
        .px(if narrow { 20.0 } else { 40.0 })
        .pt(if narrow { 8.0 } else { 18.0 })
        // Room under the last line for the floating Ask bar.
        .pb(if floating_ask {
            s.overlay_h.get() + 32.0
        } else {
            32.0
        });
    body = body
        .child(text_title(&m.title, if narrow { 28.0 } else { 36.0 }, p).mb(16.0))
        .child(chips(s, k, m, p, layer));
    if let Some(form) = project::form(s, k, &m.id, p) {
        body = body.child(form);
    }
    body = body.children(super::notes_status::banner(s, m, p).map(|b| b.mt(20.0)));
    body = body.child(tab_row(s, k, m, narrow, p, layer).mt(20.0));
    match main_tab(s, m) {
        Tab::Summary => {
            let body = summary_sections(s, m, p, body);
            body.children(super::notes_status::provenance(m, p))
        }
        Tab::Notes => body
            .child(
                super::live::your_notes(&m.notes, p)
                    .key("sec-your-notes")
                    .mt(24.0),
            )
            .child(
                roles::meta(
                    "Typed during the meeting. The summary used these notes first.",
                    p,
                )
                .mt(10.0)
                .px(4.0),
            ),
    }
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
    let panel_on = s.transcript_open && !m.lines.is_empty();
    let (_, panel_w, covers) = split_geometry(s, narrow, panel_on);
    // No room beside the summary: the transcript is the page, and the Ask bar stays at the
    // bottom so people can still ask about what they are reading.
    if panel_on && covers {
        return pane
            .child(
                col()
                    .w_full()
                    .px(if narrow { 20.0 } else { 40.0 })
                    .pt(if narrow { 8.0 } else { 18.0 })
                    .child(text_title(&m.title, 24.0, p).mb(12.0))
                    .child(tab_row(s, k, m, narrow, p, layer)),
            )
            .child(transcript_panel(s, k, m, true, p).grow(1.0))
            .child(ask::overlay(s, k, p, narrow, layer));
    }
    let doc = col()
        .w_full()
        .items_center()
        .child(document(s, k, m, narrow, true, p, layer));
    if !panel_on {
        return pane
            .child(scroll(doc, "meeting-scroll"))
            .child(ask::overlay(s, k, p, narrow, layer));
    }
    // Wide: the summary and its Ask bar on the left, a hairline, the transcript on the right.
    let main = col()
        .key("meeting-main")
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .fill(p.bg)
        .child(scroll(doc, "meeting-scroll"))
        .child(ask::overlay(s, k, p, narrow, layer));
    pane.child(
        row()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .min_w(0.0)
            .child(main)
            .child(el().w(1.0).h_full().fill(p.line))
            .child(transcript_panel(s, k, m, false, p).w(panel_w)),
    )
}

#[cfg(test)]
mod tests {
    use super::find_all;

    #[test]
    fn matches_ignore_case_and_return_byte_ranges() {
        assert_eq!(find_all("Launch the launch", "launch"), vec![0..6, 11..17]);
        assert!(find_all("Launch", "").is_empty());
        assert!(find_all("Launch", "zebra").is_empty());
    }

    #[test]
    fn a_text_whose_case_folding_changes_length_gets_no_marks() {
        // U+0130 lowercases to two characters, so byte ranges would not line up.
        assert!(find_all("\u{130}stanbul launch", "launch").is_empty());
    }

    #[test]
    fn matches_never_split_a_character() {
        let text = "caf\u{e9} CAF\u{c9} caf\u{e9}";
        for r in find_all(text, "caf\u{e9}") {
            assert!(text.is_char_boundary(r.start));
            assert!(text.is_char_boundary(r.end));
        }
    }
}
