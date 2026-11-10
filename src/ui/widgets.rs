//! Small shared pieces: text, chips, links, headings, bullets and the drawn icons.
use super::{
    State, roles,
    theme::{BODY, BODY_LH, Palette},
};
use rune::{Color, Cursor, El, Link, LinkPaint, ScrollSpec, Span, TextStyle, col, el, row, text};
use sidenote::core::{self, Point};

/// A glyph or icon character (a tick, a chevron, a close cross) at its own size. Words use the
/// roles in `roles.rs` instead.
pub fn label(s: impl Into<String>, size: f32, weight: f32, ink: Color) -> El {
    super::icons::styled(
        s,
        TextStyle::new(size, weight).lh((size * 1.55).round()),
        ink,
    )
}

pub fn spaced_caps(s: &str, p: Palette) -> El {
    roles::CAPTION.wrap(s.to_uppercase(), p.faint)
}

/// A section heading in the reading column, with an optional count pill after it.
pub fn heading(s: &str, count: Option<usize>, key: &str, p: Palette) -> El {
    roles::section(s, count, p).key(key.to_string())
}

/// A quiet outlined chip for facts about a meeting.
pub fn chip(s: impl Into<String>, p: Palette) -> El {
    roles::META
        .wrap(s, p.soft)
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
    pressable(roles::CONTROL.icon_label(s, p.accent).px(4.0).radius(5.0)).hover(p.chip)
}

/// A small text button for toolbars and headers.
pub fn quiet_button(s: &str, key: &str, p: Palette) -> El {
    pressable(roles::CONTROL.wrap(s, p.soft))
        .key(key.to_string())
        .px(8.0)
        .py(2.0)
        .radius(6.0)
        .hover(p.chip)
}

pub fn hairline(p: Palette) -> El {
    el().w_full().h(1.0).fill(p.line)
}

#[derive(Clone, Copy)]
pub enum BannerKind {
    Info,
    Working,
    Problem,
}

/// Shared page-column notices. Actions and optional details are added by the caller.
pub fn banner(title: &str, body: impl Into<String>, kind: BannerKind, p: Palette) -> El {
    let problem = matches!(kind, BannerKind::Problem);
    let ink = if problem { p.warn } else { p.ink };
    let mark = match kind {
        BannerKind::Problem => super::icons::draw(&super::icons::WARNING, 16.0, ink),
        BannerKind::Info => roles::NAME.wrap("i", p.soft),
        BannerKind::Working => rune_kit::Spinner::new()
            .size(16.0)
            .color(p.accent)
            .build(&super::theme::kit(&p)),
    };
    col()
        .w_full()
        .min_w(0.0)
        .padding(16.0)
        .gap(12.0)
        .radius(10.0)
        .fill(if problem { p.warnbg } else { p.card })
        .border(1.0, if problem { p.warn } else { p.line })
        .role(if problem { "alert" } else { "status" })
        .child(
            row()
                .gap(10.0)
                .items_center()
                .child(mark)
                .child(roles::NAME.wrap(title, ink).grow(1.0).min_w(0.0)),
        )
        .child(roles::SMALL.wrap(body, p.ink))
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

pub fn sidebar_icon(open: bool, p: Palette) -> El {
    super::icons::draw(
        if open {
            &super::icons::PANEL_CLOSE
        } else {
            &super::icons::PANEL_OPEN
        },
        16.0,
        p.soft,
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
        (None, "decided") => super::icons::draw(&super::icons::FLAG, 16.0, p.ok),
        (None, "concern") => super::icons::draw(&super::icons::WARNING, 16.0, p.warn),
        (None, "open") => super::icons::draw(&super::icons::HELP, 16.0, p.open),
        (None, "response") => el()
            .size(6.0, 6.0)
            .radius(3.0)
            .border(1.3, p.faint)
            .mt(11.0),
        _ => el().size(5.0, 5.0).radius(2.5).fill(p.faint).mt(11.0),
    };
    col().w(22.0).items_center().child(mark)
}

/// Recording positions keep a minutes field even before the first minute.
pub fn recording_time(at: &str) -> String {
    let shown = if at.matches(':').count() == 2 {
        core::short(at)
    } else {
        at
    };
    if shown.len() == 2 && !shown.contains(':') {
        format!("0:{shown}")
    } else if let Some(seconds) = shown.strip_prefix("00:") {
        format!("0:{seconds}")
    } else {
        shown.to_string()
    }
}

/// The timestamp that opens the transcript at its line.
pub fn stamp(at: &[String], origin: &str, p: Palette) -> Option<El> {
    let first = at.first()?.clone();
    let origin = origin.to_string();
    let shown = format!("\u{b7} {} in recording", recording_time(&first));
    Some(
        pressable(
            text(shown.clone())
                .font(roles::META.medium().style().mono())
                .color(p.faint),
        )
        .key(format!("stamp-{origin}"))
        .label(format!("Open the transcript at {}", recording_time(&first)))
        .help("Time in the recording. Open the transcript.")
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
                .size(roles::META.size)
                .color(p.faint),
        );
    }
    let mut stamp_link = None;
    if let Some(at) = pt.at.first() {
        content.push('\u{a0}');
        let start = content.len();
        content.push_str(&format!("at\u{a0}{}", recording_time(at)));
        let range = start..content.len();
        spans.push(
            Span::new(range.start, range.end)
                .mono()
                .weight(400.0)
                .size(roles::META.size)
                .color(p.faint),
        );
        stamp_link = Some(
            Link::new(range, format!("{key}\u{1f}{at}"))
                .label(format!("Open the transcript at {}", recording_time(at)))
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
    role: roles::Role,
    ink: Color,
    mark_bg: Color,
) -> El {
    let spans: Vec<Span> = ranges
        .iter()
        .map(|r| Span::new(r.start, r.end).weight(600.0).bg(mark_bg, 1.5))
        .collect();
    role.wrap(text_, ink).spans(spans)
}

/// A warning strip above a page, with a Dismiss link.
pub fn notice_bar(note: &str, p: Palette) -> El {
    row()
        .px(24.0)
        .py(8.0)
        .gap(12.0)
        .items_center()
        .fill(p.warnbg)
        .child(roles::CONTROL.wrap(note, p.warn).grow(1.0))
        .child(
            link("Dismiss", p)
                .key("notice-dismiss")
                .on_press(|s: &mut State| s.notice = None),
        )
}

/// A small rounded label on a tint: a status, a kind of change.
pub fn badge(s: impl Into<String>, ink: Color, bg: Option<Color>) -> El {
    let b = text(s)
        .font(roles::META.bold().style())
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
            pressable(roles::CONTROL.icon_label(format!("\u{2039}  {back}"), p.soft))
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

/// A key cap, as in "Ctrl K": a small outlined label for a shortcut.
pub fn keycap(s: impl Into<String>, p: Palette) -> El {
    roles::CAPTION
        .wrap(s, p.faint)
        .px(6.0)
        .py(1.0)
        .radius(5.0)
        .border(1.0, p.line)
}

/// A short confirmation, pinned near the top of the window. It slides in and fades; Rune
/// shortens that to a fade when the system asks for reduced motion.
pub fn toast(text_: &str, p: Palette) -> El {
    col()
        .key("toast")
        .pin(Some(0.0), Some(62.0), Some(0.0), None)
        .items_center()
        .child(
            row()
                .gap(8.0)
                .items_center()
                .px(16.0)
                .py(8.0)
                .radius(999.0)
                .fill(p.card)
                .border(1.0, p.line)
                .shadow(4.0, 16.0, p.shadow)
                .transition(super::ask::rise(8.0))
                .child(label("\u{2713}", 13.0, 700.0, p.ok))
                .child(roles::CONTROL.wrap(text_, p.ink)),
        )
}

#[cfg(test)]
mod tests {
    use super::recording_time;

    #[test]
    fn recording_times_keep_minutes_when_the_source_omits_hours() {
        for (source, shown) in [
            ("00:00:24", "0:24"),
            ("00:24", "0:24"),
            ("00:04:10", "04:10"),
            ("04:10", "04:10"),
            ("01:04:10", "01:04:10"),
        ] {
            assert_eq!(recording_time(source), shown);
        }
    }
}
