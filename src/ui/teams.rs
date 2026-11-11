//! Compact manual teams and one guarded inline editor.
use super::{
    State, cards, icons, people::TeamEditor, roles, tasks::button, theme::Palette,
    widgets::hairline,
};
use crate::ui::widgets::ButtonDisabled;
use rune::{El, Key, KeyEvent, TextEdit, Tx, col, row};
use rune_kit::{Kit, TextInput};
use sidenote::core;

pub fn open(s: &mut State, original: Option<core::Team>) {
    if s.edit_busy {
        return;
    }
    let team = original.clone().unwrap_or_default();
    s.tasks.team_name = TextEdit::new(team.name);
    s.tasks.team_members = TextEdit::new(team.members.join(", "));
    s.tasks.team_aliases = TextEdit::new(
        team.aliases
            .iter()
            .map(|(a, m)| format!("{a} = {m}"))
            .collect::<Vec<_>>()
            .join(", "),
    );
    s.tasks.teams_notice = None;
    s.people.team_editor = Some(TeamEditor {
        original,
        advanced: false,
        remove_confirm: false,
        member_search: TextEdit::default(),
    });
    super::scene_scroll(s, "team-editor");
}
pub fn cancel(s: &mut State) {
    if s.edit_busy {
        return;
    }
    s.people.team_editor = None;
    s.tasks.team_name = TextEdit::default();
    s.tasks.team_members = TextEdit::default();
    s.tasks.team_aliases = TextEdit::default();
    s.tasks.teams_notice = None;
}
fn form_key(s: &mut State, e: &KeyEvent, tx: &mut Tx) -> bool {
    if e.key == Key::Escape {
        cancel(s);
        tx.blur();
        true
    } else {
        false
    }
}
fn save(s: &mut State) {
    if s.edit_busy {
        return;
    }
    let Some(editor) = &s.people.team_editor else {
        return;
    };
    let name = s.tasks.team_name.text.trim().to_string();
    if name.is_empty() {
        s.tasks.teams_notice = Some("Enter a team name.".into());
        return;
    }
    if s.tasks.teams.teams.iter().any(|team| {
        core::people::key(&team.name) == core::people::key(&name)
            && editor
                .original
                .as_ref()
                .is_none_or(|old| core::people::key(&old.name) != core::people::key(&team.name))
    }) {
        s.tasks.teams_notice = Some("Use a different name for each team.".into());
        return;
    }
    let members = s
        .tasks
        .team_members
        .text
        .split(',')
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mut aliases = std::collections::BTreeMap::new();
    for pair in s
        .tasks
        .team_aliases
        .text
        .split(',')
        .map(str::trim)
        .filter(|a| !a.is_empty())
    {
        let Some((alias, member)) = pair.split_once('=') else {
            s.tasks.teams_notice = Some("Write each spelling as spelling = member.".into());
            return;
        };
        if alias.trim().is_empty()
            || !members
                .iter()
                .any(|m| m.eq_ignore_ascii_case(member.trim()))
        {
            s.tasks.teams_notice = Some("Each spelling needs a member listed above.".into());
            return;
        }
        aliases.insert(alias.trim().into(), member.trim().into());
    }
    s.publish_edit(super::edit_worker::Job::TeamEdit {
        store: s.tasks.store.clone(),
        original: editor.original.clone(),
        replacement: Some(core::Team {
            name,
            members,
            aliases,
        }),
    });
}
fn editor(s: &State, k: &Kit, p: Palette) -> El {
    let editor = s.people.team_editor.as_ref().unwrap();
    let mut out = col()
        .key("team-editor")
        .w_full()
        .padding(14.0)
        .gap(8.0)
        .child(roles::NAME.wrap(
            if editor.original.is_some() {
                "Edit team"
            } else {
                "Add team"
            },
            p.ink,
        ))
        .child(roles::CONTROL.wrap("Team name", p.soft))
        .child(
            TextInput::new(&s.tasks.team_name)
                .placeholder("Example: Design")
                .on_edit(|s: &mut State| &mut s.tasks.team_name)
                .on_key(form_key)
                .on_submit(|s: &mut State, _| save(s))
                .disabled(s.edit_busy)
                .height(36.0)
                .build(k)
                .key("team-name")
                .w_full(),
        )
        .child(roles::CONTROL.wrap("Members", p.soft));
    let people = core::people::roster(&s.meetings, &s.tasks.teams, &s.tasks.all, &s.people.prefs);
    let mut names = std::collections::BTreeMap::new();
    for name in s.tasks.teams.teams.iter().flat_map(|t| &t.members).chain(
        people
            .iter()
            .filter(|person| person.side == Some(core::people::Side::OurTeam))
            .map(|p| &p.name),
    ) {
        names.insert(core::people::key(name), name.clone());
    }
    if names.len() > 12 {
        out = out.child(
            TextInput::new(&editor.member_search)
                .placeholder("Find a member")
                .leading_icon(&icons::SEARCH)
                .leading_icon_size(16.0)
                .on_edit(|s: &mut State| &mut s.people.team_editor.as_mut().unwrap().member_search)
                .on_key(form_key)
                .height(36.0)
                .build(k)
                .key("team-member-search")
                .w_full(),
        );
    }
    let mut choices = row().key("team-members").w_full().flex_wrap().gap(6.0);
    for name in names.into_values().filter(|n| {
        n.to_lowercase()
            .contains(&editor.member_search.text.to_lowercase())
    }) {
        let selected = s
            .tasks
            .team_members
            .text
            .split(',')
            .any(|m| m.trim().eq_ignore_ascii_case(&name));
        choices = choices.child(
            button(
                &name,
                format!("team-member-{}", core::people::key(&name)),
                false,
                p,
            )
            .border(1.0, if selected { p.accent } else { p.line })
            .fill(if selected { p.sel } else { p.card })
            .role("checkbox")
            .checked(selected)
            .button_disabled(s.edit_busy)
            .on_press(move |s: &mut State| {
                let mut members = s
                    .tasks
                    .team_members
                    .text
                    .split(',')
                    .map(str::trim)
                    .filter(|m| !m.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                if selected {
                    members.retain(|m| !m.eq_ignore_ascii_case(&name));
                } else {
                    members.push(name.clone());
                }
                s.tasks.team_members = TextEdit::new(members.join(", "));
            }),
        );
    }
    out = out.child(choices).child(
        cards::fold_control("Advanced", "team-advanced", editor.advanced, p)
            .button_disabled(s.edit_busy)
            .on_press(|s: &mut State| {
                if let Some(e) = &mut s.people.team_editor {
                    e.advanced = !e.advanced;
                }
            }),
    );
    if editor.advanced {
        out=out.child(roles::CONTROL.wrap("Other names for the same person",p.soft))
            .child(roles::META.wrap("For example, Alex C = Alex Chen. Separate pairs with commas. Each name on the right must be a member.",p.soft))
            .child(TextInput::new(&s.tasks.team_aliases).placeholder("Alex C = Alex Chen").on_edit(|s:&mut State| &mut s.tasks.team_aliases)
                .on_key(form_key).on_submit(|s:&mut State,_|save(s)).disabled(s.edit_busy).height(36.0).build(k).key("team-aliases").w_full());
        if let Some(original) = &editor.original {
            if editor.remove_confirm {
                let original = original.clone();
                out = out
                    .child(roles::SMALL.wrap(
                        "Remove this team and its member list? Meeting summaries and tasks will stay.",
                        p.warn,
                    ))
                    .child(
                        row()
                            .gap(8.0)
                            .child(button("Keep team", "team-keep", true, p).on_press(
                                |s: &mut State| {
                                    if let Some(e) = &mut s.people.team_editor {
                                        e.remove_confirm = false;
                                    }
                                },
                            ))
                            .child(
                                button("Remove team", "team-remove-confirm", false, p)
                                    .button_disabled(s.edit_busy)
                                    .on_press(move |s: &mut State| {
                                        s.publish_edit(super::edit_worker::Job::TeamEdit {
                                            store: s.tasks.store.clone(),
                                            original: Some(original.clone()),
                                            replacement: None,
                                        });
                                    }),
                            ),
                    );
            } else {
                out = out.child(
                    button("Remove team", "team-remove", false, p)
                        .button_disabled(s.edit_busy)
                        .on_press(|s: &mut State| {
                            if let Some(e) = &mut s.people.team_editor {
                                e.remove_confirm = true;
                            }
                        }),
                );
            }
        }
    }
    out.child(
        row()
            .gap(8.0)
            .child(
                button("Save team", "team-save", true, p)
                    .button_disabled(s.edit_busy)
                    .on_press(save),
            )
            .child(
                button("Cancel", "team-cancel", false, p)
                    .button_disabled(s.edit_busy)
                    .on_press(cancel),
            ),
    )
}
pub fn section(s: &State, k: &Kit, p: Palette) -> El {
    let mut out = col()
        .key("settings-teams")
        .fill(super::hex::settings::group_highlight(
            s,
            "settings-teams",
            p,
        ))
        .w_full()
        .mt(12.0)
        .gap(8.0)
        .child(
            row()
                .w_full()
                .items_center()
                .justify_between()
                .child(roles::GROUP.wrap("Teams", p.soft))
                .child(
                    super::tasks::value_button(
                        row()
                            .gap(6.0)
                            .items_center()
                            .child(icons::draw(&icons::PLUS, 16.0, p.soft))
                            .child(roles::CONTROL.wrap("Add team", p.soft)),
                        "team-add",
                        "Add team".into(),
                        p,
                    )
                    .button_disabled(s.edit_busy)
                    .on_press(|s: &mut State| open(s, None)),
                ),
        );
    let mut panel = cards::panel(p);
    if s.tasks.teams.teams.is_empty() {
        panel = panel.child(
            roles::SMALL
                .wrap("No teams yet. Add a team to group tasks by owner.", p.soft)
                .padding(14.0),
        );
    }
    for (i, team) in s.tasks.teams.teams.iter().enumerate() {
        if i > 0 {
            panel = panel.child(hairline(p));
        }
        let original = team.clone();
        let opened = s
            .people
            .team_editor
            .as_ref()
            .and_then(|e| e.original.as_ref())
            .is_some_and(|t| t.name == team.name);
        panel = panel.child(
            cards::row_shell()
                .key(format!("team-row-{i}"))
                .child(cards::icon_tile(&icons::PEOPLE, p))
                .child(cards::copy(
                    roles::NAME.wrap(&team.name, p.ink),
                    &team.members.join(", "),
                    p,
                ))
                .child(
                    super::tasks::value_button(
                        row()
                            .gap(6.0)
                            .items_center()
                            .child(icons::draw(
                                if opened {
                                    &icons::CHEVRON_DOWN
                                } else {
                                    &icons::CHEVRON_RIGHT
                                },
                                16.0,
                                p.soft,
                            ))
                            .child(
                                roles::CONTROL.wrap(if opened { "Close" } else { "Open" }, p.soft),
                            ),
                        &format!("team-edit-{i}"),
                        format!("{} {}", if opened { "Close" } else { "Open" }, team.name),
                        p,
                    )
                    .button_disabled(s.edit_busy)
                    .on_press(move |s: &mut State| {
                        if opened {
                            cancel(s)
                        } else {
                            open(s, Some(original.clone()))
                        }
                    }),
                ),
        );
        if s.people
            .team_editor
            .as_ref()
            .and_then(|e| e.original.as_ref())
            .is_some_and(|t| t.name == team.name)
        {
            panel = panel.child(hairline(p)).child(editor(s, k, p));
        }
    }
    if s.people
        .team_editor
        .as_ref()
        .is_some_and(|e| e.original.is_none())
    {
        panel = panel.child(hairline(p)).child(editor(s, k, p));
    }
    out = out.child(panel);
    if let Some(message) = &s.tasks.teams_notice {
        out = out.child(
            roles::SMALL
                .wrap(
                    message,
                    if message == "Teams saved" {
                        p.ok
                    } else {
                        p.warn
                    },
                )
                .role("status"),
        );
    }
    out
}
