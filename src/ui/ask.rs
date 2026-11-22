//! The Ask bar, floating at the bottom of a meeting or the home page: one quiet pill with the
//! question box, the model's name and a send button that fades in once there is text. The
//! box shows a few suggestion chips while it has focus, and Esc folds them away. While a question
//! runs the arrow becomes Stop and a muted line counts the seconds. The thread of answers sits
//! above the pill. On the home page it asks across every meeting, and its answers cite meetings.
//!
//! Motion: each move is one ease of about 180 ms with no bounce. Rune shortens it to a fade when
//! the system asks for reduced motion.
use super::{
    AskRun, State,
    meeting::{COLUMN, PAGE_COLUMN},
    roles,
    theme::Palette,
    widgets::{ButtonDisabled, hairline, pressable, quiet_button, spaced_caps},
};
use regex::{Captures, Regex};
use rune::{El, Key, KeyEvent, Spring, SpringPair, Transition, Tx, col, el, row, text};
use rune_kit::{Kit, Markdown, MarkdownStyle, Menu, MenuItem, Placement, Spinner, TextInput};
use sidenote::core::{self, Meeting, Message, ask::SUGGESTIONS, ask_all};
use std::sync::LazyLock;

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
        roles::TITLE.wrap(&m.text, p.ink)
    } else {
        let origin = key.clone();
        let all = s.selected.is_none() && !s.ask_current_live();
        Markdown::new(if all {
            link_citations(&m.text, &s.meetings)
        } else {
            linkify(&m.text)
        })
        .style(
            MarkdownStyle::from_kit(k)
                .text(p.ink)
                .muted(p.soft)
                .link(p.ink)
                .body_font(roles::READ.style()),
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
        .children((!user).then(|| answer_label(s, m)).flatten().map(|label| {
            roles::META
                .wrap(label, p.faint)
                .key(format!("answer-model-{i}"))
                .mt(4.0)
        }))
}

/// "Codex \u{b7} GPT-6.1-Sol" under an answer: the helper and the model it reported. A helper
/// that reported none reads "Codex (model not reported)", never the model that was asked for.
/// Messages saved before this was recorded have no runner and show nothing.
pub(super) fn answer_label(s: &State, m: &Message) -> Option<String> {
    let name = m.runner.as_deref()?;
    let runner = s.runners.iter().find(|r| r.name == name);
    let agent = runner.map_or_else(
        || short_runner(name).to_string(),
        |r| core::ask_options::agent_name(r).to_string(),
    );
    let fallback = if m.fallback { " \u{b7} fallback" } else { "" };
    Some(match (&m.model, runner) {
        (Some(id), Some(r)) => format!("{agent} \u{b7} {}{fallback}", model_label(r, Some(id))),
        (Some(id), None) => format!("{agent} \u{b7} {id}{fallback}"),
        (None, _) => format!("{agent} (model not reported){fallback}"),
    })
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
fn status(run: &AskRun) -> String {
    if run.cancel.stopped() {
        "Stopping...".to_string()
    } else if run.starting_codex {
        "Starting Codex\u{2026}".into()
    } else if !run.turn_started {
        run.steps
            .last()
            .cloned()
            .unwrap_or_else(|| "Reading the meeting...".into())
    } else {
        elapsed(run.started.elapsed().as_secs())
    }
}
fn progress(run: &AskRun, k: &Kit, p: Palette) -> El {
    let head = status(run);
    let step = if run.starting_codex || !run.turn_started {
        String::new()
    } else {
        run.steps
            .last()
            .map(|s| format!("{}...", s.trim_end_matches('.')))
            .unwrap_or_default()
    };
    row()
        .key("ask-progress")
        .w_full()
        .gap(8.0)
        .items_center()
        .px(8.0)
        .pt(4.0)
        .pb(14.0)
        .child(Spinner::new().size(14.0).color(p.faint).build(k))
        .child(roles::CONTROL.wrap(head, p.soft))
        .child(
            roles::META
                .wrap(step, p.faint)
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
    if !hidden {
        super::response_time::visible_operation(
            run.map_or(s.ask_frame_response_time, |r| r.response_time),
        );
    }
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
    let about = if s.selected.is_some() || s.ask_current_live() {
        "this meeting"
    } else {
        "all your meetings"
    };
    let title = if s.ask_current_live() {
        "Asked about this meeting".into()
    } else if asked > 0 {
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
        .gap(8.0)
        // The caption takes what the actions leave and wraps; Hide and Clear keep their own room.
        .child(
            if s.ask_current_live() {
                roles::META.medium().wrap(&title, p.soft)
            } else {
                spaced_caps(&title, p)
            }
            .key("ask-caption")
            .grow(1.0)
            .min_w(0.0),
        )
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
    let panel = if s.on_live() {
        panel
            .max_h(super::live::limits(s).thread)
            .min_h(0.0)
            .clip_rounded(16.0)
    } else {
        panel
    };
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
        if run.text.is_empty() {
            messages = messages.child(progress(run, k, p));
        } else {
            messages = messages.child(
                message(
                    s,
                    k,
                    s.saved.messages.len(),
                    &Message {
                        role: "assistant".into(),
                        text: run.text.clone(),
                        runner: Some(run.runner.clone()),
                        model: run.reported_model.clone(),
                        requested_model: run.requested_model.clone(),
                        fallback: run.fell_back,
                        ..Default::default()
                    },
                    p,
                )
                .key("ask-streaming-answer")
                .role("status"),
            );
        }
    }
    let unanswered = run.is_none()
        && error.is_none()
        && s.saved.messages.last().is_some_and(|m| m.role == "user");
    if unanswered {
        messages = messages.child(
            roles::META
                .wrap(
                    "No answer was saved for this question. Ask it again to retry.",
                    p.faint,
                )
                .px(8.0)
                .pb(14.0),
        );
    }
    if let Some(e) = error {
        messages = messages.child(
            roles::META
                .medium()
                .wrap(format!("Could not answer. {e}"), p.warn)
                .role("alert")
                .px(8.0)
                .pb(14.0),
        );
    }
    // The thread floats over the meeting, so it takes at most about half the pane.
    let max_h = if s.on_live() {
        (super::live::limits(s).thread - 44.0).max(0.0)
    } else {
        (s.pane_h.get() * 0.5).clamp(180.0, 460.0)
    };
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
    super::widgets::button(
        text,
        &format!("suggest-{i}"),
        super::widgets::ButtonRole::Secondary,
        p,
    )
    .preserve_focus_on_press(true)
    .transition(rise(8.0))
    .enter_delay(i as f32 * 0.03)
    .on_press(move |s: &mut State| s.send(text))
}

/// Gutter of the notes column at normal widths. The saved meeting page's Ask bar uses it, so the bar
/// lines up with the notes text; at 390 wide the Ask panel keeps its 16 point gutter.
const NOTES_GUTTER: f32 = 24.0;

/// The suggestion a meeting offers, left-aligned in a row above the Ask bar.
fn meeting_chips(p: Palette) -> El {
    row()
        .key("ask-suggestions")
        .w_full()
        .items_center()
        .gap(8.0)
        .pb(8.0)
        .child(
            recipe(0, "Draft a follow-up", p)
                .min_h(26.0)
                .h(26.0)
                .py(2.0),
        )
}

/// A few ideas to ask, floating above the pill while its box has focus and is empty.
fn suggestions(s: &State, narrow: bool, p: Palette) -> El {
    let list: &[&str] = if s.companies.open {
        &[
            "Where do things stand?",
            "What are they waiting on from us?",
            "Prep my next call",
        ]
    } else if s.selected.is_some() || s.ask_current_live() {
        SUGGESTIONS
    } else {
        ask_all::SUGGESTIONS
    };
    let mut r = row()
        .key("ask-suggestions")
        .w_full()
        .flex_wrap()
        .items_center()
        .gaps(8.0, 8.0);
    if !s.companies.open {
        r = r.justify_center();
    }
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

/// A model's menu name; a helper that lists none shows its own name.
fn model_label(runner: &core::Runner, model: Option<&str>) -> String {
    model
        .map(|id| {
            core::ask_session::models(runner)
                .into_iter()
                .find(|m| m.id == id)
                .map_or_else(|| id.to_string(), |m| m.name)
        })
        .unwrap_or_else(|| short_runner(&runner.name).to_string())
}

/// What the picker's button says: the helper and model the next question uses. While an answer
/// runs and the choice has changed since it started, it says "Next answer: ...", so the
/// answer on screen is never mistaken for the new choice.
pub(super) fn picker_label(s: &State) -> String {
    let current = choice_label(s, s.ask_runner_name(), s.ask_model().as_deref());
    match s.asks.get(&s.ask_key()) {
        Some(run)
            if Some(run.runner.as_str()) != s.ask_runner_name() || run.model != s.ask_model() =>
        {
            format!("Next answer: {current}")
        }
        _ => current,
    }
}

/// "Codex \u{b7} GPT-6.1-Sol", or "Codex \u{b7} provider default" when no model is set.
fn choice_label(s: &State, name: Option<&str>, model: Option<&str>) -> String {
    let Some(name) = name else {
        return "Choose a model".into();
    };
    let Some(runner) = s.runners.iter().find(|r| r.name == name) else {
        return name.to_string();
    };
    let agent = core::ask_options::agent_name(runner);
    match model {
        Some(id) => format!("{agent} \u{b7} {}", model_label(runner, Some(id))),
        None => format!("{agent} \u{b7} provider default"),
    }
}

/// The model picker: one button showing the helper and model, and one popover list. Default
/// first (it follows Settings and shows what it resolves to), then each installed helper's
/// models in a group, then a Thinking switch. It is on the meeting page, the live screen and
/// before the first question of a new chat.
fn runner_choice(
    s: &State,
    k: &Kit,
    p: Palette,
    narrow: bool,
    notes_column: bool,
    layer: &mut Vec<El>,
) -> El {
    let shown = picker_label(s);
    let (caption, width, padding) = match (narrow, notes_column) {
        (true, true) => (shown.replacen("Claude Code · ", "Claude · ", 1), 144.0, 5.0),
        (true, false) => (shown.clone(), 150.0, 12.0),
        _ => (shown.clone(), 240.0, 12.0),
    };
    let chevron = if s.runner_open {
        k.glyph.chevron_up
    } else {
        k.glyph.chevron_down
    };
    let trigger = super::widgets::button("", "ask-model", super::widgets::ButtonRole::Ghost, p)
        .px(padding)
        .gap(0.0)
        .child(
            row()
                .gap(2.0)
                .items_center()
                .min_w(0.0)
                .shrink()
                .child(
                    text(caption)
                        .font(roles::META.medium().style())
                        .color(p.soft)
                        .truncate()
                        .min_w(0.0),
                )
                .child(rune::icon(chevron, 16.0).color(p.faint)),
        )
        .role("combobox")
        .label(format!("Model: {shown}"))
        .help(format!("Model for the next answer: {shown}"))
        .preserve_focus_on_press(true)
        .min_w(0.0)
        .max_w(width)
        .on_press(|s: &mut State| {
            s.runner_open = !s.runner_open;
            s.ask_settings.thinking_open = false;
            if s.runner_open {
                s.reload_runners();
            }
        });
    let overridden = if s.ask_live_meeting() {
        s.ask_settings.meeting_override.is_some()
    } else {
        s.ask_settings.pick.is_some()
    };
    let (default_runner, default_model) = super::ask_settings::default_choice(s);
    let mut menu = Menu::new("runner")
        .trigger(trigger)
        .open(s.runner_open)
        .placement(Placement::TopEnd)
        .min_width(280.0)
        .on_close(|s: &mut State| s.runner_open = false)
        .item(
            MenuItem::new("Default")
                .description(choice_label(
                    s,
                    default_runner.as_deref(),
                    default_model.as_deref(),
                ))
                .checked(!overridden)
                .on_select(|s: &mut State| s.choose_ask_default()),
        );
    for (agent, runner, models) in super::ask_settings::picker_groups(s) {
        menu = menu.group_label(agent);
        for (label, id) in models {
            let chosen =
                overridden && s.ask_runner_name() == Some(runner.as_str()) && s.ask_model() == id;
            let runner = runner.clone();
            menu = menu.item(
                MenuItem::new(label)
                    .checked(chosen)
                    .on_select(move |s: &mut State| s.choose_ask_pick(runner.clone(), id.clone())),
            );
        }
    }
    let thinks = s
        .runners
        .iter()
        .find(|r| Some(r.name.as_str()) == s.ask_runner_name())
        .is_some_and(core::ask_options::supports_thinking);
    if thinks && !s.ask_settings.connection.settings.enabled {
        menu = menu.group_label("Thinking");
        let value = s.ask_thinking();
        for level in core::ask_options::Thinking::ALL {
            menu = menu.item(
                MenuItem::new(level.label())
                    .checked(level == value)
                    .on_select(move |s: &mut State| {
                        s.set_ask_thinking(level);
                        s.runner_open = false;
                    }),
            );
        }
    }
    // Helpers that are not installed stay listed, marked, so a missing one is not a mystery.
    let mut missing: Vec<&str> = s
        .runners
        .iter()
        .filter(|r| !s.runner_found.contains(&r.name))
        .map(core::ask_options::agent_name)
        .filter(|agent| {
            !s.runners.iter().any(|r| {
                s.runner_found.contains(&r.name) && core::ask_options::agent_name(r) == *agent
            })
        })
        .collect();
    missing.sort_unstable();
    missing.dedup();
    for agent in missing {
        menu = menu.item(
            MenuItem::new(agent)
                .description("Not installed")
                .disabled(true),
        );
    }
    menu.build(k, layer).min_w(0.0).shrink()
}

fn scope_choice(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> El {
    let current = if s.ask_settings.live_all {
        "All meetings"
    } else {
        "This meeting"
    };
    let trigger = pressable(
        row()
            .gap(6.0)
            .items_center()
            .child(roles::META.wrap(current, p.soft))
            .child(rune::icon(k.glyph.chevron_down, 12.0).color(p.faint)),
    )
    .key("ask-scope")
    .role("combobox")
    .label(format!("Ask scope: {current}"))
    .preserve_focus_on_press(true)
    .px(10.0)
    .py(5.0)
    .radius(8.0)
    .hover(p.chip)
    .on_press(|s: &mut State| s.ask_settings.scope_open = !s.ask_settings.scope_open);
    Menu::new("ask-scope-menu")
        .trigger(trigger)
        .open(s.ask_settings.scope_open)
        .placement(Placement::TopEnd)
        .min_width(180.0)
        .on_close(|s: &mut State| s.ask_settings.scope_open = false)
        .item(
            MenuItem::new("This meeting")
                .checked(!s.ask_settings.live_all)
                .on_select(|s: &mut State| s.set_live_scope(false)),
        )
        .item(
            MenuItem::new("All meetings")
                .checked(s.ask_settings.live_all)
                .on_select(|s: &mut State| s.set_live_scope(true)),
        )
        .build(k, layer)
}

/// The room a send or Stop button takes, and how wide the pill is at rest.
const ACTION: f32 = 28.0;
const IDLE_W: f32 = 580.0;

/// The round button at the pill's right end: a send arrow once there is text, a Stop square
/// while a question runs, nothing otherwise. Its slot keeps its size so the model name never
/// shifts when the button comes and goes.
fn action(s: &State, p: Palette, ready: bool, pending: bool) -> El {
    let slot = col().key("ask-action").size(ACTION, ACTION).shrink();
    if pending {
        let run = s.asks.get(&s.ask_key());
        let stopping = run.is_some_and(|r| r.cancel.stopped());
        let who = run.map_or("the answer", |r| short_runner(&r.runner));
        return slot.child(
            super::widgets::icon_button(
                &super::icons::STOP,
                "Stop",
                "ask-stop",
                super::widgets::ButtonRole::Primary,
                p,
            )
            .help(format!("Stop {who}"))
            .transition(pop())
            .dim(if stopping { 0.5 } else { 1.0 })
            .on_press(|s: &mut State| s.stop_ask()),
        );
    }
    if ready {
        return slot.child(
            super::widgets::icon_button(
                &super::icons::ARROW_UP,
                "Ask (Enter)",
                "ask-send",
                super::widgets::ButtonRole::Primary,
                p,
            )
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
    let mut note = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(2.0)
        .child(roles::CONTROL.wrap(
            "Ask works with Claude Code, Codex or OpenCode. Meeting summaries need Claude Code.",
            p.ink,
        ));
    if s.agents_rechecked {
        note = note.child(roles::META.wrap(
            "Still not found. If you just installed one, press Check again, or restart Sidenote if it still fails.",
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
            crate::ui::widgets::action("Check again", p)
                .key("ask-recheck")
                .on_press(|s: &mut State| {
                    s.refresh_ai_notes();
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
    let meeting_bar = s.selected.is_some();
    // On the saved meeting page the bar is the notes column's width and shares its left edge.
    let notes_column = meeting_bar && !s.on_live() && !s.companies.open && !s.meeting_ui.tabs;
    // The thread keeps a 16 gutter in a narrow window; the bar steps in to where the notes start,
    // which is the same 16 on a phone and 24 beyond it.
    let bar_inset = if notes_column && narrow {
        (super::meeting::gutter(s) - 16.0).max(0.0)
    } else {
        0.0
    };
    let mut stack = col().w_full().gap(if meeting_bar { 0.0 } else { 10.0 });
    if notes_column {
        // The wide page's notes column is the bar's width exactly, with no gutter of its own.
        let gutter = if super::meeting::strip(s) {
            0.0
        } else if narrow {
            16.0
        } else {
            NOTES_GUTTER
        };
        stack = stack.max_w(COLUMN + 2.0 * gutter).px(gutter);
    } else if !s.companies.open {
        stack = stack
            .max_w(PAGE_COLUMN)
            .px(if narrow { 16.0 } else { 28.0 })
            .items_center();
    }
    if s.back.is_some() && !(s.selected.is_some() && !s.meeting_ui.tabs && s.transcript_open) {
        stack = stack.child(
            row().w_full().justify_end().child(
                super::widgets::button(
                    if s.check_back {
                        "\u{2039} Back to the check"
                    } else if s.cite_back.is_some() {
                        "\u{2039} Back to your answer"
                    } else {
                        "\u{2191} Back to the summary point"
                    },
                    "back-to-summary",
                    super::widgets::ButtonRole::Primary,
                    p,
                )
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
    if let Some(notice) = &s.ask_settings.fallback_notice {
        stack = stack.child(
            roles::META
                .wrap(notice, p.soft)
                .key("ask-fallback-notice")
                .role("status")
                .w_full()
                .px(14.0)
                .py(8.0)
                .radius(10.0)
                .fill(p.card)
                .border(1.0, p.line),
        );
    }
    if s.ask_settings.connection.settings.enabled
        || s.ask_settings.connection.labels.contains_key(&id)
    {
        let label = s
            .ask_settings
            .connection
            .labels
            .get(&id)
            .cloned()
            .unwrap_or_else(|| s.ask_settings.connection.settings.label());
        let selected = s.ask_settings.connection.settings.label();
        let label =
            if !pending && (!s.ask_settings.connection.settings.enabled || label != selected) {
                format!("Last attempt: {label}")
            } else {
                label
            };
        stack = stack.child(
            roles::META
                .wrap(label, p.soft)
                .key("ask-endpoint-label")
                .role("status")
                .w_full(),
        );
        if !pending
            && s.ask_settings.connection.settings.enabled
            && s.ask_settings
                .connection
                .labels
                .get(&id)
                .is_some_and(|label| label != &selected)
        {
            stack = stack.child(
                roles::META
                    .wrap(format!("Next Ask: {selected}"), p.soft)
                    .key("ask-next-endpoint-label")
                    .w_full(),
            );
        }
        if let Some(metrics) = s.ask_settings.connection.metrics.get(&id) {
            stack = stack.child(
                roles::META
                    .wrap(metrics.label(), p.faint)
                    .key("ask-endpoint-timing")
                    .w_full(),
            );
        }
        if let Some(recovery) = super::cloud_ask::recovery(s, p) {
            stack = stack.child(recovery);
        }
    }
    if !s.has_agent() && !pending && s.runner_error.is_none() {
        let pill = if super::ai_notes::setup_visible(s) && !s.ai_notes.ask_setup_open {
            raised(
                super::widgets::action("Ask anything", p)
                    .key("ask-open")
                    .px(20.0)
                    .py(10.0)
                    .on_press(|s: &mut State| s.ai_notes.ask_setup_open = true),
                p,
            )
        } else {
            raised(no_agent(s, p), p)
        };
        return place(s, beside(stack, lead, pill, narrow), narrow, p);
    }
    let empty = s.question.text.trim().is_empty();
    let focused = s.ask_open;
    if empty && !pending && focused && s.selected.is_none() && !s.on_live() {
        stack = stack.child(suggestions(s, narrow, p));
    }
    let input = TextInput::new(&s.question)
        .plain()
        .placeholder(if s.companies.open {
            s.companies
                .selected
                .as_ref()
                .and_then(|id| s.companies.data.catalog.records.get(id))
                .map(|c| format!("Ask about {}", c.name))
                .unwrap_or_else(|| "Ask about this company".into())
        } else {
            "Ask anything".into()
        })
        .label(if s.companies.open {
            "Ask about this company"
        } else if s.selected.is_some() || s.ask_current_live() {
            "Ask about this meeting"
        } else {
            "Ask about your meetings"
        })
        .height(28.0)
        .on_edit(|s: &mut State| &mut s.question)
        .on_submit(|s: &mut State, _| {
            let q = s.question.text.clone();
            s.send(&q);
        })
        .on_key(|s: &mut State, e: &KeyEvent, tx: &mut Tx| {
            // Esc folds the bar back to rest.
            if e.key == Key::Escape {
                s.runner_open = false;
                s.ask_settings.thinking_open = false;
                if s.selected.is_some() && !s.meeting_ui.tabs && s.transcript_open {
                    s.close_transcript_sheet();
                } else {
                    tx.blur();
                }
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
    let wide = s.on_live() || focused || pending || !empty || !s.saved.messages.is_empty();
    let in_meeting = s.selected.is_some() || s.on_live();
    let model = Some({
        if s.ask_settings.connection.settings.enabled {
            quiet_button("Connection", "ask-endpoint-settings", p)
                .button_disabled(pending)
                .help(if pending {
                    "Available after this answer finishes"
                } else {
                    "Open Ask connection settings"
                })
                .on_press(|s: &mut State| {
                    if !s.asks.contains_key(&s.ask_key()) {
                        s.open_connection_settings();
                    }
                })
        } else {
            match &s.runner_error {
                Some(e) => roles::META
                    .medium()
                    .wrap(format!("Runners are not set up. {e}"), p.warn)
                    .px(8.0)
                    .max_w(260.0),
                None => runner_choice(s, k, p, narrow, notes_column, layer),
            }
        }
    });
    // Use the empty send slot for help so it does not squeeze the field or picker at 390 px.
    let help = (notes_column
        && empty
        && !pending
        && s.runner_error.is_none()
        && !s.ask_settings.connection.settings.enabled)
        .then(|| super::widgets::HelpHint::Model.build(k, p, layer));
    // The level control sits inside the bar, at its left; the bar is 40 points tall.
    let lead_in = lead.is_some();
    let mut pill = row()
        .key("ask-pill")
        .gap(4.0)
        .items_center()
        .pl(if lead_in {
            3.0
        } else if narrow {
            14.0
        } else {
            20.0
        })
        .pr(4.0)
        .py(if lead_in { 2.0 } else { 6.0 })
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
        .children(lead)
        .child(input)
        .children(model)
        .child(
            action(s, p, ready, pending)
                .items_center()
                .justify_center()
                .children(help),
        );
    // Beside the recording pill the Ask pill takes the rest of the row; alone it is a column.
    pill = if s.companies.open || notes_column {
        pill.w_full()
    } else {
        pill.w_full().max_w(if wide { PAGE_COLUMN } else { IDLE_W })
    };
    if s.live_scope_available() {
        stack = stack.child(
            row()
                .w_full()
                .justify_end()
                .child(scope_choice(s, k, p, layer)),
        );
    }
    // A meeting offers one suggestion today, in a row above the bar.
    if in_meeting && empty && !pending {
        stack = stack.child(meeting_chips(p).px(bar_inset));
    }
    // The thread keeps the page's 16 point gutter at 390 wide; the bar still starts where the
    // notes do, so it steps in by the difference.
    let stack = stack.child(if bar_inset > 0.0 {
        col().w_full().px(bar_inset).child(pill)
    } else {
        pill
    });
    place(s, stack, narrow, p)
}

fn place(s: &State, stack: El, narrow: bool, p: Palette) -> El {
    if s.on_live() {
        col()
            .key("ask-overlay")
            .w_full()
            .max_h(super::live::limits(s).space)
            .min_h(0.0)
            .shrink()
            .items_center()
            .pb(18.0)
            .fill(p.bg)
            .child(
                stack
                    .max_h((super::live::limits(s).space - 18.0).max(0.0))
                    .min_h(0.0)
                    .shrink()
                    .scroll(rune::ScrollSpec {
                        stick_bottom: false,
                    }),
            )
    } else if s.companies.open {
        col().key("company-ask").w_full().min_w(0.0).child(stack)
    } else if s.selected.is_some() && !s.meeting_ui.tabs {
        // Left-aligned, like the notes above it.
        col()
            .key("ask-overlay")
            .w_full()
            .items_start()
            .shrink()
            .py(12.0)
            .child(stack)
    } else {
        dock(stack, narrow, p)
    }
}

/// Puts `lead` next to `pill` (above it when narrow) at the bottom of `stack`.
fn beside(stack: El, lead: Option<El>, pill: El, _narrow: bool) -> El {
    let Some(lead) = lead else {
        return stack.child(pill);
    };
    stack.child(
        row()
            .key("meeting-bottom-bar")
            .w_full()
            .gap(8.0)
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

#[cfg(test)]
mod meeting_agent_tests {
    use super::*;
    #[test]
    fn catalog_names_replace_ids_and_unknown_ids_remain_unchanged() {
        let runner =
            serde_json::from_str::<Vec<core::Runner>>(include_str!("../../default-runners.json"))
                .unwrap()
                .into_iter()
                .find(|r| r.name == "Codex")
                .unwrap();
        assert_eq!(model_label(&runner, Some("gpt-6.1-terra")), "GPT-6.1 Terra");
        assert_eq!(
            model_label(&runner, Some("unlisted-custom-model")),
            "unlisted-custom-model"
        );
    }
    #[test]
    fn warming_and_started_turn_have_one_distinct_status() {
        let mut run = AskRun::stand_in("Codex", &[], 14);
        run.starting_codex = true;
        run.turn_started = false;
        assert_eq!(status(&run), "Starting Codex\u{2026}");
        run.starting_codex = false;
        run.turn_started = true;
        assert!(status(&run).starts_with("Thinking,"));
    }
}
