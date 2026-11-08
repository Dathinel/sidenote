//! The Ask bar, floating at the bottom of a meeting or the home page: one quiet pill with the
//! question box, the model's name and a round send arrow that fades in once there is text. The
//! box shows a few suggestion chips while it has focus, and Esc folds them away. While a question
//! runs the arrow becomes Stop and a muted line counts the seconds. The thread of answers sits
//! above the pill. On the home page it asks across every meeting, and its answers cite meetings.
//!
//! Motion: each move is one ease of about 180 ms with no bounce. Rune shortens it to a fade when
//! the system asks for reduced motion.
use super::{
    AskRun, State,
    meeting::COLUMN,
    theme::{BODY_LH, Palette},
    widgets::{hairline, label, link, pressable, quiet_button, spaced_caps},
};
use regex::{Captures, Regex};
use rune::{El, Key, KeyEvent, Spring, SpringPair, TextStyle, Transition, Tx, col, el, row, text};
use rune_kit::{Kit, Markdown, MarkdownStyle, Menu, MenuItem, Placement, Spinner, TextInput};
use sidenote::core::{self, Meeting, Message, ask::SUGGESTIONS, ask_all};
use std::{f32::consts::FRAC_PI_2, sync::LazyLock};

static STAMP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[((?:\d{1,2}:)?\d{1,2}:\d{2})\]").expect("stamp pattern"));

/// Turns `[mm:ss]` and `[hh:mm:ss]` into Markdown links the thread can press.
fn linkify(answer: &str) -> String {
    STAMP
        .replace_all(answer, |c: &Captures| {
            format!("[{}](stamp:{})", core::short(&c[1]), &c[1])
        })
        .into_owned()
}

/// `[[<meeting id>@mm:ss]]` and `[[<meeting id>]]`, as the home Ask is told to cite.
static CITE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[\[(\d+-\d+)(?:@((?:\d{1,2}:)?\d{1,2}:\d{2}))?\]\]").expect("citation pattern")
});

/// A citation's time as shown. `mm:ss` stays as written; only a leading `00:` hour goes.
pub(super) fn cite_time(stamp: &str) -> &str {
    if stamp.matches(':').count() == 2 {
        core::short(stamp)
    } else {
        stamp
    }
}

/// The longest a meeting's title gets inside a citation link.
const CITE_TITLE: usize = 26;

/// A title short enough for a link, with the characters Markdown would act on escaped.
pub(super) fn short_title(title: &str) -> String {
    let title = title.trim();
    let cut = if title.chars().count() > CITE_TITLE {
        let head: String = title.chars().take(CITE_TITLE).collect();
        let head = head.rsplit_once(' ').map_or(head.as_str(), |(h, _)| h);
        format!(
            "{}\u{2026}",
            head.trim_end_matches([',', ';', ':', '-', ' '])
        )
    } else {
        title.to_string()
    };
    let mut out = String::new();
    for c in cut.chars() {
        if "\\*_`[]<>".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Turns the home Ask's citations into Markdown links the thread can press: the short meeting
/// title, then the time when there is one. An id that is not a known meeting stays plain text.
pub(super) fn link_citations(answer: &str, meetings: &[Meeting]) -> String {
    CITE.replace_all(answer, |c: &Captures| {
        let id = &c[1];
        let Some(m) = meetings.iter().find(|m| m.id == id) else {
            return c[0].to_string();
        };
        let title = short_title(&m.title);
        match c.get(2) {
            Some(at) => format!(
                "[{title} \u{b7} {}](cite:{id}@{})",
                cite_time(at.as_str()),
                at.as_str()
            ),
            None => format!("[{title}](cite:{id})"),
        }
    })
    .into_owned()
}

fn message(s: &State, k: &Kit, i: usize, m: &Message, p: Palette) -> El {
    let key = s.msg_key(i);
    let from = s.back.as_deref() == Some(key.as_str());
    let user = m.role == "user";
    let body = if user {
        label(&m.text, 15.0, 600.0, p.ink)
    } else {
        let origin = key.clone();
        let all = s.selected.is_none();
        Markdown::new(if all {
            link_citations(&m.text, &s.meetings)
        } else {
            linkify(&m.text)
        })
        .style(
            MarkdownStyle::from_kit(k)
                .text(p.ink)
                .muted(p.soft)
                .link(p.accent)
                .body_font(TextStyle::new(15.0, 400.0).lh(BODY_LH - 2.0)),
        )
        .on_link_tx(move |s: &mut State, url, tx| {
            if let Some(stamp) = url.strip_prefix("stamp:") {
                s.jump(stamp, origin.clone(), tx);
            } else if let Some(target) = url.strip_prefix("cite:") {
                s.open_citation(target, origin.clone(), tx);
            }
        })
        .build(k)
    };
    // A question and its answer read as one block; the hairline goes above each question.
    col()
        .key(key)
        .w_full()
        .pt(if user { 14.0 } else { 4.0 })
        .pb(if user { 2.0 } else { 14.0 })
        .px(8.0)
        .radius(8.0)
        .fill(if from { p.hit } else { p.card })
        .child(body)
}

/// One ease for the whole bar: 180 ms in, 140 ms out, no bounce.
pub(super) const EASE_IN: Spring = Spring::new(0.18, 0.0);
const EASE_OUT: Spring = Spring::new(0.14, 0.0);

/// Fades in while sliding up `rise` points, and out the same way.
pub(super) fn rise(rise: f32) -> Transition {
    Transition::offset(0.0, rise)
        .combined(Transition::opacity())
        .springs(SpringPair::new(EASE_IN, EASE_OUT))
}

/// Fades in while growing from 90 %, for the send and Stop buttons.
fn pop() -> Transition {
    Transition::scale(0.9).springs(SpringPair::new(EASE_IN, EASE_OUT))
}

/// "Thinking, 12 s", then "Thinking, 1 min 5 s".
fn elapsed(secs: u64) -> String {
    if secs < 60 {
        format!("Thinking, {secs} s")
    } else {
        format!("Thinking, {} min {} s", secs / 60, secs % 60)
    }
}

/// What a running Ask shows: a quiet line with the seconds so far and the step it is on.
fn progress(run: &AskRun, k: &Kit, p: Palette) -> El {
    let head = if run.cancel.stopped() {
        "Stopping...".to_string()
    } else {
        elapsed(run.started.elapsed().as_secs())
    };
    let step = run
        .steps
        .last()
        .map(|s| format!("{}...", s.trim_end_matches('.')))
        .unwrap_or_default();
    row()
        .key("ask-progress")
        .w_full()
        .gap(8.0)
        .items_center()
        .px(8.0)
        .pt(4.0)
        .pb(14.0)
        .child(Spinner::new().size(14.0).color(p.faint).build(k))
        .child(label(head, 13.5, 500.0, p.soft))
        .child(
            label(step, 13.0, 400.0, p.faint)
                .key("ask-step")
                .role("status")
                .min_w(0.0),
        )
}

pub(super) fn raised(e: El, p: Palette) -> El {
    e.fill(p.card)
        .border(1.0, p.line)
        .shadow(8.0, 28.0, p.shadow)
}

fn thread(s: &State, k: &Kit, id: &str, p: Palette) -> Option<El> {
    let run = s.asks.get(id);
    let error = s.ask_errors.get(id);
    if s.saved.messages.is_empty() && run.is_none() && error.is_none() {
        return None;
    }
    let hidden = s.thread_hidden && run.is_none();
    let mut actions = row().gap(2.0).items_center().child(
        quiet_button(if hidden { "Show" } else { "Hide" }, "ask-toggle", p)
            .on_press(|s: &mut State| s.thread_hidden = !s.thread_hidden),
    );
    if run.is_none() {
        actions = actions.child(
            quiet_button("Clear", "ask-clear", p).on_press(|s: &mut State| s.clear_thread()),
        );
    }
    let asked = s.saved.messages.iter().filter(|m| m.role == "user").count();
    let about = if s.selected.is_some() {
        "this meeting"
    } else {
        "all your meetings"
    };
    let title = if asked > 0 {
        format!(
            "Asked about {about} \u{b7} {asked} question{}",
            if asked == 1 { "" } else { "s" }
        )
    } else {
        format!("Ask about {about}")
    };
    let header = row()
        .w_full()
        .justify_between()
        .items_center()
        .pl(16.0)
        .pr(8.0)
        .py(8.0)
        .child(spaced_caps(&title, p))
        .child(actions);
    let panel = raised(
        col()
            .key("ask-thread")
            .w_full()
            .radius(16.0)
            .transition(rise(10.0))
            .child(header),
        p,
    );
    if hidden {
        return Some(panel);
    }
    let mut messages = col().w_full().px(10.0).py(2.0);
    for (i, m) in s.saved.messages.iter().enumerate() {
        if i > 0 && m.role == "user" {
            messages = messages.child(hairline(p));
        }
        messages = messages.child(message(s, k, i, m, p));
    }
    if let Some(run) = run {
        messages = messages.child(progress(run, k, p));
    }
    let unanswered = run.is_none()
        && error.is_none()
        && s.saved.messages.last().is_some_and(|m| m.role == "user");
    if unanswered {
        messages = messages.child(
            label(
                "No answer was saved for this question. Ask it again to retry.",
                13.5,
                400.0,
                p.faint,
            )
            .px(8.0)
            .pb(14.0),
        );
    }
    if let Some(e) = error {
        messages = messages.child(
            label(format!("Could not answer. {e}"), 13.5, 500.0, p.warn)
                .role("alert")
                .px(8.0)
                .pb(14.0),
        );
    }
    // The thread floats over the meeting, so it takes at most about half the pane.
    let max_h = (s.pane_h.get() * 0.5).clamp(180.0, 460.0);
    let list = col()
        .key("thread-scroll")
        .w_full()
        .max_h(max_h)
        .min_h(0.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(messages);
    Some(panel.child(hairline(p)).child(list))
}

fn recipe(i: usize, text: &'static str, p: Palette) -> El {
    pressable(label(text, 13.0, 500.0, p.soft))
        .key(format!("suggest-{i}"))
        // Pressing a chip must not take the keyboard from the box, or the chips would vanish
        // before the press landed.
        .preserve_focus_on_press(true)
        // Each chip slides up 8 points and fades in 30 ms after the one before it.
        .transition(rise(8.0))
        .enter_delay(i as f32 * 0.03)
        .px(14.0)
        .py(6.0)
        .radius(999.0)
        .fill(p.card)
        .border(1.0, p.line)
        .shadow(2.0, 10.0, p.shadow)
        .hover(p.chip)
        .on_press(move |s: &mut State| s.send(text))
}

/// A few ideas to ask, floating above the pill while its box has focus and is empty.
fn suggestions(s: &State, narrow: bool, p: Palette) -> El {
    let list = if s.selected.is_some() {
        SUGGESTIONS
    } else {
        ask_all::SUGGESTIONS
    };
    let mut r = row()
        .key("ask-suggestions")
        .w_full()
        .flex_wrap()
        .justify_center()
        .items_center()
        .gaps(8.0, 8.0);
    // Three chips always, a fourth when it still fits on the line: a chip is about 7 points a
    // letter plus its padding and gap.
    let room = if narrow { 320.0 } else { 640.0 };
    let mut used = 0.0;
    for (i, text) in list.iter().enumerate().take(4) {
        let width = text.chars().count() as f32 * 7.0 + 36.0;
        if i == 3 && used + width > room {
            break;
        }
        used += width;
        r = r.child(recipe(i, text, p));
    }
    r
}

/// "Claude Sonnet (answer only)" reads as "Sonnet": the picker stays small. What the
/// suffix said moves to a tooltip.
fn short_runner(name: &str) -> &str {
    let name = name.strip_prefix("Claude ").unwrap_or(name);
    name.strip_suffix(" (answer only)").unwrap_or(name)
}

/// The menu's name for a model. A text-only one says so, so two of the same name differ.
fn menu_name(name: &str) -> String {
    let name = name.strip_prefix("Claude ").unwrap_or(name);
    match name.strip_suffix(" (answer only)") {
        Some(base) => format!("{base} \u{b7} text only"),
        None => name.to_string(),
    }
}

fn runner_note(tools: bool) -> &'static str {
    if tools {
        "Can look things up and use tools"
    } else {
        "Answers from the text only"
    }
}

/// The model's name, muted, inside the pill. Pressing it opens a small menu of agents. An agent
/// whose program is not on PATH says "Not installed" and cannot be picked.
fn runner_choice(s: &State, k: &Kit, p: Palette, narrow: bool, layer: &mut Vec<El>) -> El {
    let name = s.runner.as_deref().unwrap_or("Choose a model");
    let tools = s
        .runners
        .iter()
        .find(|r| Some(&r.name) == s.runner.as_ref())
        .is_some_and(|r| r.tools);
    let chevron = if s.runner_open {
        k.glyph.chevron_up
    } else {
        k.glyph.chevron_down
    };
    let shown = if s.runner.is_some() {
        short_runner(name)
    } else {
        name
    };
    let trigger = pressable(
        row()
            .gap(2.0)
            .items_center()
            .child(
                text(shown.to_string())
                    .font(TextStyle::new(12.5, 500.0).lh(18.0))
                    .color(p.faint)
                    .truncate()
                    .min_w(0.0),
            )
            .child(rune::icon(chevron, 12.0).color(p.faint)),
    )
    .key("ask-model")
    .role("combobox")
    .label(format!("Model: {name}"))
    .help(if s.runner.is_some() {
        format!("Model: {shown}. {}.", runner_note(tools))
    } else {
        "Choose the model that answers".to_string()
    })
    .preserve_focus_on_press(true)
    .min_w(0.0)
    .max_w(if narrow { 110.0 } else { 150.0 })
    .pl(10.0)
    .pr(6.0)
    .py(6.0)
    .radius(999.0)
    .hover(p.chip)
    .on_press(|s: &mut State| {
        s.runner_open = !s.runner_open;
        if s.runner_open {
            s.reload_runners();
        }
    });
    let mut menu = Menu::new("runner")
        .trigger(trigger)
        .open(s.runner_open)
        .placement(Placement::TopEnd)
        .min_width(260.0)
        .on_close(|s: &mut State| s.runner_open = false);
    // Agents that can run come first; the rest follow, marked.
    let (found, missing): (Vec<_>, Vec<_>) = s
        .runners
        .iter()
        .partition(|r| s.runner_found.contains(&r.name));
    for r in found {
        let chosen = r.name.clone();
        menu = menu.item(
            MenuItem::new(menu_name(&r.name))
                .description(runner_note(r.tools))
                .checked(s.runner.as_deref() == Some(r.name.as_str()))
                .on_select(move |s: &mut State| {
                    s.runner = Some(chosen.clone());
                    s.runner_open = false;
                }),
        );
    }
    for r in missing {
        menu = menu.item(
            MenuItem::new(menu_name(&r.name))
                .description("Not installed")
                .disabled(true),
        );
    }
    menu.build(k, layer).min_w(0.0).shrink()
}

/// The room a send or Stop button takes, and how wide the pill is at rest.
const ACTION: f32 = 36.0;
const IDLE_W: f32 = 580.0;

/// The round button at the pill's right end: a send arrow once there is text, a Stop square
/// while a question runs, nothing otherwise. Its slot keeps its size so the model name never
/// shifts when the button comes and goes.
fn action(s: &State, p: Palette, ready: bool, pending: bool) -> El {
    let slot = col().key("ask-action").size(ACTION, ACTION).shrink();
    let disc = || {
        col()
            .size(ACTION, ACTION)
            .radius(999.0)
            .fill(p.accent)
            .center()
    };
    if pending {
        let run = s.asks.get(&s.ask_key());
        let stopping = run.is_some_and(|r| r.cancel.stopped());
        let who = run.map_or("the answer", |r| short_runner(&r.runner));
        return slot.child(
            pressable(disc().child(el().size(12.0, 12.0).radius(3.0).fill(p.card)))
                .key("ask-stop")
                .label("Stop")
                .help(format!("Stop {who}"))
                .transition(pop())
                .dim(if stopping { 0.5 } else { 1.0 })
                .on_press(|s: &mut State| s.stop_ask()),
        );
    }
    if ready {
        return slot.child(
            pressable(
                disc().child(
                    rune::icon(&rune_kit::glyphs::ARROW_RIGHT, 16.0)
                        .color(p.card)
                        .icon_rotation(-FRAC_PI_2),
                ),
            )
            .key("ask-send")
            .label("Ask (Enter)")
            .transition(pop())
            .on_press(|s: &mut State| {
                let q = s.question.text.clone();
                s.send(&q);
            }),
        );
    }
    slot
}

/// What the pill says when no agent CLI is on this machine.
fn no_agent(s: &State, p: Palette) -> El {
    let mut note = col().grow(1.0).min_w(0.0).gap(2.0).child(label(
        "Ask needs an agent: install Claude Code, Codex or OpenCode",
        14.0,
        500.0,
        p.ink,
    ));
    if s.agents_rechecked {
        note = note.child(label(
            "Still not found. If you just installed one, restart Sidenote.",
            12.5,
            400.0,
            p.faint,
        ));
    }
    row()
        .key("ask-pill")
        .w_full()
        .max_w(IDLE_W)
        .gap(10.0)
        .items_center()
        .px(20.0)
        .py(10.0)
        .radius(26.0)
        .child(note)
        .child(
            link("Check again", p)
                .key("ask-recheck")
                .on_press(|s: &mut State| {
                    s.reload_runners();
                    s.agents_rechecked = !s.has_agent();
                }),
        )
}

/// Pinned to the bottom of the meeting pane or the home page, over what is behind it.
pub fn overlay(s: &State, k: &Kit, p: Palette, narrow: bool, layer: &mut Vec<El>) -> El {
    overlay_with(s, k, p, narrow, layer, None, None)
}

/// The bar with room for more: `lead` sits beside the pill (above it in a narrow window), and
/// `above` floats over the thread. The live page puts its recording pill and transcript here.
pub fn overlay_with(
    s: &State,
    k: &Kit,
    p: Palette,
    narrow: bool,
    layer: &mut Vec<El>,
    lead: Option<El>,
    above: Option<El>,
) -> El {
    let id = s.ask_key();
    let pending = s.asks.contains_key(&id);
    let mut stack = col()
        .w_full()
        .max_w(COLUMN)
        .px(if narrow { 12.0 } else { 28.0 })
        .items_center()
        .gap(10.0);
    if s.back.is_some() {
        stack = stack.child(
            row().w_full().justify_end().child(
                pressable(label(
                    if s.check_back {
                        "\u{2039} Back to the check"
                    } else if s.cite_back.is_some() {
                        "\u{2039} Back to your answer"
                    } else {
                        "\u{2191} Back to the summary point"
                    },
                    13.5,
                    600.0,
                    p.card,
                ))
                .key("back-to-summary")
                .px(16.0)
                .py(7.0)
                .radius(999.0)
                .fill(p.accent)
                .shadow(4.0, 16.0, p.shadow)
                .on_press_tx(|s: &mut State, tx| s.go_back(tx)),
            ),
        );
    }
    if let Some(panel) = above {
        stack = stack.child(panel);
    }
    if let Some(panel) = thread(s, k, &id, p) {
        stack = stack.child(panel);
    }
    if !s.has_agent() && !pending && s.runner_error.is_none() {
        // Without an agent there is nothing to type into: the pill says what to install.
        let pill = raised(no_agent(s, p), p);
        return dock(beside(stack, lead, pill, narrow), narrow, p);
    }
    let empty = s.question.text.trim().is_empty();
    let focused = s.ask_open;
    if empty && !pending && focused {
        stack = stack.child(suggestions(s, narrow, p));
    }
    let input = TextInput::new(&s.question)
        .plain()
        .placeholder("Ask anything")
        .label(if s.selected.is_some() {
            "Ask about this meeting"
        } else {
            "Ask about your meetings"
        })
        .height(40.0)
        .on_edit(|s: &mut State| &mut s.question)
        .on_submit(|s: &mut State, _| {
            let q = s.question.text.clone();
            s.send(&q);
        })
        .on_key(|s: &mut State, e: &KeyEvent, tx: &mut Tx| {
            // Esc folds the bar back to rest.
            if e.key == Key::Escape {
                s.runner_open = false;
                tx.blur();
                return true;
            }
            false
        })
        .build(k)
        .key("ask-input")
        .grow(1.0)
        .min_w(0.0);
    let ready = !empty && !pending;
    // The pill lines up with the thread above it when there is one.
    let wide = focused || pending || !empty || !s.saved.messages.is_empty();
    let model = match &s.runner_error {
        Some(e) => label(format!("Runners are not set up. {e}"), 12.5, 500.0, p.warn)
            .px(8.0)
            .max_w(260.0),
        None => runner_choice(s, k, p, narrow, layer),
    };
    let beside_lead = lead.is_some() && !narrow;
    let mut pill = row()
        .key("ask-pill")
        .gap(4.0)
        .items_center()
        .pl(if narrow { 14.0 } else { 20.0 })
        .pr(6.0)
        .py(6.0)
        .radius(999.0)
        // It widens to the width of the thread and rises 4 points on focus.
        .motion(EASE_IN)
        .offset(0.0, if focused { -4.0 } else { 0.0 })
        .offset_motion(EASE_IN)
        .fill(p.card)
        .border(1.0, if focused { p.faint } else { p.line })
        .shadow(
            if focused { 8.0 } else { 3.0 },
            if focused { 28.0 } else { 16.0 },
            p.shadow,
        )
        .child(input)
        .child(model)
        .child(action(s, p, ready, pending));
    // Beside the recording pill the Ask pill takes the rest of the row; alone it is a column.
    pill = if beside_lead {
        pill.grow(1.0).min_w(0.0)
    } else {
        pill.w_full().max_w(if wide { COLUMN } else { IDLE_W })
    };
    dock(beside(stack, lead, pill, narrow), narrow, p)
}

/// Puts `lead` next to `pill` (above it when narrow) at the bottom of `stack`.
fn beside(stack: El, lead: Option<El>, pill: El, narrow: bool) -> El {
    let Some(lead) = lead else {
        return stack.child(pill);
    };
    if narrow {
        return stack.child(lead).child(pill);
    }
    stack.child(
        row()
            .key("live-bar")
            .w_full()
            .gap(10.0)
            .items_center()
            .child(lead)
            .child(pill.grow(1.0).min_w(0.0)),
    )
}

/// The stack pinned to the bottom edge, centred. A short fade lifts the page out from under it,
/// and from the fade down the ground is solid page colour, so the chips, the thread and the
/// pill never have loose page text showing through them.
fn dock(stack: El, narrow: bool, p: Palette) -> El {
    col()
        .key("ask-overlay")
        .pin(Some(0.0), None, Some(0.0), Some(0.0))
        .items_center()
        .child(el().w_full().h(48.0).linear_gradient(
            std::f32::consts::PI,
            [
                (0.0, p.bg.alpha(0.0)),
                (0.55, p.bg.alpha(0.88)),
                (1.0, p.bg),
            ],
        ))
        .child(
            col()
                .w_full()
                .items_center()
                .pb(if narrow { 10.0 } else { 18.0 })
                .fill(p.bg)
                .child(stack),
        )
}
