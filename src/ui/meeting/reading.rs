//! Reading keeps the narrative, actions and transcript in their own scroll areas.
use super::*;
use crate::ui::{cards, icons, tasks};
use rune_kit::{Menu, MenuItem, SegmentedControl};
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

pub fn menu_items(s: &State, m: &Meeting, menu: Menu) -> Menu {
    let id = m.id.clone();
    let mut out = menu
        .separator()
        .item(
            MenuItem::new("Ask model and effort").on_select(|s: &mut State| {
                s.more_menu = None;
                s.open_hex();
                s.hex.section = crate::ui::hex::Section::Connect;
                crate::ui::scene_scroll(s, "ask-agent-row");
            }),
        )
        .item(MenuItem::new("Project").on_select({
            let id = m.id.clone();
            move |s: &mut State| {
                s.more_menu = None;
                s.project_menu = Some(id.clone());
            }
        }))
        .item(MenuItem::new("Memory from this meeting").on_select({
            let id = m.id.clone();
            move |s: &mut State| {
                s.more_menu = None;
                s.open_memory();
                s.memory.settle_all = true;
                s.memory.settle_meeting = Some(id.clone());
            }
        }))
        .group_label("View")
        .item(
            MenuItem::new("Summary")
                .checked(main_tab(s, m) == Tab::Summary)
                .on_select(|s: &mut State| {
                    s.set_meeting_tab(Tab::Summary);
                    s.more_menu = None;
                }),
        )
        .item(
            MenuItem::new("Your notes")
                .disabled(m.notes.is_empty())
                .checked(main_tab(s, m) == Tab::Notes)
                .on_select(|s: &mut State| {
                    s.set_meeting_tab(Tab::Notes);
                    s.more_menu = None;
                }),
        )
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
        .group_label("Next summary uses");
    let picked = picked_template(s, m);
    for (title, choice, enabled) in [
        ("Standard", "standard", true),
        ("Your instructions", "custom", s.meeting_ui.custom),
    ] {
        let id = id.clone();
        out = out.item(
            MenuItem::new(title)
                .checked(choice == picked)
                .disabled(!enabled)
                .on_select(move |s: &mut State| s.choose_summary_template(&id, choice)),
        );
    }
    out = out.item(
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

/// "15 Nov · 2 min". A part that is not known is left out, so nobody sees "Length not recorded".
fn facts_line(date: Option<String>, minutes: Option<u64>) -> String {
    [date, minutes.map(|n| format!("{n} min"))]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" \u{b7} ")
}

/// A small icon and its words on the meta row.
fn fact(icon: &'static rune::Icon, words: String, p: Palette) -> El {
    row()
        .gap(5.0)
        .items_center()
        .child(icons::draw(icon, 14.0, p.faint))
        .child(roles::META.line(words, p.soft))
}

fn header(s: &State, k: &Kit, m: &Meeting, p: Palette, layer: &mut Vec<El>) -> El {
    let facts = facts_line(m.started.map(|d| d.format("%-d %b").to_string()), m.minutes);
    let people = m.summary.people.len();
    let mut controls = row()
        .key("meeting-controls")
        .w_full()
        .flex_wrap()
        .gap(12.0)
        .items_center()
        .child(
            SegmentedControl::new(
                &["Notes", "Your notes"],
                Some(usize::from(s.meeting_tab == Tab::Notes)),
            )
            .label("Meeting view")
            .fit()
            .on_pick::<State>(|s, i, tx| {
                s.set_meeting_tab(if i == 0 { Tab::Summary } else { Tab::Notes });
                tx.blur();
            })
            .build(k)
            .key("meeting-view"),
        );
    if !facts.is_empty() {
        controls = controls.child(fact(&icons::DIARY, facts, p).key("meeting-facts"));
    }
    if people > 0 {
        controls = controls.child(
            fact(&icons::PEOPLE, people.to_string(), p)
                .key("meeting-people")
                .label(match people {
                    1 => "1 person".to_string(),
                    n => format!("{n} people"),
                })
                .help(m.summary.people.join(", ")),
        );
    }
    if let Some(project) = project::reading_chip(s, k, &m.id, p, layer) {
        controls = controls.child(project);
    }
    controls = controls.child(el().grow(1.0));
    controls = controls.child(more::menu(s, k, m, "meeting", p, layer));
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
        .children(crate::ui::live::parts_status(s, p))
        .children(project::form(s, k, &m.id, p))
}

fn point(s: &State, pt: &Point, origin: String, p: Palette) -> El {
    let key = format!("meeting-point-{origin}");
    let source_key = format!("meeting-source-{origin}");
    let mut body = roles::READ
        .wrap(&pt.text, p.ink)
        .key(format!("meeting-point-text-{origin}"))
        .selectable(true)
        .w_full();
    if pt.kind == "decided" {
        body = body.font(roles::READ.bold().style());
    }
    let mut line = row()
        .key(key)
        .w_full()
        .gap(8.0)
        .py(4.0)
        .px(6.0)
        .radius(6.0)
        .items_start()
        // Pointing at the line, or keyboard focus on its time, tints the row and shows the time.
        .hover(p.chip)
        .hover_group()
        .child(roles::READ.wrap("•", p.soft).w(12.0))
        .child(body.grow(1.0).min_w(0.0));
    if let Some(at) = pt.at.first() {
        let time = at.clone();
        let shown = crate::ui::widgets::recording_time(at);
        let back = origin.clone();
        line = line.child(
            pressable(roles::META.line(&shown, p.ink))
                .key(source_key)
                .label(format!("Open source at {shown}"))
                .help("Open this line in the transcript")
                .px(6.0)
                .py(2.0)
                .radius(5.0)
                .fill(p.sel)
                .hover(p.hit)
                .group_opacity(0.0, 1.0)
                .on_press_tx(move |s: &mut State, tx| {
                    s.jump(&time, back.clone(), tx);
                }),
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
            roles::READ.wrap("No personal notes for this meeting.", p.soft)
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
        return body.children(crate::ui::notes_status::banner(s, m, p));
    }
    if !summary.context.is_empty() {
        body = body.child(
            roles::SECTION
                .wrap("Context", p.ink)
                .key("sec-context")
                .selectable(true)
                .mb(6.0),
        );
        for (i, line) in summary.context.iter().enumerate() {
            body = body.child(
                roles::READ
                    .wrap(line, p.ink)
                    .key(format!("ctx-{i}"))
                    .selectable(true),
            );
        }
    }
    for (ti, topic) in summary.topics.iter().enumerate() {
        body = body.child(
            roles::TITLE
                .wrap(&topic.title, p.ink)
                .key(format!("topic-{}", ti + 1))
                .selectable(true)
                .mt(22.0)
                .mb(4.0),
        );
        for (pi, pt) in topic.points.iter().enumerate() {
            body = body.child(point(s, pt, format!("pt-{ti}-{pi}"), p));
        }
    }
    if !summary.verify.is_empty() {
        body = body.child(
            roles::TITLE
                .wrap("Check before acting", p.ink)
                .selectable(true)
                .mt(20.0),
        );
        for (i, line) in summary.verify.iter().enumerate() {
            body = body.child(
                roles::READ
                    .wrap(line, p.soft)
                    .key(format!("verify-{i}"))
                    .selectable(true),
            );
        }
    }
    body.children(
        draft
            .as_ref()
            .map(|_| crate::ui::notes_status::progress("Writing notes", p)),
    )
    .children(crate::ui::notes_status::banner(s, m, p))
    .children(crate::ui::notes_status::provenance(m, p))
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
        .child(
            col()
                .key("meeting-scroll-frame")
                .w_full()
                .grow(1.0)
                .min_w(0.0)
                .min_h(0.0)
                .child(
                    scroll(
                        doc.max_w(COLUMN)
                            .px(24.0)
                            .pt(6.0)
                            .pb(s.overlay_h.get() + 32.0),
                        "meeting-scroll",
                    )
                    .grow(1.0),
                )
                // The page fades into the Ask bar, so a chip at the cut edge never shows as a
                // sliver of a bar.
                .child(
                    el().key("meeting-scroll-fade")
                        .pin(Some(0.0), None, Some(0.0), Some(0.0))
                        .h(28.0)
                        .linear_gradient(
                            std::f32::consts::PI,
                            [(0.0, p.bg.alpha(0.0)), (1.0, p.bg)],
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
            .items_center()
            .shrink()
            .px(16.0)
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

#[cfg(test)]
mod tests {
    use super::facts_line;

    #[test]
    fn the_facts_line_leaves_out_what_is_not_known() {
        let date = || Some("17 Nov".to_string());
        assert_eq!(facts_line(date(), Some(2)), "17 Nov \u{b7} 2 min");
        assert_eq!(facts_line(date(), None), "17 Nov");
        assert_eq!(facts_line(None, Some(22)), "22 min");
        assert_eq!(facts_line(None, None), "");
    }
}
