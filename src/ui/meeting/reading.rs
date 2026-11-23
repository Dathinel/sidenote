//! Reading keeps the narrative, actions and transcript in their own scroll areas.
use super::*;
use crate::ui::{
    icons, notes_status, tasks,
    widgets::{self, HelpHint},
};
use rune::{Color, KeyEvent};
use rune_kit::{Placement, Popover};
use sidenote::core::{self, people::Side};

pub fn return_key(origin: &str) -> String {
    if origin.starts_with("pt-") {
        format!("meeting-point-{origin}")
    } else if let Some(index) = origin.strip_prefix("todo-") {
        format!("meeting-task-{index}")
    } else {
        origin.into()
    }
}

impl State {
    pub fn close_transcript_sheet(&mut self) {
        self.transcript_open = false;
        self.transcript_filter.focused = false;
        self.transcript_jump.focused = false;
        if self
            .back
            .as_deref()
            .is_some_and(|origin| origin.starts_with("pt-") || origin.starts_with("todo-"))
        {
            self.back = None;
        }
    }

    pub fn set_meeting_layout(&mut self, tabs: bool) {
        let mut prefs = core::UiPrefs::load(&self.store.root);
        prefs.meeting_tabs = tabs;
        match prefs.save(&self.store.root) {
            Ok(()) => {
                self.meeting_ui.tabs = tabs;
                self.meeting_ui.status_open = false;
                self.more_menu = None;
            }
            Err(_) => self.notice = Some("Could not save the meeting layout. Try again.".into()),
        }
    }
    fn choose_summary_template(&mut self, id: &str, choice: &str) {
        let mut prefs = core::UiPrefs::load(&self.store.root);
        prefs
            .meeting_summary_templates
            .insert(id.into(), choice.into());
        match prefs.save(&self.store.root) {
            Ok(()) => {
                self.meeting_ui.templates = prefs.meeting_summary_templates;
                self.more_menu = None;
            }
            Err(_) => self.notice = Some("Could not save the summary template. Try again.".into()),
        }
    }
}

pub fn menu_items(s: &State, m: &Meeting, menu: El, p: Palette) -> El {
    use crate::ui::widgets::{menu_row, menu_separator};
    let group = |label| {
        row()
            .h(28.0)
            .px(8.0)
            .items_center()
            .child(roles::META.line(label, p.faint))
    };
    let item = |name| menu_row(name, p, None, None, None);
    let choice_item = |name, checked| {
        item(name)
            .role("menuitemcheckbox")
            .checked(checked)
            .focus_selected(checked)
            .child(
                icons::draw(&icons::CHECK, 16.0, p.soft)
                    .opacity(if checked { 1.0 } else { 0.0 })
                    .a11y_hidden(),
            )
    };
    let id = m.id.clone();
    let mut out = menu
        .child(menu_separator(p))
        .child(
            item("Ask model and effort").on_press(|s: &mut State| {
                s.more_menu = None;
                s.open_hex();
                s.hex.section = crate::ui::hex::Section::Connect;
                crate::ui::scene_scroll(s, "ask-agent-row");
            }),
        )
        .child(item("Project").on_press({
            let id = m.id.clone();
            move |s: &mut State| {
                s.more_menu = None;
                s.project_menu = Some(id.clone());
            }
        }))
        .child(item("Memory from this meeting").on_press({
            let id = m.id.clone();
            move |s: &mut State| {
                s.more_menu = None;
                s.open_memory();
                s.memory.settle_all = true;
                s.memory.settle_meeting = Some(id.clone());
            }
        }))
        .child(group("View"))
        .child(
            choice_item("Summary", main_tab(s, m) == Tab::Summary)
                .on_press(|s: &mut State| {
                    s.set_meeting_tab(Tab::Summary);
                    s.more_menu = None;
                }),
        )
        .child(
            choice_item("Your notes", main_tab(s, m) == Tab::Notes)
                .disabled(m.notes.is_empty())
                .focusable(!m.notes.is_empty())
                .layer_opacity(if m.notes.is_empty() { 0.5 } else { 1.0 })
                .on_press(|s: &mut State| {
                    s.set_meeting_tab(Tab::Notes);
                    s.more_menu = None;
                }),
        )
        .child(menu_separator(p))
        .child(group("Layout"))
        .child(
            choice_item("Reading", !s.meeting_ui.tabs)
                .on_press(|s: &mut State| s.set_meeting_layout(false)),
        )
        .child(
            choice_item("Tabs", s.meeting_ui.tabs)
                .on_press(|s: &mut State| s.set_meeting_layout(true)),
        )
        .child(menu_separator(p))
        .child(group("Next summary uses"));
    let picked = picked_template(s, m);
    for (title, choice, enabled) in [
        ("Standard", "standard", true),
        ("Your instructions", "custom", s.meeting_ui.custom),
    ] {
        let id = id.clone();
        out = out.child(
            choice_item(title, choice == picked)
                .disabled(!enabled)
                .focusable(enabled)
                .layer_opacity(if enabled { 1.0 } else { 0.5 })
                .on_press(move |s: &mut State| s.choose_summary_template(&id, choice)),
        );
    }
    out = out.child(
        item("Edit summary prompt").on_press(|s: &mut State| {
            s.more_menu = None;
            s.open_hex();
            s.hex.section = crate::ui::hex::Section::PromptLibrary;
            crate::ui::scene_scroll(s, "agent-instructions");
        }),
    );
    out = out.child(item("Write an email").on_press(|s: &mut State| {
        s.more_menu = None;
        s.question = TextEdit::new(
            "Draft a short email about this meeting. Use only what was discussed and keep it ready to copy.",
        );
        s.question.focused = true;
    }));
    if !m.lines.is_empty() {
        out = out.child(
            item("Copy transcript")
                .on_press_tx(move |s: &mut State, tx| s.copy_transcript(&id, tx)),
        );
    }
    out
}

/// The instructions the next summary of this meeting will use: "standard" or "custom".
fn picked_template(s: &State, m: &Meeting) -> &'static str {
    let custom = s.meeting_ui.custom;
    let generated = serde_json::from_str::<serde_json::Value>(&m.summary_raw)
        .ok()
        .and_then(|v| {
            v.get("made_by")?
                .get("instructions")?
                .as_str()
                .map(str::to_string)
        });
    let picked = s
        .meeting_ui
        .templates
        .get(&m.id)
        .map(String::as_str)
        .unwrap_or(match generated.as_deref() {
            Some("custom") => "custom",
            Some(_) => "standard",
            None if custom => "custom",
            None => "standard",
        });
    if picked == "custom" && custom {
        "custom"
    } else {
        "standard"
    }
}

fn fact(icon: &'static rune::Icon, p: Palette) -> El {
    row()
        .h(28.0)
        .gap(6.0)
        .items_center()
        .px(6.0)
        .radius(8.0)
        .child(icons::draw(icon, 14.0, p.faint).a11y_hidden())
}

fn meeting_facts(m: &Meeting, p: Palette) -> Option<El> {
    let date = m.started.map(|d| d.format("%-d %b").to_string());
    let duration = m.minutes.map(|n| format!("{n} min"));
    let name = match (&date, &duration) {
        (Some(date), Some(duration)) => format!("Date: {date}. Duration: {duration}"),
        (Some(date), None) => format!("Date: {date}"),
        (None, Some(duration)) => format!("Duration: {duration}"),
        (None, None) => return None,
    };
    let separator = date.is_some() && duration.is_some();
    Some(
        fact(&icons::DIARY, p)
            .key("meeting-facts")
            .role("text")
            .label(name)
            .focusable(true)
            .focus_ring(2.0, 1.0, p.accent)
            .children(date.map(|date| roles::FACT.line(date, p.soft).a11y_hidden()))
            .children(separator.then(|| {
                widgets::dot(3.0, p.mark)
                    .key("meeting-fact-separator")
                    .a11y_hidden()
            }))
            .children(duration.map(|duration| roles::FACT.line(duration, p.soft).a11y_hidden())),
    )
}

fn view_switch(s: &State, k: &Kit, p: Palette) -> El {
    let selected = usize::from(s.meeting_tab == Tab::Notes);
    row()
        .key("meeting-view")
        .role("radiogroup")
        .label("Meeting view")
        .focus_group()
        .h(28.0)
        .padding(2.0)
        .radius(8.0)
        .fill(p.chip)
        .equal_columns(2)
        .child(
            el().key("indicator")
                .h(24.0)
                .radius(6.0)
                .fill(p.bg)
                .shadow(0.0, 1.0, p.shadow)
                .token_motion(k.motion.control)
                .a11y_hidden()
                .grid_cell(selected),
        )
        .children(
            [(Tab::Summary, "Notes"), (Tab::Notes, "Your notes")]
                .into_iter()
                .enumerate()
                .map(|(index, (tab, name))| {
                    let on = index == selected;
                    row()
                        .key(format!("segment{index}"))
                        .role("radio")
                        .label(name)
                        .position_in_set(index + 1, 2)
                        .focusable(true)
                        .focus_selected(on)
                        .focus_ring(2.0, 1.0, p.accent)
                        .h(24.0)
                        .px(10.0)
                        .center()
                        .radius(6.0)
                        .hover(p.button_hover)
                        .on_press_tx(move |s: &mut State, tx| {
                            s.set_meeting_tab(tab);
                            tx.blur();
                        })
                        .child(
                            rune::text(name)
                                .font(rune::TextStyle {
                                    weight: if on { 600.0 } else { k.ty.ui.weight },
                                    ..k.ty.ui
                                })
                                .color(if on { p.ink } else { p.soft })
                                .a11y_hidden(),
                        )
                        .grid_cell(index)
                }),
        )
}

const ENGLISH_ONLY: &str = "English-only transcript. Hindi or Telugu parts may be missing.";

/// Whether to say the transcript was English-only. It follows the word stamped in the summary on
/// screen, so a summary with no word shows it: nothing recorded that its language passes ran.
fn english_only(s: &State, m: &Meeting) -> bool {
    s.meeting_tab == Tab::Summary
        && m.has_summary()
        && m.summary_language != Some(core::notes::Language::Routed)
}

pub(super) fn status_chip(
    s: &State,
    k: &Kit,
    m: &Meeting,
    p: Palette,
    layer: &mut Vec<El>,
) -> Option<El> {
    if s.meeting_tab != Tab::Summary {
        return None;
    }
    let english = english_only(s, m);
    let engine = notes_status::meeting_engine_message(s, m);
    if !english && engine.is_none() {
        return None;
    }
    let close_on_escape = |s: &mut State, e: &KeyEvent, tx: &mut Tx| {
        if e.key != Key::Escape || !s.meeting_ui.status_open {
            return false;
        }
        s.meeting_ui.status_open = false;
        tx.focus("meeting-status", true);
        true
    };
    let (status, name) = if english {
        ("English only", "Transcript status: English only")
    } else {
        ("Transcript status", "Transcript status")
    };
    let toggle = pressable(
        fact(&icons::INFO, p)
            .px(8.0)
            .child(roles::FACT.line(status, p.faint))
            .fill(if s.meeting_ui.status_open {
                p.chip
            } else {
                Color::TRANSPARENT
            }),
    )
    .key("meeting-status")
    .label(name)
    .hover(p.chip)
    .focus_ring(2.0, 1.0, p.accent)
    .on_key(close_on_escape)
    .on_press_tx(|s: &mut State, tx| {
        s.meeting_ui.status_open = !s.meeting_ui.status_open;
        if s.meeting_ui.status_open && tx.by_keys() {
            tx.focus_when_shown("meeting-status-settings", true);
        }
    });
    let body = col()
        .w(274.0)
        .px(2.0)
        .gap(8.0)
        .on_key(close_on_escape)
        .children(english.then(|| {
            roles::SMALL
                .wrap(ENGLISH_ONLY, p.ink)
                .key("meeting-english-only")
        }))
        .children(engine.map(|copy| roles::SMALL.wrap(copy, p.ink).key("meeting-engine-status")))
        .child(
            quiet_button("Open language settings", "meeting-status-settings", p)
                .self_start()
                .on_press(|s: &mut State| {
                    s.meeting_ui.status_open = false;
                    s.open_hex();
                    s.hex.section = crate::ui::hex::Section::MeetingModels;
                }),
        );
    let mut kit = k.clone();
    kit.c.surface_raised = p.raised;
    kit.radius.md = 10.0;
    Some(
        Popover::new("meeting-status-popover")
            .label("Transcript status")
            .trigger(toggle)
            .open(s.meeting_ui.status_open)
            .placement(Placement::BottomStart)
            .on_close(|s: &mut State| s.meeting_ui.status_open = false)
            .child(body)
            .build(&kit, layer),
    )
}

/// `side` is the page gutter; the wide page puts the header in the notes column and passes 0.
fn header(s: &State, k: &Kit, m: &Meeting, side: f32, p: Palette, layer: &mut Vec<El>) -> El {
    let people = m.summary.people.len();
    let phone = super::phone(s);
    // Keep the phone controls together regardless of how wide the facts are.
    let mut first = row()
        .key("meeting-controls")
        .w_full()
        .gaps(6.0, 6.0)
        .items_center()
        .child(view_switch(s, k, p));
    let mut rest = Vec::new();
    if phone {
        rest.extend(meeting_facts(m, p));
    } else {
        first = first.flex_wrap().children(meeting_facts(m, p));
    }
    if people > 0 {
        rest.push(
            fact(&icons::PEOPLE, p)
                .key("meeting-people")
                .role("text")
                .label(match people {
                    1 => "1 person".to_string(),
                    n => format!("{n} people"),
                })
                .help(m.summary.people.join(", "))
                .focusable(true)
                .focus_ring(2.0, 1.0, p.accent)
                .child(roles::FACT.line(people.to_string(), p.soft).a11y_hidden()),
        );
    }
    rest.extend(project::reading_chip(s, k, &m.id, p, layer));
    rest.extend(status_chip(s, k, m, p, layer));
    let menu = more::menu(s, k, m, "meeting", p, layer);
    let meta = if phone {
        let mut meta = col().key("meeting-meta").w_full().gap(6.0);
        first = first.child(el().grow(1.0).h(28.0)).child(menu);
        meta = meta.child(first);
        if !rest.is_empty() {
            // The empty slot after the project is where the section rail's chip goes.
            meta = meta.child(
                row()
                    .key("meeting-controls-more")
                    .w_full()
                    .flex_wrap()
                    .gaps(6.0, 6.0)
                    .items_center()
                    .children(rest)
                    .child(el().key("meeting-sections-slot")),
            );
        }
        meta
    } else {
        row()
            .key("meeting-meta")
            .w_full()
            .gap(6.0)
            .items_start()
            .child(first.children(rest).grow(1.0).min_w(0.0))
            .child(menu)
    };
    col()
        .key("meeting-header")
        .w_full()
        .gap(12.0)
        .px(side)
        .pt(18.0)
        .pb(22.0)
        .child(
            // 26 on a page, 24 on a phone; the line is a quarter taller than the size.
            text_title(&m.title, if phone { 24.0 } else { 32.0 }, p)
                .max_lines(2)
                .key("meeting-title")
                .help(&m.title),
        )
        .child(meta)
        .children(crate::ui::live::parts_status(s, p))
        .children(project::form(s, k, &m.id, p))
}

/// The bullet before a note: an ink dot for a decision, a ring for an open point, a grey dot
/// for everything else. All three are 5 px, so the text starts at the same x in every row.
fn bullet(kind: &str, origin: &str, p: Palette) -> El {
    let mark = match kind {
        "decided" => widgets::dot(5.0, p.ink),
        "open" => el().size(5.0, 5.0).radius(2.5).border(1.0, p.mark),
        _ => widgets::dot(5.0, p.mark),
    };
    // 5 px in from the 6 px row padding puts the dot 11 px from the row edge; 9 px down centres
    // it on the first 23 px line below the 2 px row padding.
    col()
        .w(20.0)
        .pt(9.0)
        .pl(5.0)
        .child(mark.key(format!("meeting-bullet-{origin}")))
}

fn point(s: &State, pt: &Point, origin: String, timed: bool, p: Palette) -> El {
    let key = format!("meeting-point-{origin}");
    let source_key = format!("meeting-source-{origin}");
    let decided = pt.kind == "decided";
    let ink = match pt.kind.as_str() {
        "decided" => p.ink,
        "open" => p.soft,
        _ => p.body,
    };
    let mut body = roles::READ
        .wrap(&pt.text, ink)
        .key(format!("meeting-point-text-{origin}"))
        .selectable(true)
        .w_full();
    if decided {
        body = body.font(roles::READ.bold().style());
    }
    let mut line = row()
        .key(key)
        .w_full()
        .gap(0.0)
        .py(2.0)
        .pl(if pt.kind == "response" { 26.0 } else { 6.0 })
        .pr(6.0)
        .radius(6.0)
        .items_start()
        // Pointing at the line, or keyboard focus on its time, tints the row and shows the time.
        .hover(p.chip)
        .hover_group()
        .child(bullet(&pt.kind, &origin, p))
        .child(body.grow(1.0).min_w(0.0));
    if timed && let Some(at) = pt.at.iter().find(|at| core::seconds(at).is_some()) {
        let time = at.clone();
        let shown = crate::ui::widgets::recording_time(at);
        let back = origin.clone();
        let pill = pressable(roles::TIME.line(&shown, p.ink))
            .key(source_key)
            .label(format!("Open source at {shown}"))
            .help("Open this line in the transcript")
            .px(6.0)
            .py(2.0)
            .mt(1.0)
            .radius(5.0)
            .fill(p.sel)
            .hover(p.chip)
            .focus_ring(2.0, 1.0, p.accent)
            .group_opacity(0.0, 1.0)
            .on_press_tx(move |s: &mut State, tx| {
                s.jump(&time, back.clone(), tx);
            });
        // On a phone the hidden pill would still take its width from the text, so it floats over
        // the row's right end instead, 3 down (the row's 2 and the pill's 1) and 6 in.
        line = line.child(if super::phone(s) {
            pill.pin(None, Some(3.0), Some(6.0), None)
        } else {
            pill
        });
    }
    if s.back.as_deref() == Some(&origin) {
        line = line.fill(p.sel);
    }
    line
}

/// A topic's points as (summary index, point): decisions, then facts, then open points, each
/// group in the summary's order. A reply travels with the row before it, so a reply to a decision
/// stays under that decision. The index stays the summary's, so row keys and the way back from
/// the transcript do not move.
fn decisions_first(points: &[Point]) -> Vec<(usize, &Point)> {
    let group = |pt: &Point| match pt.kind.as_str() {
        "decided" => 0,
        "open" => 2,
        _ => 1,
    };
    let mut blocks: Vec<Vec<(usize, &Point)>> = Vec::new();
    for (i, pt) in points.iter().enumerate() {
        match blocks.last_mut() {
            Some(block) if pt.kind == "response" => block.push((i, pt)),
            _ => blocks.push(vec![(i, pt)]),
        }
    }
    blocks.sort_by_key(|block| group(block[0].1));
    blocks.into_iter().flatten().collect()
}

/// One section heading. Every heading on the page uses this role; the first sits right under the
/// header, which already ends 28 px above the notes, and each later one gets 28 px of its own.
fn heading(label: &str, key: String, first: bool, p: Palette) -> El {
    roles::SECTION
        .wrap(label, p.ink)
        .key(key)
        .selectable(true)
        .mt(if first { 0.0 } else { roles::SECTION_TOP })
        .mb(8.0)
}

fn narrative(s: &State, m: &Meeting, p: Palette) -> El {
    let mut body = col().key("summary-top").w_full().gap(0.0);
    if s.meeting_tab == Tab::Notes {
        return body.child(if m.notes.is_empty() {
            roles::READ.wrap("No personal notes for this meeting.", p.soft)
        } else {
            crate::ui::live::your_notes(&m.notes, p).key("sec-your-notes")
        });
    }
    body = body.children(crate::ui::live::recording_note(s, p));
    let draft = draft_summary(m);
    let summary = draft.as_ref().unwrap_or(&m.summary);
    if draft.is_some() {
        body = body.child(
            roles::META
                .wrap(DRAFT_NOTICE, p.soft)
                .key("draft-summary-notice")
                .mb(12.0),
        );
    }
    if !m.has_summary() && draft.is_none() {
        return body.children(notes_status::meeting_banner(s, m, p));
    }
    let mut first = true;
    if !summary.context.is_empty() {
        body = body
            .child(heading("Context", "sec-context".into(), first, p))
            .child(
                roles::READ
                    .wrap(summary.context.join(" "), p.body)
                    .key("ctx-0")
                    .selectable(true),
            );
        first = false;
    }
    let timed = m.timed();
    for (ti, topic) in summary.topics.iter().enumerate() {
        body = body.child(heading(&topic.title, format!("topic-{}", ti + 1), first, p));
        first = false;
        for (pi, pt) in decisions_first(&topic.points) {
            body = body.child(point(s, pt, format!("pt-{ti}-{pi}"), timed, p));
        }
    }
    if !summary.verify.is_empty() {
        body = body.child(heading(
            "Check before acting",
            "sec-verify".into(),
            first,
            p,
        ));
        for (i, line) in summary.verify.iter().enumerate() {
            body = body.child(
                roles::READ
                    .wrap(line, p.soft)
                    .key(format!("verify-{i}"))
                    .py(2.0)
                    .selectable(true),
            );
        }
    }
    body.children(
        draft
            .as_ref()
            .map(|_| crate::ui::notes_status::progress("Writing notes", p)),
    )
    // The rows have no gap of their own (column gap 0), so a bordered banner needs its own room.
    .children(notes_status::meeting_banner(s, m, p).map(|banner| banner.mt(16.0)))
    .children(crate::ui::notes_status::provenance(m, p))
}

/// The label over a rail group: the name at 600 in soft, the count at 400 in faint, the help hint.
fn rail_label(name: String, count: Option<usize>, hint: Option<El>, p: Palette) -> El {
    row()
        .items_center()
        .gap(6.0)
        .h(22.0)
        .px(8.0)
        .mb(4.0)
        .child(roles::FACT.bold().wrap(name, p.soft))
        .children(count.map(|n| roles::COUNT.wrap(n.to_string(), p.faint)))
        .children(hint)
}

/// The small label over one side of the tasks ("Ours 4", "They will 2").
fn rail_side(name: &str, count: usize, p: Palette) -> El {
    row()
        .items_center()
        .gap(6.0)
        .h(22.0)
        .px(8.0)
        .mb(4.0)
        .child(roles::META.bold().wrap(name, p.soft))
        .child(roles::META.wrap(count.to_string(), p.faint))
}

/// A task or question row: 8 all round, the 16 px mark at the left, the text beside it.
fn rail_row(mark: El, body: El) -> El {
    row()
        .w_full()
        .items_start()
        .gap(10.0)
        .padding(8.0)
        .radius(8.0)
        .child(mark.mt(1.0))
        .child(body.grow(1.0).min_w(0.0))
}

/// The line between two rail rows, inset like the row padding.
fn rail_rule(p: Palette) -> El {
    col().w_full().px(8.0).child(hairline(p))
}

fn rail(s: &State, k: &Kit, m: &Meeting, p: Palette, layer: &mut Vec<El>) -> El {
    let items: Vec<_> = s
        .tasks
        .all
        .iter()
        .filter(|t| t.meeting_id == m.id && !t.manual)
        .collect();
    let mut rail = col().key("meeting-actions");
    let mut top = 0.0;
    let mut task_notice = notes_status::tasks_notice(s, m, p).map(|n| n.px(8.0).mb(4.0));
    let groups: Vec<_> = [(false, "Ours"), (true, "They will")]
        .into_iter()
        .map(|(external, name)| {
            let group: Vec<_> = items
                .iter()
                .filter(|t| {
                    let side = t.owner.as_ref().and_then(|owner| {
                        s.people
                            .prefs
                            .people_sides
                            .get(&core::people::key(&s.tasks.teams.owner_name(owner)))
                    });
                    (side == Some(&Side::External)) == external
                })
                .collect();
            (external, name, group)
        })
        .filter(|(.., group)| !group.is_empty())
        .collect();
    // "Tasks 4". "Ours" only means something next to "They will", so the two sides get their
    // own small labels only when a They-will group exists.
    let sides = groups.iter().any(|(external, ..)| *external);
    if !groups.is_empty() {
        let total: usize = groups.iter().map(|(.., group)| group.len()).sum();
        let hint = HelpHint::Tasks.build(k, p, layer);
        rail = rail
            .child(rail_label("Tasks".into(), Some(total), Some(hint), p).key("meeting-task-count"))
            .children(task_notice.take());
    }
    let mut gap = 0.0;
    for (external, name, group) in groups {
        if sides {
            rail = rail.child(rail_side(name, group.len(), p).mt(gap).key(if external {
                "meeting-external-task-count"
            } else {
                "meeting-ours-task-count"
            }));
        }
        gap = roles::SECTION_TOP;
        let mut list = col().w_full().key(if external {
            "meeting-external-tasks"
        } else {
            "meeting-tasks"
        });
        for (i, t) in group.iter().enumerate() {
            if i > 0 {
                list = list.child(rail_rule(p));
            }
            let id = m.id.clone();
            let index = t.index;
            let text = t.task.clone();
            let mut title = roles::NAME
                .wrap(
                    tasks::shown_title(s, t),
                    if t.done { p.faint } else { p.ink },
                )
                .max_lines(2);
            if t.done {
                title = title.strike(0, tasks::shown_title(s, t).len(), None);
            }
            let due = t
                .due
                .as_deref()
                .map(core::nice_date)
                .unwrap_or_else(|| "No date".into());
            let owner = t.owner.as_deref().unwrap_or("No owner");
            let tick = pressable(icons::draw(
                if t.done { &icons::DONE } else { &icons::CIRCLE },
                16.0,
                if t.done { p.ok } else { p.soft },
            ))
            .key(format!("meeting-tick-{index}"))
            .role("checkbox")
            .checked(t.done)
            .label(format!(
                "Mark {} {}",
                t.title,
                if t.done { "open" } else { "done" }
            ))
            .on_press(move |s: &mut State| s.toggle_task_at(&id, &text));
            let task_id = m.id.clone();
            list = list.child(
                rail_row(
                    tick,
                    col()
                        .gap(1.0)
                        .child(
                            pressable(title)
                                .key(format!("meeting-task-open-{index}"))
                                .on_press(move |s: &mut State| {
                                    s.go_home();
                                    s.view = tasks::View::Tasks;
                                    s.open_task(&task_id, index);
                                }),
                        )
                        .children(
                            (!t.description.is_empty())
                                .then(|| roles::SMALL.line(&t.description, p.soft)),
                        )
                        .child(roles::META.wrap(
                            format!(
                                "{owner} \u{b7} {due}{}",
                                if t.due_default { " \u{b7} guessed" } else { "" }
                            ),
                            p.faint,
                        )),
                )
                .key(format!("meeting-task-{index}")),
            );
        }
        rail = rail.child(list);
        top = roles::SECTION_TOP;
    }
    if let Some(notice) = task_notice {
        rail = rail
            .child(rail_label("Tasks".into(), None, None, p).mt(top))
            .child(notice);
        top = roles::SECTION_TOP;
    }
    if !m.summary.to_settle.is_empty() {
        rail = rail.child(
            rail_label(
                "Open questions".into(),
                Some(m.summary.to_settle.len()),
                Some(HelpHint::Questions.build(k, p, layer)),
                p,
            )
            .mt(top),
        );
        let mut list = col().w_full().key("meeting-questions");
        for (i, q) in m.summary.to_settle.iter().enumerate() {
            if i > 0 {
                list = list.child(rail_rule(p));
            }
            list =
                list.child(
                    rail_row(
                        icons::draw(&icons::HELP, 16.0, p.soft),
                        col()
                            .gap(1.0)
                            .child(roles::SMALL.wrap(&q.question, p.ink))
                            .children(q.waits_on.as_ref().map(|name| {
                                roles::META.wrap(format!("Waiting on {name}"), p.faint)
                            })),
                    )
                    .key(format!("meeting-question-{i}")),
                );
        }
        rail = rail.child(list);
    }
    rail.child(crate::ui::memory::settled::meeting_footer(s, m, p).mt(20.0))
}

pub fn transcript_height(s: &State) -> f32 {
    (s.meeting_ui.window_h.get() * 0.45).clamp(140.0, 380.0)
}

/// Saved-meeting dock control. The pause lane can fill `meeting-recording-slot`.
pub(super) fn recording_control(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> El {
    crate::ui::live::parts_control(s, k, p, layer)
}

pub fn pane(s: &State, k: &Kit, p: Palette, narrow: bool, layer: &mut Vec<El>) -> El {
    let Some(m) = s.current() else {
        return col();
    };
    let wide = super::wide(s);
    let strip = super::strip(s);
    // The wide grid already sets the notes 44 points in from its edge, so its column has no
    // gutter of its own; below the grid's width the notes keep the page gutter.
    let side = super::gutter(s);
    // With no gutter and the tasks stacked, their rows hang 8 either side of the column, so the
    // scroll area is 8 wider each side there, as the tasks column's is on the wide grid.
    let hang = if strip && !wide { 8.0 } else { 0.0 };
    let fade = (150.0 - s.overlay_h.get()).max(0.0);
    let mut doc = narrative(s, m, p);
    if !wide {
        doc = doc.child(rail(s, k, m, p, layer).mx(-8.0).mt(roles::SECTION_TOP));
    }
    let mut main = col()
        .key("meeting-main")
        .on_key(|s: &mut State, e, _| {
            if e.key == Key::Escape && s.transcript_open {
                s.close_transcript_sheet();
                true
            } else {
                false
            }
        })
        .min_h(0.0);
    main = if strip {
        main.w(COLUMN).min_w(COLUMN).max_w(COLUMN)
    } else {
        main.grow(1.0).min_w(0.0)
    };
    let mut frame = col().key("meeting-scroll-frame");
    frame = if hang > 0.0 {
        let w = COLUMN + 2.0 * hang;
        frame.w(w).min_w(w).max_w(w).ml(-hang).mr(-hang)
    } else {
        frame.w_full()
    };
    main = main.child(
        frame
            .grow(1.0)
            .min_w(0.0)
            .min_h(0.0)
            .child(
                scroll(
                    doc.max_w(COLUMN + 2.0 * (side + hang))
                        .px(side + hang)
                        .pt(6.0)
                        .pb(s.overlay_h.get() + 32.0),
                    "meeting-scroll",
                )
                .grow(1.0),
            )
            // The fade is 150 from the window bottom. The Ask stack sits below the scroll area
            // here, so only the part above it is drawn; with a thread open there is none.
            .child(
                el().key("meeting-scroll-fade")
                    .pin(Some(0.0), None, Some(0.0), Some(0.0))
                    .h(fade)
                    .linear_gradient(
                        std::f32::consts::PI,
                        [
                            (0.0, p.bg.alpha(0.0)),
                            (1.0, p.bg.alpha((fade / 75.0).min(1.0))),
                        ],
                    ),
            ),
    );
    let control = recording_control(s, k, p, layer);
    let panel = s.transcript_open.then(|| {
        col()
            .key("meeting-transcript-view")
            .focus_return("meeting-transcript")
            .w_full()
            .h(transcript_height(s))
            .min_h(transcript_height(s))
            .max_h(transcript_height(s))
            .child(transcript_panel(s, k, m, true, p))
    });
    main = main.child(if let Some(panel) = panel {
        col()
            .key("meeting-transcript-dock")
            .w_full()
            .items_start()
            .shrink()
            .px(side)
            .pb(36.0)
            .child(
                col()
                    .key("meeting-transcript-surface")
                    .max_w(COLUMN)
                    .w_full()
                    .radius(14.0)
                    .clip_rounded(14.0)
                    .fill(p.card)
                    .border(1.0, p.line)
                    .child(panel)
                    .child(hairline(p))
                    .child(control.self_start()),
            )
    } else {
        ask::overlay_with(s, k, p, narrow, layer, Some(control), None)
    });
    let mut columns = row().w_full().grow(1.0).min_h(0.0).min_w(0.0);
    if wide {
        columns = columns.pr(EDGE);
        // The tasks' rows hang 8 either side of their column, like the mock, so the text gap to the
        // notes is the 40 and the rows' hairlines and check circles are not cut by the scroll area.
        let tasks = scroll(
            col().px(8.0).pb(24.0).child(
                col()
                    .key("meeting-tasks-column")
                    .w(TASKS)
                    .child(rail(s, k, m, p, layer).mx(-8.0)),
            ),
            "meeting-rail-scroll",
        )
        .w(TASKS + 16.0)
        .min_w(TASKS + 16.0)
        .max_w(TASKS + 16.0)
        .grow(0.0)
        .ml(GAP - 8.0)
        .mr(-8.0);
        columns = columns
            .justify_center()
            .child(grid("meeting-grid", main, GRID).child(tasks));
    } else if strip {
        // Narrower than the grid but wide enough for the strip: the notes keep their column and
        // the tasks sit under them.
        columns = columns
            .justify_center()
            .child(grid("meeting-grid", main, STRIP + COLUMN));
    } else {
        columns = columns.child(main);
    }
    let mut head = header(s, k, m, side, p, layer);
    if strip {
        head = row()
            .w_full()
            .pr(if wide { EDGE } else { 0.0 })
            .justify_center()
            .child(grid(
                "meeting-header-grid",
                head.w(COLUMN).min_w(COLUMN).max_w(COLUMN),
                if wide { GRID } else { STRIP + COLUMN },
            ));
    }
    let mut pane = col()
        .key("meeting-pane")
        .w_full()
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .fill(p.bg);
    if let Some(note) = &s.notice {
        pane = pane.child(notice_bar(note, p));
    }
    pane.child(head).child(columns)
}

/// The grid row, `width` wide: the 44 point strip kept for the section rail, then `notes`. The
/// header and the page body both sit in one, so the title lines up with the notes.
fn grid(key: &str, notes: El, width: f32) -> El {
    row()
        .key(key)
        .w(width)
        .min_w(width)
        .max_w(width)
        .child(
            el().key(format!("{key}-strip"))
                .w(STRIP)
                .min_w(STRIP)
                .max_w(STRIP),
        )
        .child(notes)
}

/// Publication-arrival scene, confined to the renderer's invented meeting directory.
pub fn scene_publication(s: &mut State, home: bool) {
    assert!(crate::ui::isolated());
    crate::ui::scene_screens(s);
    let current = s.current().unwrap().clone();
    assert!(current.folder.starts_with(&s.store.root));
    let folder = if home {
        current.folder.parent().unwrap().join("2000000000000-2")
    } else {
        current.folder
    };
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("manifest.json"), serde_json::json!({"title":"Meeting at 15:14","title_source":"default",
        "status":"complete","created_at_ms":chrono::Utc::now().timestamp_millis()-120_000,"duration_ms":60_000}).to_string()).unwrap();
    std::fs::write(
        folder.join("english-summary.json"),
        serde_json::json!({"title":"Latest workshop decisions",
        "context":["A synthetic meeting published while this window stays open."]})
        .to_string(),
    )
    .unwrap();
    let options = s.refresh_options();
    s.apply_event(crate::ui::Event::Refreshed(crate::ui::refresh::Result {
        generation: core::mutation_generation(),
        options,
        data: Some(Box::new(crate::ui::refresh::Data::read(options))),
    }));
    if home {
        s.go_home();
    }
}
