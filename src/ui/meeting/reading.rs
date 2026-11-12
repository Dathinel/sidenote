//! Reading keeps the narrative, actions and transcript in their own scroll areas.
use super::*;
use crate::ui::{cards, icons, tasks};
use rune_kit::{Menu, MenuItem, Placement, SegmentedControl};
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

pub fn sheet_bottom(s: &State) -> f32 {
    s.meeting_ui.dock_h.get() + 10.0
}

pub fn sheet_height(s: &State) -> f32 {
    let available = (s.viewport_h.get() - sheet_bottom(s) - 8.0).max(180.0);
    s.meeting_ui
        .sheet_h
        .unwrap_or(s.meeting_ui.window_h.get() * 0.55)
        .clamp(180.0, available)
}

fn resize_handle(s: &State, p: Palette) -> El {
    col()
        .key("transcript-resize")
        .w_full()
        .h(18.0)
        .shrink()
        .center()
        .cursor(rune::Cursor::ResizeRow)
        .role("separator")
        .focusable(true)
        .label(format!(
            "Resize transcript, {} pixels",
            sheet_height(s).round()
        ))
        .help("Drag up or down, or use the arrow keys to resize the transcript.")
        .hover(p.chip)
        .child(el().w(36.0).h(4.0).radius(2.0).fill(p.faint.alpha(0.5)))
        .on_drag(|s: &mut State, d, _| match d.phase {
            rune::Phase::Down => {
                d.data = if s.meeting_ui.sheet_h.is_some() {
                    sheet_height(s)
                } else {
                    -sheet_height(s)
                }
            }
            rune::Phase::Move => s.meeting_ui.sheet_h = Some(d.data.abs() - d.dy),
            rune::Phase::Up => {
                if d.cancelled() {
                    s.meeting_ui.sheet_h = (d.data > 0.0).then_some(d.data);
                } else {
                    s.meeting_ui.sheet_h = Some(sheet_height(s));
                }
            }
        })
        .on_key(|s: &mut State, e, _| {
            let change = match e.key {
                Key::Up => 24.0,
                Key::Down => -24.0,
                _ => return false,
            };
            s.meeting_ui.sheet_h = Some(sheet_height(s) + change);
            true
        })
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
                self.meeting_ui.template_open = false;
            }
            Err(_) => self.notice = Some("Could not save the summary template. Try again.".into()),
        }
    }
}

pub fn menu_items(s: &State, m: &Meeting, menu: Menu) -> Menu {
    let id = m.id.clone();
    let mut out = menu
        .separator()
        .group_label("Layout")
        .item(
            MenuItem::new("Reading")
                .checked(!s.meeting_ui.tabs)
                .on_select(|s: &mut State| s.set_meeting_layout(false)),
        )
        .item(
            MenuItem::new("Tabs")
                .checked(s.meeting_ui.tabs)
                .on_select(|s: &mut State| s.set_meeting_layout(true)),
        )
        .separator()
        .item(
            MenuItem::new("Edit summary prompt").on_select(|s: &mut State| {
                s.more_menu = None;
                s.open_hex();
                s.hex.section = crate::ui::hex::Section::PromptLibrary;
                crate::ui::scene_scroll(s, "agent-instructions");
            }),
        );
    out=out.item(MenuItem::new("Write an email").on_select(|s:&mut State| {
        s.more_menu=None;s.question=TextEdit::new("Draft a short email about this meeting. Use only what was discussed and keep it ready to copy.");
        s.question.focused = true;
    }));
    if !m.lines.is_empty() {
        out = out.item(
            MenuItem::new("Copy transcript")
                .on_select_tx(move |s: &mut State, tx| s.copy_transcript(&id, tx)),
        );
    }
    out
}

fn template(s: &State, k: &Kit, m: &Meeting, p: Palette, layer: &mut Vec<El>) -> El {
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
    let picked = if picked == "custom" && !custom {
        "standard"
    } else {
        picked
    };
    let shown = if picked == "custom" {
        "Your instructions"
    } else {
        "Standard"
    };
    let trigger = tasks::value_button(
        row()
            .gap(5.0)
            .items_center()
            .child(roles::META.line(format!("Summary: {shown}"), p.soft))
            .child(icons::draw(&icons::CHEVRON_DOWN, 16.0, p.soft)),
        "meeting-template",
        format!("Summary: {shown}"),
        p,
    )
    .help("Choose the instructions for the next summary. The current meeting summary is kept.")
    .on_press(|s: &mut State| s.meeting_ui.template_open = !s.meeting_ui.template_open);
    let mut menu = Menu::new("meeting-template-menu")
        .trigger(trigger)
        .open(s.meeting_ui.template_open)
        .placement(Placement::BottomStart)
        .min_width(240.0);
    if s.meeting_ui.template_open {
        menu = menu.on_close(|s: &mut State| s.meeting_ui.template_open = false);
        for (title, choice, enabled) in [
            ("Standard", "standard", true),
            ("Your instructions", "custom", custom),
        ] {
            let id = m.id.clone();
            menu = menu.item(
                MenuItem::new(title)
                    .checked(choice == picked)
                    .disabled(!enabled)
                    .on_select(move |s: &mut State| s.choose_summary_template(&id, choice)),
            );
        }
        menu = menu
            .separator()
            .item(
                MenuItem::new("Edit summary prompt").on_select(|s: &mut State| {
                    s.meeting_ui.template_open = false;
                    s.open_hex();
                    s.hex.section = crate::ui::hex::Section::PromptLibrary;
                    crate::ui::scene_scroll(s, "agent-instructions");
                }),
            );
    }
    menu.build(k, layer)
}

fn header(s: &State, k: &Kit, m: &Meeting, p: Palette, layer: &mut Vec<El>) -> El {
    let date = m
        .started
        .map(|d| d.format("%-d %b").to_string())
        .unwrap_or_else(|| "Date not recorded".into());
    let length = m
        .minutes
        .map(|n| format!("{n} min"))
        .unwrap_or_else(|| "Length not recorded".into());
    let facts = format!(
        "{date} \u{b7} {length} \u{b7} {} {}",
        m.summary.people.len(),
        if m.summary.people.len() == 1 {
            "person"
        } else {
            "people"
        }
    );
    let switch = SegmentedControl::new(
        &["Summary", "Your notes"],
        Some(usize::from(s.meeting_tab == Tab::Notes)),
    )
    .label("Meeting view")
    .width(Some(188.0))
    .on_pick::<State>(|s, i, tx| {
        s.set_meeting_tab(if i == 0 { Tab::Summary } else { Tab::Notes });
        tx.blur();
    })
    .build(k)
    .key("meeting-view");
    let mut controls = row()
        .key("meeting-controls")
        .w_full()
        .flex_wrap()
        .gap(8.0)
        .items_center()
        .child(switch)
        .child(
            chip(facts, p)
                .key("meeting-facts")
                .help(m.summary.people.join(", ")),
        );
    if let Some(project) = project::reading_chip(s, k, &m.id, p, layer) {
        controls = controls.child(project);
    }
    controls = controls
        .child(template(s, k, m, p, layer))
        .child(more::menu(s, k, m, "meeting", p, layer));
    let ours: Vec<_> = m
        .summary
        .people
        .iter()
        .map(|n| s.tasks.teams.owner_name(n))
        .filter(|name| {
            s.people
                .affiliations
                .side(name, &s.tasks.teams, &s.people.prefs)
                == Some(Side::OurTeam)
        })
        .collect();
    col()
        .key("meeting-header")
        .w_full()
        .gap(10.0)
        .px(24.0)
        .pt(18.0)
        .pb(16.0)
        .child(
            text_title(&m.title, 32.0, p)
                .max_lines(2)
                .key("meeting-title")
                .help(&m.title),
        )
        .child(controls)
        .child(
            roles::META
                .line(
                    format!(
                        "Our side: {}",
                        if ours.is_empty() {
                            "not set".into()
                        } else {
                            ours.join(", ")
                        }
                    ),
                    p.soft,
                )
                .key("meeting-our-side"),
        )
        .children(project::form(s, k, &m.id, p))
        .children(crate::ui::notes_status::banner(s, m, p))
}

fn point(s: &State, pt: &Point, origin: String, p: Palette) -> El {
    let key = format!("meeting-point-{origin}");
    let source_key = format!("meeting-source-{origin}");
    let mut body = roles::READ.wrap(&pt.text, p.ink).w_full();
    if pt.kind == "decided" {
        body = body.font(roles::READ.bold().style());
    }
    let mut line = row()
        .key(key)
        .w_full()
        .gap(8.0)
        .py(4.0)
        .items_start()
        .hover_group()
        .child(roles::READ.wrap("•", p.soft).w(12.0))
        .child(body.grow(1.0).min_w(0.0));
    if let Some(at) = pt.at.first() {
        let time = at.clone();
        let back = origin.clone();
        let meta = match pt.who.as_deref().filter(|who| !who.is_empty()) {
            Some(who) => format!("{who} \u{b7} {}", crate::ui::widgets::recording_time(at)),
            None => crate::ui::widgets::recording_time(at),
        };
        let source = roles::META
            .line(&meta, p.soft.alpha(0.0))
            .max_w(140.0)
            .key(source_key)
            .group_color(p.soft)
            .link(
                rune::Link::new(0..meta.len(), origin.clone())
                    .label(format!("Open source at {at}"))
                    .help("Open this line in the transcript")
                    .focused(rune::LinkPaint::new().color(p.soft).bg(p.chip)),
            )
            .on_link(move |s: &mut State, _, tx| s.jump(&time, back.clone(), tx));
        let time = at.clone();
        let back = origin.clone();
        line = line.child(
            row().gap(4.0).items_center().child(source).child(
                pressable(icons::draw(&icons::SEARCH, 16.0, p.soft))
                    .key(format!("meeting-source-icon-{origin}"))
                    .label(format!("Open source at {at}"))
                    .focusable(false)
                    .group_opacity(0.0, 1.0)
                    .on_press_tx(move |s: &mut State, tx| {
                        s.jump(&time, back.clone(), tx);
                        tx.blur();
                    }),
            ),
        );
    }
    if s.back.as_deref() == Some(&origin) {
        line = line.fill(p.hit);
    }
    line
}

fn narrative(s: &State, m: &Meeting, p: Palette) -> El {
    let mut body = col().key("summary-top").w_full().gap(4.0);
    if s.meeting_tab == Tab::Notes {
        return body.child(if m.notes.is_empty() {
            roles::READ.wrap("Your notes are empty. Add a note.", p.soft)
        } else {
            crate::ui::live::your_notes(&m.notes, p).key("sec-your-notes")
        });
    }
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
        return body.child(roles::READ.wrap(
            "The meeting summary will appear here when it is ready.",
            p.soft,
        ));
    }
    if !summary.context.is_empty() {
        body = body.child(
            roles::SECTION
                .wrap("Context", p.ink)
                .key("sec-context")
                .mb(6.0),
        );
        for (i, line) in summary.context.iter().enumerate() {
            body = body.child(roles::READ.wrap(line, p.ink).key(format!("ctx-{i}")));
        }
    }
    for (ti, topic) in summary.topics.iter().enumerate() {
        body = body.child(
            roles::TITLE
                .wrap(&topic.title, p.ink)
                .key(format!("topic-{}", ti + 1))
                .mt(22.0)
                .mb(4.0),
        );
        for (pi, pt) in topic.points.iter().enumerate() {
            body = body.child(point(s, pt, format!("pt-{ti}-{pi}"), p));
        }
    }
    if !summary.verify.is_empty() {
        body = body.child(roles::TITLE.wrap("Check before acting", p.ink).mt(20.0));
        for (i, line) in summary.verify.iter().enumerate() {
            body = body.child(roles::READ.wrap(line, p.soft).key(format!("verify-{i}")));
        }
    }
    body.children(crate::ui::notes_status::provenance(m, p))
}

fn rail(s: &State, m: &Meeting, p: Palette) -> El {
    let items: Vec<_> = s
        .tasks
        .all
        .iter()
        .filter(|t| t.meeting_id == m.id && !t.manual)
        .collect();
    let mut rail = col().key("meeting-actions").w_full().gap(12.0);
    for (external, name) in [(false, "Ours"), (true, "They will")] {
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
        if group.is_empty() {
            continue;
        }
        rail = rail.child(
            roles::GROUP
                .wrap(format!("Tasks \u{b7} {name} {}", group.len()), p.soft)
                .key(if external {
                    "meeting-external-task-count"
                } else {
                    "meeting-task-count"
                }),
        );
        let mut card = cards::panel(p).key(if external {
            "meeting-external-tasks"
        } else {
            "meeting-tasks"
        });
        for (i, t) in group.iter().enumerate() {
            if i > 0 {
                card = card.child(hairline(p));
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
                18.0,
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
            card = card.child(
                cards::row_shell()
                    .key(format!("meeting-task-{index}"))
                    .items_start()
                    .padding(10.0)
                    .child(tick.mt(3.0))
                    .child(
                        col()
                            .grow(1.0)
                            .min_w(0.0)
                            .gap(3.0)
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
                                    .then(|| roles::META.line(&t.description, p.soft)),
                            )
                            .child(roles::META.wrap(
                                format!(
                                    "{owner} · {due}{}",
                                    if t.due_default { " · guessed" } else { "" }
                                ),
                                p.soft,
                            )),
                    ),
            );
        }
        rail = rail.child(card);
    }
    if !items.is_empty() {
        rail = rail.children(crate::ui::notes_status::tasks_notice(s, m, p));
    }
    if !m.summary.to_settle.is_empty() {
        rail = rail.child(
            roles::GROUP
                .wrap(
                    format!("Open questions · {}", m.summary.to_settle.len()),
                    p.soft,
                )
                .mt(8.0),
        );
        let mut card = cards::panel(p).key("meeting-questions");
        for (i, q) in m.summary.to_settle.iter().enumerate() {
            if i > 0 {
                card = card.child(hairline(p));
            }
            card = card.child(
                cards::row_shell()
                    .padding(10.0)
                    .items_start()
                    .child(icons::draw(&icons::HELP, 18.0, p.soft))
                    .child(
                        col()
                            .grow(1.0)
                            .min_w(0.0)
                            .gap(3.0)
                            .child(roles::NAME.wrap(&q.question, p.ink))
                            .children(q.waits_on.as_ref().map(|name| {
                                roles::META.wrap(format!("Waiting on {name}"), p.soft)
                            })),
                    ),
            );
        }
        rail = rail.child(card);
    }
    rail.child(crate::ui::memory::settled::meeting_footer(s, m, p).mt(8.0))
}

pub fn pane(s: &State, k: &Kit, p: Palette, narrow: bool, layer: &mut Vec<El>) -> El {
    let Some(m) = s.current() else {
        return col();
    };
    let wide = s.app_w.get() - crate::ui::nav::width(s) >= 980.0;
    let mut doc = narrative(s, m, p);
    if !wide {
        doc = doc.child(rail(s, m, p).mt(24.0));
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
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .child(scroll(
            doc.max_w(COLUMN)
                .px(24.0)
                .pt(6.0)
                .pb(s.overlay_h.get() + 32.0),
            "meeting-scroll",
        ));
    if s.transcript_open {
        main = main.child(
            el().key("meeting-summary-dim")
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .fill(rune::Color::BLACK.alpha(0.1))
                .label("Close transcript")
                .on_press(|s: &mut State| s.close_transcript_sheet()),
        );
    }
    let transcript = tasks::value_button(
        row()
            .gap(6.0)
            .items_center()
            .child(icons::draw(&icons::AUDIO_LINES, 16.0, p.soft))
            .child(roles::CONTROL.line("Transcript", p.soft)),
        "meeting-transcript",
        "Transcript".into(),
        p,
    )
    .shrink()
    .on_press_tx(|s: &mut State, tx| {
        s.toggle_transcript();
        tx.blur();
    });
    let transcript = if narrow {
        transcript
    } else {
        transcript.self_center()
    };
    main = main.child(ask::overlay_with(
        s,
        k,
        p,
        narrow,
        layer,
        Some(transcript),
        None,
    ));
    if s.transcript_open {
        main = main.child(
            col()
                .key("transcript-sheet")
                .focus_return("meeting-transcript")
                .pin(Some(16.0), None, Some(16.0), Some(sheet_bottom(s)))
                .h(sheet_height(s))
                .min_h(0.0)
                .radius(10.0)
                .clip_rounded(10.0)
                .padding(1.0)
                .border(1.0, p.faint.alpha(0.4))
                .fill(p.card)
                .shadow(8.0, 28.0, p.shadow)
                .child(resize_handle(s, p))
                .child(transcript_panel(s, k, m, true, p).grow(1.0)),
        );
    }
    let mut columns = row().w_full().grow(1.0).min_h(0.0).min_w(0.0).child(main);
    if wide {
        columns = columns.child(
            scroll(rail(s, m, p).px(12.0).pb(24.0), "meeting-rail-scroll")
                .w(324.0)
                .min_w(324.0)
                .max_w(324.0)
                .grow(0.0),
        );
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
    pane.child(header(s, k, m, p, layer)).child(columns)
}
