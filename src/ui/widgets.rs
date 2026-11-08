//! Small shared pieces: text, chips, links, headings, bullets and the drawn icons.
use super::{
    State,
    theme::{BODY, BODY_LH, Palette},
};
use hex_meetings::core::{self, Point};
use rune::{Color, Cursor, El, Link, LinkPaint, ScrollSpec, Span, TextStyle, col, el, row, text};

pub fn label(s: impl Into<String>, size: f32, weight: f32, ink: Color) -> El {
    text(s)
        .font(TextStyle::new(size, weight).lh((size * 1.55).round()))
        .color(ink)
        .wrap()
}

/// One line that ends in an ellipsis when it does not fit.
pub fn line(s: impl Into<String>, size: f32, weight: f32, ink: Color) -> El {
    text(s)
        .font(TextStyle::new(size, weight).lh((size * 1.5).round()))
        .color(ink)
        .truncate()
        .min_w(0.0)
}

pub fn spaced_caps(s: &str, p: Palette) -> El {
    label(s.to_uppercase(), 11.5, 600.0, p.faint)
}

/// A section heading in the reading column, with an optional quiet count after it.
pub fn heading(s: &str, count: Option<usize>, key: &str, p: Palette) -> El {
    let mut r = row()
        .key(key.to_string())
        .items_end()
        .gap(8.0)
        .mt(40.0)
        .mb(6.0)
        .child(label(s, 19.0, 600.0, p.ink));
    if let Some(n) = count {
        r = r.child(label(n.to_string(), 14.0, 500.0, p.faint));
    }
    r
}

/// A quiet outlined chip for facts about a meeting.
pub fn chip(s: impl Into<String>, p: Palette) -> El {
    label(s, 13.0, 400.0, p.soft)
        .px(10.0)
        .py(2.0)
        .radius(999.0)
        .border(1.0, p.line)
}

pub fn pressable(e: El) -> El {
    e.role("button").focusable(true).cursor(Cursor::Pointer)
}

/// A text link with a quiet hover.
pub fn link(s: impl Into<String>, p: Palette) -> El {
    pressable(label(s, 13.5, 600.0, p.accent).px(4.0).radius(5.0)).hover(p.chip)
}

/// A small text button for toolbars and headers.
pub fn quiet_button(s: &str, key: &str, p: Palette) -> El {
    pressable(label(s, 13.0, 500.0, p.soft))
        .key(key.to_string())
        .px(8.0)
        .py(2.0)
        .radius(6.0)
        .hover(p.chip)
}

pub fn hairline(p: Palette) -> El {
    el().w_full().h(1.0).fill(p.line)
}

pub fn scroll(content: El, key: &str) -> El {
    col()
        .key(key.to_string())
        .grow(1.0)
        .min_h(0.0)
        .min_w(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(content)
}

/// The sidebar toggle: a window outline with its left panel marked.
pub fn sidebar_icon(open: bool, p: Palette) -> El {
    let frame = row().size(18.0, 15.0).radius(3.5).border(1.5, p.soft);
    if open {
        frame.child(el().w(6.0).h_full().fill(p.soft))
    } else {
        frame
            .child(el().w(4.0).h_full())
            .child(el().w(1.5).h_full().fill(p.soft))
    }
}

/// A meeting's mark in the home list: a page with three lines on a soft tile.
pub fn page_icon(p: Palette) -> El {
    let rule = |w: f32| el().w(w).h(1.5).radius(1.0).fill(p.faint);
    col()
        .size(32.0, 32.0)
        .radius(9.0)
        .fill(p.side)
        .center()
        .child(
            col()
                .size(13.0, 16.0)
                .radius(2.5)
                .border(1.5, p.faint)
                .px(2.5)
                .pt(4.0)
                .gap(2.0)
                .child(rule(6.0))
                .child(rule(6.0))
                .child(rule(4.0)),
        )
}

/// The bullet column of a point. Colour carries the kind, the shape stays small.
fn bullet(kind: &str, number: Option<usize>, p: Palette) -> El {
    let glyph = |g: &str, ink: Color, weight: f32| {
        text(g.to_string())
            .font(TextStyle::new(14.0, weight).lh(BODY_LH))
            .color(ink)
    };
    let mark = match (number, kind) {
        (Some(n), _) => glyph(&format!("{n}."), p.ok, 600.0),
        (None, "decided") => glyph("\u{2713}", p.ok, 700.0),
        (None, "concern") => glyph("!", p.warn, 800.0),
        (None, "open") => glyph("?", p.open, 700.0),
        (None, "response") => el()
            .size(6.0, 6.0)
            .radius(3.0)
            .border(1.3, p.faint)
            .mt(11.0),
        _ => el().size(5.0, 5.0).radius(2.5).fill(p.faint).mt(11.0),
    };
    col().w(22.0).items_center().child(mark)
}

/// The timestamp that opens the transcript at its line.
pub fn stamp(at: &[String], origin: &str, p: Palette) -> Option<El> {
    let first = at.first()?.clone();
    let origin = origin.to_string();
    let shown = core::short(&first).to_string();
    Some(
        pressable(
            text(shown.clone())
                .font(TextStyle::new(12.5, 500.0).mono())
                .color(p.faint),
        )
        .key(format!("stamp-{origin}"))
        .label(format!("Open the transcript at {shown}"))
        .px(5.0)
        .py(1.0)
        .radius(4.0)
        .hover(p.chip)
        .on_press_tx(move |s: &mut State, tx| s.jump(&first, origin.clone(), tx)),
    )
}

/// A point: bullet, text with the speaker, and a timestamp link. Decisions are bold, open
/// questions are softer, and responses sit one level in, under the point they answer.
pub fn point_row(pt: &Point, key: String, number: Option<usize>, from: bool, p: Palette) -> El {
    // Decided points are the key items of a topic, in bold. The numbered Decisions list is
    // all decisions, so it uses a lighter weight to stay readable.
    let decided = pt.kind == "decided";
    let weight = match (decided, number) {
        (true, None) => 600.0,
        (_, Some(_)) => 500.0,
        _ => 400.0,
    };
    let ink = if pt.kind == "open" { p.soft } else { p.ink };
    let mut content = pt.text.clone();
    let mut spans = Vec::new();
    if let Some(who) = &pt.who {
        content.push_str("  ");
        let start = content.len();
        // Non-breaking spaces keep the speaker on one line.
        content.push_str(&format!("\u{b7}\u{a0}{}", who.replace(' ', "\u{a0}")));
        spans.push(
            Span::new(start, content.len())
                .weight(400.0)
                .size(14.0)
                .color(p.faint),
        );
    }
    let mut stamp_link = None;
    if let Some(at) = pt.at.first() {
        content.push_str("  ");
        let start = content.len();
        content.push_str(core::short(at));
        let range = start..content.len();
        spans.push(
            Span::new(range.start, range.end)
                .mono()
                .weight(400.0)
                .size(12.5)
                .color(p.faint),
        );
        stamp_link = Some(
            Link::new(range, format!("{key}\u{1f}{at}"))
                .label(format!("Open the transcript at {}", core::short(at)))
                .help("Read this in the transcript")
                .hover(LinkPaint::new().color(p.accent).bg(p.chip)),
        );
    }
    let mut body = text(content)
        .font(TextStyle::new(BODY, weight).lh(BODY_LH))
        .color(ink)
        .wrap()
        .spans(spans)
        .grow(1.0)
        .min_w(0.0);
    if let Some(l) = stamp_link {
        body = body.link(l).on_link(|s: &mut State, id, tx| {
            if let Some((origin, stamp)) = id.split_once('\u{1f}') {
                s.jump(stamp, origin.to_string(), tx);
            }
        });
    }
    let mut r = row()
        .key(key)
        .gap(8.0)
        .py(3.0)
        .pr(6.0)
        .radius(8.0)
        .child(bullet(&pt.kind, number, p))
        .child(body);
    if pt.kind == "response" {
        r = r.pl(26.0);
    }
    if from {
        r = r.fill(p.hit);
    }
    r
}

/// Highlighted text from byte ranges into `text`.
pub fn highlighted(
    text_: &str,
    ranges: &[std::ops::Range<usize>],
    size: f32,
    ink: Color,
    mark_bg: Color,
) -> El {
    let spans: Vec<Span> = ranges
        .iter()
        .map(|r| Span::new(r.start, r.end).weight(600.0).bg(mark_bg, 1.5))
        .collect();
    label(text_, size, 400.0, ink).spans(spans)
}

/// A warning strip above a page, with a Dismiss link.
pub fn notice_bar(note: &str, p: Palette) -> El {
    row()
        .px(24.0)
        .py(8.0)
        .gap(12.0)
        .items_center()
        .fill(p.warnbg)
        .child(label(note, 13.5, 500.0, p.warn).grow(1.0))
        .child(
            link("Dismiss", p)
                .key("notice-dismiss")
                .on_press(|s: &mut State| s.notice = None),
        )
}

/// A small rounded label on a tint: a status, a kind of change.
pub fn badge(s: impl Into<String>, ink: Color, bg: Option<Color>) -> El {
    let b = text(s)
        .font(TextStyle::new(12.0, 600.0).lh(18.0))
        .color(ink)
        .px(8.0)
        .py(1.0)
        .radius(999.0);
    match bg {
        Some(f) => b.fill(f),
        None => b,
    }
}

/// A filled dot, as the mark of a severity or a project.
pub fn dot(size: f32, ink: Color) -> El {
    el().size(size, size).radius(size / 2.0).fill(ink)
}

/// Where a list page and its right-hand panel sit: the list's width with the panel open, the
/// panel's width, and whether the panel covers the list because there is no room for both.
pub fn split_geometry(s: &State, narrow: bool, panel_open: bool) -> (f32, f32, bool) {
    let pane = s.app_w.get() - super::nav::width(s);
    let overlay = narrow || pane < 780.0;
    let panel = (pane * 0.38).clamp(340.0, 420.0);
    let list = if panel_open && !overlay {
        pane - panel - 1.0
    } else {
        pane
    };
    (list, panel, overlay)
}

/// The panel's top strip: a caption, the way back on a narrow window, and the close button.
pub fn panel_top(title: &str, back: &str, overlay: bool, close: fn(&mut State), p: Palette) -> El {
    let shut = pressable(label("\u{d7}", 22.0, 400.0, p.soft))
        .key("panel-close")
        .label("Close the panel (Escape)")
        .px(10.0)
        .radius(8.0)
        .hover(p.chip)
        .on_press_tx(move |s: &mut State, tx| {
            close(s);
            tx.blur();
        });
    let mut top = row().w_full().h(48.0).px(12.0).gap(8.0).items_center();
    if overlay {
        top = top.child(
            pressable(label(format!("\u{2039}  {back}"), 13.5, 500.0, p.soft))
                .key("panel-back")
                .label(format!("Back to {back} (Escape)"))
                .px(10.0)
                .py(4.0)
                .radius(8.0)
                .hover(p.chip)
                .on_press_tx(move |s: &mut State, tx| {
                    close(s);
                    tx.blur();
                }),
        );
    }
    top.child(
        spaced_caps(if overlay { "" } else { title }, p)
            .ml(if overlay { 0.0 } else { 8.0 })
            .grow(1.0),
    )
    .child(shut)
}

/// A panel's frame: top strip, scrolling body, hairline, footer.
pub fn panel_frame(key: &str, top: El, body: El, foot: El, p: Palette) -> El {
    col()
        .key(key.to_string())
        .h_full()
        .min_w(0.0)
        .min_h(0.0)
        .fill(p.card)
        .child(top)
        .child(scroll(body, &format!("{key}-scroll")))
        .child(hairline(p))
        .child(foot)
}

/// The list column, a hairline and the panel, side by side; or the panel alone when it covers
/// the list.
pub fn list_and_panel(list: El, panel: Option<El>, panel_w: f32, overlay: bool, p: Palette) -> El {
    let body = row().w_full().grow(1.0).min_h(0.0).min_w(0.0);
    match panel {
        None => body.child(list),
        Some(panel) if overlay => body.child(panel.grow(1.0)),
        Some(panel) => body
            .child(list)
            .child(el().w(1.0).h_full().fill(p.line))
            .child(panel.w(panel_w)),
    }
}
