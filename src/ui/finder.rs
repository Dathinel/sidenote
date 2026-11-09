//! The Ctrl+K palette: one box that jumps to a meeting, a page or an action.
//!
//! An empty box lists the actions, the pages and the newest meetings. Typing narrows all three:
//! meetings by the same search the Home list uses, pages and actions by name. Tab hands the query
//! to the Ask bar. A feature that is switched off in Settings never appears here.
use super::{
    State, hex,
    list::{people_line, snippet},
    nav::Page,
    roles,
    tasks::View,
    theme::Palette,
    widgets::{hairline, keycap, pressable},
};
use rune::{El, Key, KeyEvent, ScrollSpec, TextEdit, Tx, col, el, row};
use rune_kit::{Kit, TextInput};
use sidenote::core::{self, SearchHit};

/// How many meetings a typed query lists.
const MEETING_ROWS: usize = 6;
/// How many of the newest meetings an empty box lists.
const RECENT_ROWS: usize = 5;

#[derive(Clone)]
pub enum Target {
    Meeting(String),
    Meetings,
    Tasks,
    Memory,
    Check,
    Settings(Option<hex::Section>),
    NewMeeting,
    Ask,
    Area(String),
    Project(String),
    Areas,
}

pub struct Entry {
    section: &'static str,
    title: String,
    detail: String,
    hit: Option<SearchHit>,
    target: Target,
}

pub struct Finder {
    entries: Vec<Entry>,
    cursor: usize,
}

fn contains(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(needle)
}

/// What the palette lists for `query`. Pages and actions come after the meetings when there is
/// a query, and before them when the box is empty.
fn entries(s: &State, query: &str) -> Vec<Entry> {
    let q = query.trim().to_lowercase();
    let typed = !q.is_empty();
    let f = &s.config.features;

    let mut actions = Vec::new();
    if hex::start_blocked(s).is_none() {
        actions.push(Entry {
            section: "Actions",
            title: "New meeting".into(),
            detail: "Start recording".into(),
            hit: None,
            target: Target::NewMeeting,
        });
    }
    if s.has_agent() {
        actions.push(Entry {
            section: "Actions",
            title: "Ask".into(),
            detail: if s.selected.is_some() {
                "Ask your agent about this meeting".into()
            } else {
                "Ask your agent about your meetings".into()
            },
            hit: None,
            target: Target::Ask,
        });
    }
    actions.retain(|e| !typed || contains(&e.title, &q) || contains(&e.detail, &q));

    let page = |title: &str, detail: &str, target: Target| Entry {
        section: "Pages",
        title: title.into(),
        detail: detail.into(),
        hit: None,
        target,
    };
    let mut pages = vec![page(
        "Meetings",
        "Every summarised meeting",
        Target::Meetings,
    )];
    if f.tasks {
        pages.push(page("Tasks", "To-dos from your meetings", Target::Tasks));
    }
    if f.memory {
        pages.push(page(
            "Memory",
            "What your meetings taught it",
            Target::Memory,
        ));
    }
    if f.daily_check {
        pages.push(page("Diary", "Your days, newest first", Target::Check));
    }
    pages.push(page("Settings", "Hex and this app", Target::Settings(None)));
    if typed {
        // The Settings tabs are listed by name only when the query asks for one of them.
        for section in hex::Section::ALL {
            pages.push(page(
                &format!("Settings \u{b7} {}", section.name()),
                section.blurb(),
                Target::Settings(Some(section)),
            ));
        }
        pages.retain(|e| contains(&e.title, &q) || contains(&e.detail, &q));
    }

    let mut meetings = Vec::new();
    if f.projects && typed {
        if contains("Areas project migration", &q) {
            pages.push(page(
                "Areas",
                "All areas and project migration",
                Target::Areas,
            ));
        }
        for area in s
            .areas
            .catalog
            .areas
            .values()
            .filter(|a| contains(&a.name, &q) || a.aliases.iter().any(|n| contains(n, &q)))
        {
            pages.push(page(&area.name, "Area", Target::Area(area.id.clone())));
        }
        for project in s
            .areas
            .catalog
            .projects
            .values()
            .filter(|p| p.converted_to.is_none() && contains(&p.name, &q))
        {
            pages.push(page(
                &project.name,
                "Project",
                Target::Project(project.name.clone()),
            ));
        }
    }
    if typed {
        for hit in core::search(&s.meetings, query.trim())
            .into_iter()
            .take(MEETING_ROWS)
        {
            if let Some(m) = s.meetings.iter().find(|m| m.id == hit.meeting_id) {
                meetings.push(Entry {
                    section: "Meetings",
                    title: m.title.clone(),
                    detail: meeting_detail(m),
                    hit: Some(hit),
                    target: Target::Meeting(m.id.clone()),
                });
            }
        }
    } else {
        for index in core::group_by_day(&s.meetings)
            .into_iter()
            .flat_map(|g| g.meeting_indices)
            .take(RECENT_ROWS)
        {
            let m = &s.meetings[index];
            meetings.push(Entry {
                section: "Recent meetings",
                title: m.title.clone(),
                detail: meeting_detail(m),
                hit: None,
                target: Target::Meeting(m.id.clone()),
            });
        }
    }

    let mut out = Vec::new();
    if typed {
        out.extend(meetings);
        out.extend(pages);
        out.extend(actions);
    } else {
        out.extend(actions);
        out.extend(pages);
        out.extend(meetings);
    }
    out
}

/// "Mon 3 Nov, Maya, Priya and 1 more".
fn meeting_detail(m: &core::Meeting) -> String {
    let when = m
        .started
        .map(|t| t.format("%a %-d %b").to_string())
        .unwrap_or_default();
    let who = people_line(m);
    if when.is_empty() {
        who
    } else {
        format!("{when} \u{b7} {who}")
    }
}

impl State {
    pub fn open_finder(&mut self) {
        self.finder_query = TextEdit::new("");
        self.finder_query.focused = true;
        self.finder = Some(Finder {
            entries: entries(self, ""),
            cursor: 0,
        });
    }

    pub fn close_finder(&mut self) {
        self.finder = None;
        self.finder_query = TextEdit::new("");
    }

    pub fn toggle_finder(&mut self) {
        if self.finder.is_some() {
            self.close_finder();
        } else {
            self.open_finder();
        }
    }

    pub(super) fn finder_changed(&mut self) {
        let entries = entries(self, &self.finder_query.text);
        if let Some(f) = self.finder.as_mut() {
            f.entries = entries;
            f.cursor = 0;
        }
    }

    fn finder_move(&mut self, delta: isize, tx: &mut Tx) {
        let Some(f) = self.finder.as_mut() else {
            return;
        };
        let last = f.entries.len().saturating_sub(1) as isize;
        f.cursor = (f.cursor as isize + delta).clamp(0, last) as usize;
        tx.scroll_into_view(format!("finder-row-{}", f.cursor));
    }

    /// Does what the row at `index` says, and closes the palette.
    fn finder_run(&mut self, index: usize, tx: &mut Tx) {
        let Some(target) = self
            .finder
            .as_ref()
            .and_then(|f| f.entries.get(index))
            .map(|e| e.target.clone())
        else {
            return;
        };
        self.close_finder();
        match target {
            Target::Meeting(id) => self.select(&id, tx),
            Target::Meetings => self.go_meetings(),
            Target::Tasks => {
                self.set_view(View::Tasks);
                self.nav_done();
            }
            Target::Memory => {
                self.open_memory();
                self.nav_done();
            }
            Target::Check => {
                self.open_check();
                self.nav_done();
            }
            Target::Settings(section) => {
                self.open_hex();
                if let Some(section) = section {
                    self.hex.section = section;
                }
                self.nav_done();
            }
            Target::NewMeeting => self.new_meeting(),
            Target::Ask => self.focus_ask(""),
            Target::Area(id) => self.set_filter(super::nav::Filter::Area(id)),
            Target::Project(name) => self.set_filter(super::nav::Filter::Project(name)),
            Target::Areas => self.set_filter(super::nav::Filter::Areas),
        }
    }

    /// Puts the cursor in the Ask bar with `question` in it. The Ask bar lives on Home and on a
    /// meeting, so from any other page this goes to Home first.
    pub fn focus_ask(&mut self, question: &str) {
        if self.page() != Page::Meetings || self.on_live() {
            self.go_meetings();
        }
        self.question = TextEdit::new(question);
        self.question.focused = true;
    }

    /// Tab: the typed text becomes a question in the Ask bar.
    fn finder_tab(&mut self) {
        if !self.has_agent() {
            return;
        }
        let question = self.finder_query.text.trim().to_string();
        self.close_finder();
        self.focus_ask(&question);
    }
}

/// The keys the palette owns: Up and Down, Enter, Tab and Escape. The box and the window both
/// call this, so the keys work whether or not the box still has focus.
pub fn key(s: &mut State, e: &KeyEvent, tx: &mut Tx) -> bool {
    if s.finder.is_none() {
        return false;
    }
    match e.key {
        Key::Escape => s.close_finder(),
        Key::Up => s.finder_move(-1, tx),
        Key::Down => s.finder_move(1, tx),
        Key::Enter => {
            let at = s.finder.as_ref().map_or(0, |f| f.cursor);
            s.finder_run(at, tx);
        }
        Key::Tab => s.finder_tab(),
        _ => return false,
    }
    true
}

fn result_row(i: usize, e: &Entry, active: bool, p: Palette) -> El {
    // A meeting needs two lines and its match; a page or an action reads on one line.
    let single = !matches!(e.target, Target::Meeting(_));
    let mut text_col = col().grow(1.0).min_w(0.0).gap(1.0);
    if single {
        text_col = text_col.child(
            row()
                .gap(10.0)
                .items_center()
                .child(roles::NAME.line(&e.title, p.ink))
                .child(roles::META.line(&e.detail, p.faint).grow(1.0)),
        );
    } else {
        text_col = text_col
            .child(roles::NAME.line(&e.title, p.ink))
            .child(roles::META.line(&e.detail, p.soft));
    }
    if let Some(hit) = &e.hit {
        text_col = text_col.child(snippet(hit, p));
    }
    let mut r = row()
        .w_full()
        .gap(12.0)
        .items_center()
        .px(12.0)
        .py(if single { 7.0 } else { 8.0 })
        .radius(8.0)
        .child(text_col);
    if active {
        r = r.fill(p.chip).child(keycap("Enter", p));
    }
    pressable(r)
        .key(format!("finder-row-{i}"))
        .label(e.title.clone())
        .hover(p.chip)
        .on_press_tx(move |s: &mut State, tx| s.finder_run(i, tx))
}

fn hint(cap: &str, words: &str, p: Palette) -> El {
    row()
        .gap(6.0)
        .items_center()
        .child(keycap(cap, p))
        .child(roles::meta(words, p))
}

/// The palette over the whole window, or nothing while it is closed.
pub fn overlay(s: &State, k: &Kit, p: Palette, narrow: bool) -> Option<El> {
    let f = s.finder.as_ref()?;
    let input = TextInput::new(&s.finder_query)
        .plain()
        .placeholder("Search or jump to")
        .label("Search meetings, pages and actions")
        .height(52.0)
        .on_edit(|s: &mut State| &mut s.finder_query)
        .on_change(|s: &mut State, _| s.finder_changed())
        .on_key(key)
        .build(k)
        .key("finder-input")
        .px(8.0)
        .w_full();
    let mut list = col().w_full().px(6.0).pb(8.0);
    let mut section = "";
    for (i, e) in f.entries.iter().enumerate() {
        if e.section != section {
            section = e.section;
            list = list.child(
                roles::META
                    .bold()
                    .wrap(section, p.faint)
                    .px(12.0)
                    .pt(12.0)
                    .pb(4.0),
            );
        }
        list = list.child(result_row(i, e, i == f.cursor, p));
    }
    if f.entries.is_empty() {
        list = list.child(
            roles::SMALL
                .wrap(
                    if s.has_agent() {
                        "Nothing matches. Press Tab to ask your agent."
                    } else {
                        "Nothing matches."
                    },
                    p.soft,
                )
                .px(12.0)
                .py(16.0),
        );
    }
    let mut foot = row()
        .w_full()
        .gap(16.0)
        .items_center()
        .px(16.0)
        .py(9.0)
        .child(hint("Enter", "Open", p));
    if s.has_agent() {
        foot = foot.child(hint("Tab", "Ask your agent", p));
    }
    foot = foot.child(hint("Esc", "Close", p));
    let card = col()
        .key("finder-card")
        .w_full()
        .max_w(620.0)
        .radius(14.0)
        .fill(p.card)
        .border(1.0, p.line)
        .shadow(12.0, 40.0, p.shadow)
        .transition(super::ask::rise(10.0))
        .child(input)
        .child(hairline(p))
        .child(
            col()
                .key("finder-scroll")
                .w_full()
                .max_h(if narrow { 360.0 } else { 480.0 })
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(list),
        )
        .child(hairline(p))
        .child(foot);
    Some(
        col()
            .key("finder")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .child(
                el().key("finder-scrim")
                    .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .fill(p.scrim)
                    .on_press(|s: &mut State| s.close_finder()),
            )
            .child(
                col()
                    .pin(
                        Some(0.0),
                        Some(if narrow { 56.0 } else { 84.0 }),
                        Some(0.0),
                        None,
                    )
                    .items_center()
                    .px(if narrow { 10.0 } else { 20.0 })
                    .child(card),
            ),
    )
}
