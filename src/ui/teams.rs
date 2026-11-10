//! Teams are independent of the notes settings page.
use super::{State, roles, tasks::button, theme::Palette};
use rune::{El, TextEdit, col, row};
use rune_kit::{Kit, TextInput};
use sidenote::core;

pub fn section(s: &State, k: &Kit, p: Palette) -> El {
    let mut out = col().w_full().max_w(640.0).pt(24.0).gap(14.0)
        .child(roles::SECTION.wrap("Teams",p.ink))
        .child(roles::SMALL.wrap("A task gets the team of the person who owns it. If that person is on no team, the task shows No team.",p.soft));
    for (i, team) in s.tasks.teams.teams.iter().enumerate() {
        let index = i;
        let aliases = team
            .aliases
            .iter()
            .map(|(a, m)| format!("{a} = {m}"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut card = col()
            .w_full()
            .gap(6.0)
            .py(8.0)
            .child(roles::GROUP.wrap(&team.name, p.ink))
            .child(roles::SMALL.wrap(team.members.join(", "), p.soft));
        if !aliases.is_empty() {
            card = card.child(roles::META.wrap(format!("Other names: {aliases}"), p.soft));
        }
        card = card.child(
            row()
                .w_full()
                .justify_end()
                .gap(8.0)
                .child(button("Edit", format!("team-edit-{i}"), false, p).on_press(
                    move |s: &mut State| {
                        let team = &s.tasks.teams.teams[index];
                        s.tasks.team_name = TextEdit::new(&team.name);
                        s.tasks.team_members = TextEdit::new(team.members.join(", "));
                        s.tasks.team_aliases = TextEdit::new(
                            team.aliases
                                .iter()
                                .map(|(a, m)| format!("{a} = {m}"))
                                .collect::<Vec<_>>()
                                .join(", "),
                        );
                    },
                ))
                .child(
                    button("Remove", format!("team-remove-{i}"), false, p).on_press(
                        move |s: &mut State| {
                            let mut teams = s.tasks.teams.clone();
                            teams.teams.remove(index);
                            s.publish_edit(super::edit_worker::Job::Teams {
                                store: s.tasks.store.clone(),
                                teams,
                            });
                        },
                    ),
                ),
        );
        out = out.child(card);
    }
    if s.tasks.teams.teams.is_empty() {
        out = out.child(roles::SMALL.wrap("No teams yet. Add your first team below.", p.faint));
    }
    out = out
        .child(roles::GROUP.wrap("Add or update a team", p.ink))
        .child(roles::META.wrap(
            "Use the same team name to update its members and other names.",
            p.soft,
        ))
        .child(roles::CONTROL.wrap("Team name", p.ink))
        .child(
            TextInput::new(&s.tasks.team_name)
                .placeholder("Example: Design")
                .on_edit(|s: &mut State| &mut s.tasks.team_name)
                .height(36.0)
                .build(k)
                .key("team-name")
                .w_full(),
        )
        .child(roles::CONTROL.wrap("Members, separated by commas", p.ink))
        .child(
            TextInput::new(&s.tasks.team_members)
                .placeholder("Alex Chen, Morgan Lee")
                .on_edit(|s: &mut State| &mut s.tasks.team_members)
                .height(36.0)
                .build(k)
                .key("team-members")
                .w_full(),
        )
        .child(roles::CONTROL.wrap("Other names for the same person", p.ink))
        .child(roles::META.wrap(
            "Example: Alex C is Alex Chen. Enter Alex C = Alex Chen below.",
            p.soft,
        ))
        .child(
            TextInput::new(&s.tasks.team_aliases)
                .placeholder("Alex C = Alex Chen, M Lee = Morgan Lee")
                .on_edit(|s: &mut State| &mut s.tasks.team_aliases)
                .height(36.0)
                .build(k)
                .key("team-aliases")
                .w_full(),
        )
        .child(
            button("Save team", "team-save", true, p).on_press(|s: &mut State| {
                let name = s.tasks.team_name.text.trim().to_string();
                if name.is_empty() {
                    s.tasks.teams_notice = Some("Enter a team name".into());
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
                    let Some((a, m)) = pair.split_once('=') else {
                        s.tasks.teams_notice =
                            Some("Write each spelling as spelling = member".into());
                        return;
                    };
                    if a.trim().is_empty()
                        || !members
                            .iter()
                            .any(|member| member.eq_ignore_ascii_case(m.trim()))
                    {
                        s.tasks.teams_notice =
                            Some("Each spelling needs a member listed above".into());
                        return;
                    }
                    aliases.insert(a.trim().into(), m.trim().into());
                }
                let mut teams = s.tasks.teams.clone();
                let team = core::Team {
                    name: name.clone(),
                    members,
                    aliases,
                };
                if let Some(t) = teams
                    .teams
                    .iter_mut()
                    .find(|t| t.name.eq_ignore_ascii_case(&name))
                {
                    *t = team;
                } else {
                    teams.teams.push(team);
                }
                s.publish_edit(super::edit_worker::Job::Teams {
                    store: s.tasks.store.clone(),
                    teams,
                });
            }),
        );
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
