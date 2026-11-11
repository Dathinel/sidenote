//! People is a projection of the meeting roster, with device-local side corrections.
use super::{State, cards, icons, nav, roles, teams, theme::Palette, widgets::hairline};
use rune::{El, TextEdit, col, row};
use rune_kit::{Kit, SegmentedControl, TextInput};
use sidenote::core::{
    self,
    people::{Person, Side},
};
use std::path::Path;

pub struct TeamEditor {
    pub original: Option<core::Team>,
    pub advanced: bool,
    pub remove_confirm: bool,
    pub member_search: TextEdit,
}
pub struct PeopleUi {
    pub tab: usize,
    pub search: TextEdit,
    pub prefs: core::UiPrefs,
    pub team_editor: Option<TeamEditor>,
    pub notice: Option<String>,
}
impl PeopleUi {
    pub fn load(root: &Path, meetings: &[core::Meeting], prefs: &mut core::UiPrefs) -> Self {
        let notice = if core::people::initialize(prefs, meetings) && prefs.save(root).is_err() {
            prefs.people_existing_meetings = None;
            Some("Could not remember the existing meetings. People will stay hidden until this can be saved.".into())
        } else {
            None
        };
        Self {
            tab: prefs
                .settings_tabs
                .get("People")
                .copied()
                .unwrap_or(0)
                .min(2),
            search: TextEdit::default(),
            prefs: prefs.clone(),
            team_editor: None,
            notice,
        }
    }
}

impl State {
    fn set_person_side(&mut self, person: &str, side: Side) {
        let id = core::people::key(person);
        let mut prefs = core::UiPrefs::load(&self.store.root);
        prefs.people_sides.insert(id, side);
        match prefs.save(&self.store.root) {
            Ok(()) => {
                self.people.prefs.people_sides = prefs.people_sides;
                self.people.notice = None;
            }
            Err(_) => {
                self.people.notice =
                    Some("Could not save this person's side. Your previous choice is kept.".into())
            }
        }
    }
}
fn side_control(person: &Person, k: &Kit, p: Palette) -> El {
    let mut selected_kit = k.clone();
    selected_kit.c.bg_base = p.sel;
    selected_kit.c.border_strong = p.accent;
    let name = person.name.clone();
    SegmentedControl::new(
        &["Our team", "External"],
        person.side.map(|s| usize::from(s == Side::External)),
    )
    .label(format!("Side for {}", person.name))
    .width(Some(168.0))
    .on_pick::<State>(move |s, index, _| {
        s.set_person_side(
            &name,
            if index == 0 {
                Side::OurTeam
            } else {
                Side::External
            },
        )
    })
    .build(&selected_kit)
    .key(format!("person-side-{}", core::people::key(&person.name)))
}
fn avatar(name: &str, p: Palette) -> El {
    let initials = name
        .split_whitespace()
        .take(2)
        .filter_map(|n| n.chars().next())
        .flat_map(char::to_uppercase)
        .collect::<String>();
    col()
        .size(34.0, 34.0)
        .center()
        .radius(8.0)
        .fill(p.chip)
        .child(roles::NAME.line(initials, p.soft))
}
fn base_person_row(person: &Person, affiliation: &str, k: &Kit, compact: bool, p: Palette) -> El {
    let id = core::people::key(&person.name);
    let name = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(3.0)
        .child(
            roles::NAME
                .wrap(&person.name, p.ink)
                .max_lines(2)
                .help(&person.name),
        )
        .children(
            (!person.aliases.is_empty())
                .then(|| roles::META.line(format!("also {}", person.aliases.join(", ")), p.soft)),
        )
        .children(
            person
                .side
                .is_none()
                .then(|| roles::META.line("Side not set", p.faint)),
        );
    let identity = row()
        .min_w(190.0)
        .grow(1.0)
        .gap(12.0)
        .items_center()
        .child(avatar(&person.name, p))
        .child(name);
    let date = person
        .last_seen
        .map(|d| d.format("%-d %b").to_string())
        .unwrap_or_else(|| "Unknown".into());
    if compact {
        cards::row_shell()
            .key(format!("person-{id}"))
            .flex_wrap()
            .child(identity)
            .child(side_control(person, k, p))
            .child(
                roles::META
                    .wrap(
                        format!(
                            "{} \u{b7} {} meetings \u{b7} {} \u{b7} {} open",
                            affiliation, person.meetings, date, person.open
                        ),
                        p.soft,
                    )
                    .w_full()
                    .pl(46.0),
            )
    } else {
        cards::row_shell()
            .key(format!("person-{id}"))
            .child(identity)
            .child(roles::SMALL.line(affiliation, p.soft).w(100.0))
            .child(
                roles::META
                    .line(person.meetings.to_string(), p.soft)
                    .w(52.0),
            )
            .child(roles::META.line(date, p.soft).w(65.0))
            .child(roles::META.line(person.open.to_string(), p.soft).w(40.0))
            .child(side_control(person, k, p))
    }
}
pub fn heading(compact: bool, external: bool, p: Palette) -> El {
    if compact {
        return col();
    }
    row()
        .w_full()
        .px(14.0)
        .gap(12.0)
        .mb(8.0)
        .child(roles::META.line("Person", p.faint).grow(1.0).min_w(190.0))
        .child(
            roles::META
                .line(if external { "Company" } else { "Team" }, p.faint)
                .w(100.0),
        )
        .child(roles::META.line("Meetings", p.faint).w(52.0))
        .child(roles::META.line("Last seen", p.faint).w(65.0))
        .child(roles::META.line("Open", p.faint).w(40.0))
        .child(roles::META.line("Side", p.faint).w(168.0))
}
pub fn person_row(s: &State, person: &Person, k: &Kit, compact: bool, p: Palette) -> El {
    let name = person.name.clone();
    let company = s.companies.data.company_for(&person.name);
    let mut out = col().w_full().child(base_person_row(
        person,
        company.map_or(person.team.as_str(), |c| c.name.as_str()),
        k,
        compact,
        p,
    ));
    let mut facts = row()
        .w_full()
        .flex_wrap()
        .items_center()
        .gap(10.0)
        .px(14.0)
        .pb(10.0)
        .pl(60.0);
    if let Some(c) = company {
        let id = c.id.clone();
        facts = facts.child(
            super::widgets::link(&c.name, p)
                .key(format!("person-company-link-{}", core::people::key(&name)))
                .on_press(move |s: &mut State| s.open_company(Some(id.clone()))),
        );
    }
    facts = facts.child(
        super::widgets::link("Company and email", p)
            .key(format!("person-edit-{}", core::people::key(&name)))
            .disabled(s.edit_busy)
            .on_press(move |s: &mut State| s.edit_person_company(&name)),
    );
    if let Some(email) = s
        .companies
        .data
        .catalog
        .people
        .get(&core::people::key(&person.name))
        .and_then(|l| l.email.clone())
    {
        facts = facts.child(
            super::widgets::link(&email, p)
                .key(format!(
                    "person-email-copy-{}",
                    core::people::key(&person.name)
                ))
                .label(format!("Copy email for {}", person.name))
                .on_press_tx(move |_: &mut State, tx| tx.copy(email.clone())),
        );
    }
    out = out.child(facts);
    if s.companies.person.as_deref() == Some(&person.name) {
        out = out.child(super::companies::person_editor(s, k, p));
    }
    out
}
pub fn section(s: &State, k: &Kit, p: Palette, tab: usize) -> El {
    if tab == 2 {
        return col().key("people-import").w_full().mt(18.0).child(cards::panel(p).child(cards::row_shell()
            .child(cards::icon_tile(&icons::FILE_TEXT,p))
            .child(cards::copy(roles::NAME.wrap("People import is not available yet",p.ink),
                "People come from new meeting summaries. You can add and edit teams on Our team.",p))));
    }
    let compact = s.app_w.get() - nav::width(s) < 900.0;
    let query = s.people.search.text.to_lowercase();
    let people = core::people::roster(&s.meetings, &s.tasks.teams, &s.tasks.all, &s.people.prefs);
    let filtered: Vec<_> = people
        .iter()
        .filter(|person| {
            let on_team = person.side == Some(Side::OurTeam);
            on_team == (tab == 0)
                && (query.is_empty()
                    || format!(
                        "{} {} {} {}",
                        person.name,
                        person.aliases.join(" "),
                        person.team,
                        s.companies
                            .data
                            .company_for(&person.name)
                            .map(|c| format!("{} {}", c.name, c.aliases.join(" ")))
                            .unwrap_or_default()
                    )
                    .to_lowercase()
                    .contains(&query))
        })
        .collect();
    let mut out = col()
        .key("people-roster")
        .w_full()
        .gap(12.0)
        .mt(18.0)
        .child(
            roles::META
                .wrap(
                    "People from older meetings appear when you meet them again.",
                    p.soft,
                )
                .key("people-history-note"),
        )
        .child(
            TextInput::new(&s.people.search)
                .leading_icon(&icons::SEARCH)
                .leading_icon_size(16.0)
                .plain()
                .placeholder("Find a person")
                .on_edit(|s: &mut State| &mut s.people.search)
                .on_key(|s: &mut State, e, tx| {
                    if e.key == rune::Key::Escape {
                        s.people.search = TextEdit::default();
                        tx.blur();
                        true
                    } else {
                        false
                    }
                })
                .height(36.0)
                .build(k)
                .key("people-search")
                .border(1.0, p.line)
                .radius(8.0)
                .fill(p.card)
                .w_full(),
        );
    if let Some(message) = &s.people.notice {
        out = out.child(roles::SMALL.wrap(message, p.warn).role("alert"));
    }
    out = out.child(
        roles::GROUP
            .wrap(
                if tab == 0 {
                    "Our team"
                } else {
                    "External and unassigned"
                },
                p.soft,
            )
            .mt(6.0),
    );
    if tab == 1 {
        out = out.child(
            super::widgets::link("Companies", p)
                .key("people-companies")
                .on_press(|s: &mut State| s.open_company(None)),
        );
    }
    if let Some(message) = &s.companies.notice {
        out = out.child(roles::SMALL.wrap(message, p.warn).role("alert"));
    }
    if filtered.is_empty() {
        let text = if !query.is_empty() {
            "No people match this search."
        } else if tab == 0 {
            "People show up here after your first new meetings."
        } else {
            "No external people from new meetings yet."
        };
        out = out.child(cards::panel(p).child(roles::SMALL.wrap(text, p.soft).padding(14.0)));
    } else {
        let mut groups = std::collections::BTreeMap::<Option<String>, Vec<&Person>>::new();
        for person in &filtered {
            let company = if tab == 1 {
                s.companies
                    .data
                    .company_for(&person.name)
                    .map(|c| c.id.clone())
            } else {
                None
            };
            groups.entry(company).or_default().push(person);
        }
        for (company, group) in groups {
            if tab == 1 {
                if let Some(c) = company
                    .as_ref()
                    .and_then(|id| s.companies.data.catalog.records.get(id))
                {
                    let id = c.id.clone();
                    out = out.child(
                        super::widgets::pressable(
                            row()
                                .w_full()
                                .items_center()
                                .gap(8.0)
                                .py(8.0)
                                .child(icons::draw(&icons::BUILDING_2, 18.0, p.soft))
                                .child(
                                    roles::NAME
                                        .wrap(
                                            format!("{} \u{b7} {} people", c.name, group.len()),
                                            p.ink,
                                        )
                                        .max_lines(2),
                                ),
                        )
                        .key(format!("company-header-{}", c.id))
                        .label(format!("Open company {}", c.name))
                        .on_press(move |s: &mut State| s.open_company(Some(id.clone()))),
                    );
                } else {
                    out = out.child(roles::NAME.wrap("No company assigned", p.soft));
                }
            }
            let mut panel = cards::panel(p).key(format!(
                "people-card-{}",
                company.as_deref().unwrap_or("none")
            ));
            for (i, person) in group.iter().enumerate() {
                if i > 0 {
                    panel = panel.child(hairline(p));
                }
                panel = panel.child(person_row(s, person, k, compact, p));
            }
            out = out.child(
                col()
                    .w_full()
                    .child(heading(compact, tab == 1, p))
                    .child(panel),
            );
        }
    }

    if tab == 0 {
        out = out.child(teams::section(s, k, p));
    }
    out
}
