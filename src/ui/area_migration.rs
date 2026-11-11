//! One virtual row per project; exceptions have their own virtual viewport.
use super::{
    State, area, roles,
    theme::{self, Palette},
};
use crate::ui::widgets::ButtonDisabled;
use rune::{El, GrowthSpec, ScrollSpec, col, row};
use rune_kit::{Kit, Menu, MenuItem, Placement, TextInput};
use sidenote::core::project_merge::normalized_name;

fn changed(s: &mut State) {
    if let Some(e) = &mut s.areas.editor {
        e.reconcile();
    }
    s.areas.preview = None;
}
fn project_search(s: &mut State) {
    let names = &s.areas.catalog.projects;
    if let Some(e) = &mut s.areas.editor {
        let query = e.search.text.to_lowercase();
        e.visible_projects = e
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                names
                    .get(&r.project_id)
                    .is_some_and(|p| p.name.to_lowercase().contains(&query))
                    || r.area.text.to_lowercase().contains(&query)
            })
            .map(|(i, _)| i)
            .collect();
    }
}
fn item_search(s: &mut State, index: usize) {
    if let Some(e) = &mut s.areas.editor {
        let r = &mut e.rows[index];
        let query = r.search.text.to_lowercase();
        r.visible_items = e
            .inventory
            .by_project
            .get(&r.project_id)
            .into_iter()
            .flatten()
            .copied()
            .filter(|i| {
                e.inventory.references[*i]
                    .label
                    .to_lowercase()
                    .contains(&query)
            })
            .collect();
    }
}
fn picker(s: &State, label: &str, key: &str, p: Palette) -> Menu {
    let owned = key.to_string();
    Menu::new(key)
        .trigger(
            area::button(label, key, p)
                .button_disabled(s.areas.pending.is_some())
                .on_press(move |s: &mut State| {
                    if let Some(e) = &mut s.areas.editor {
                        e.picker = if e.picker.as_deref() == Some(&owned) {
                            None
                        } else {
                            Some(owned.clone())
                        };
                    }
                }),
        )
        .open(
            s.areas
                .editor
                .as_ref()
                .is_some_and(|e| e.picker.as_deref() == Some(key)),
        )
        .placement(Placement::BottomStart)
        .on_close(|s: &mut State| {
            if let Some(e) = &mut s.areas.editor {
                e.picker = None;
            }
        })
}
fn item(s: &State, index: usize, position: usize, p: Palette) -> El {
    let Some(e) = s.areas.editor.as_ref() else {
        return col();
    };
    let Some(r) = e.rows.get(index) else {
        return col();
    };
    let Some(reference) = r
        .visible_items
        .get(position)
        .and_then(|i| e.inventory.references.get(*i))
    else {
        return col();
    };
    let chosen = r
        .assignments
        .get(&reference.key)
        .cloned()
        .unwrap_or_else(|| (!r.convert).then(|| r.project_id.clone()));
    let label = match chosen.as_ref() {
        Some(id) => s
            .areas
            .catalog
            .projects
            .get(id)
            .map(|p| p.name.as_str())
            .unwrap_or("Missing project"),
        None => "Area-only work",
    };
    let key = format!("migration-item-{}", reference.key);
    let mut menu = picker(s, label, &key, p);
    let mut choices = vec![(None, "Area-only work".to_string())];
    for candidate in &e.rows {
        if !candidate.convert
            && normalized_name(&candidate.area.text) == normalized_name(&r.area.text)
            && let Some(project) = s.areas.catalog.projects.get(&candidate.project_id)
        {
            choices.push((Some(project.id.clone()), project.name.clone()));
        }
    }
    for (target, label) in choices {
        let key = reference.key.clone();
        menu = menu.item(MenuItem::new(label).checked(chosen == target).on_select(
            move |s: &mut State| {
                if let Some(e) = &mut s.areas.editor {
                    e.rows[index]
                        .assignments
                        .insert(key.clone(), target.clone());
                    e.picker = None;
                }
                changed(s);
            },
        ));
    }
    let mut layer = Vec::new();
    let menu = menu.build(&theme::kit(&p), &mut layer);
    col()
        .w_full()
        .h(88.0)
        .gap(4.0)
        .py(6.0)
        .child(
            roles::META
                .line(format!("{} · {}", reference.kind, reference.label), p.soft)
                .help(&reference.label),
        )
        .child(menu.self_start())
        .children(layer)
}
fn project(s: &State, index: usize, p: Palette, narrow: bool) -> El {
    let Some(e) = s.areas.editor.as_ref() else {
        return col();
    };
    let Some(r) = e.rows.get(index) else {
        return col();
    };
    let name = s
        .areas
        .catalog
        .projects
        .get(&r.project_id)
        .map(|p| p.name.as_str())
        .unwrap_or("Missing project");
    let items = e
        .inventory
        .by_project
        .get(&r.project_id)
        .map_or(0, Vec::len);
    let counts = e
        .inventory
        .project_counts
        .get(&r.project_id)
        .copied()
        .unwrap_or_default();
    let words = col()
        .gap(4.0)
        .child(roles::NAME.line(name, p.ink))
        .child(roles::META.wrap(
            format!(
                "{} meeting{} · {} tasks · {} memory · {} proposals",
                counts[0],
                if counts[0] == 1 { "" } else { "s" },
                counts[1],
                counts[2],
                counts[3]
            ),
            p.faint,
        ));
    let input = TextInput::new(&r.area)
        .plain()
        .placeholder("Choose or enter area")
        .on_edit(move |s: &mut State| {
            match s.areas.editor.as_mut().and_then(|e| e.rows.get_mut(index)) {
                Some(r) => &mut r.area,
                None => &mut s.areas.input_sink,
            }
        })
        .on_change(|s: &mut State, _| changed(s))
        .height(34.0)
        .build(&theme::kit(&p))
        .key(format!("migration-area-{index}"))
        .label(format!("Area for {name}"))
        .w_full()
        .min_w(0.0)
        .border(1.0, p.line)
        .radius(6.0)
        .disabled(s.areas.pending.is_some());
    let key = format!("migration-default-{index}");
    let mut menu = picker(
        s,
        &format!(
            "All {items} items: {}",
            if r.convert {
                "area-only work"
            } else {
                "keep in project"
            }
        ),
        &key,
        p,
    );
    for convert in [false, true] {
        menu = menu.item(
            MenuItem::new(if convert {
                "Area-only work (turn project into area)"
            } else {
                "Keep in project"
            })
            .checked(r.convert == convert)
            .on_select(move |s: &mut State| {
                if let Some(e) = &mut s.areas.editor {
                    e.rows[index].convert = convert;
                    e.picker = None;
                }
                changed(s);
            }),
        );
    }
    let mut layer = Vec::new();
    let menu = menu.build(&theme::kit(&p), &mut layer);
    let exceptions = area::button(
        if r.expanded {
            "Hide item choices"
        } else {
            "Choose per item"
        },
        &format!("migration-expand-{index}"),
        p,
    )
    .button_disabled(s.areas.pending.is_some())
    .on_press(move |s: &mut State| {
        if let Some(e) = &mut s.areas.editor {
            e.rows[index].expanded = !e.rows[index].expanded;
        }
    });
    let controls = row().wrap().gap(6.0).child(menu).child(exceptions);
    let body = if narrow {
        col()
            .w_full()
            .gap(6.0)
            .child(words)
            .child(input)
            .child(controls)
    } else {
        row()
            .w_full()
            .gap(12.0)
            .items_center()
            .child(words.grow(1.0).min_w(180.0))
            .child(input.w(180.0))
            .child(controls)
    };
    let mut content = col()
        .w_full()
        .gap(6.0)
        .px(4.0)
        .py(12.0)
        .child(body)
        .children(layer);
    if r.expanded {
        content = content
            .child(
                TextInput::new(&r.search)
                    .plain()
                    .placeholder("Search these items")
                    .on_edit(move |s: &mut State| {
                        match s.areas.editor.as_mut().and_then(|e| e.rows.get_mut(index)) {
                            Some(r) => &mut r.search,
                            None => &mut s.areas.input_sink,
                        }
                    })
                    .on_change(move |s: &mut State, _| item_search(s, index))
                    .height(34.0)
                    .build(&theme::kit(&p))
                    .key(format!("migration-item-search-{index}"))
                    .w_full()
                    .border(1.0, p.line)
                    .radius(6.0),
            )
            .children(
                r.visible_items
                    .is_empty()
                    .then(|| roles::SMALL.wrap("No items match this search.", p.soft)),
            )
            .child(
                col()
                    .key(format!("migration-exceptions-{index}"))
                    .w_full()
                    .h(if narrow { 280.0 } else { 260.0 })
                    .min_h(0.0)
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .child(
                        col()
                            .key(format!("migration-exception-rows-{index}"))
                            .growth(GrowthSpec::default())
                            .virtual_rows(
                                r.visible_items.len(),
                                88.0,
                                move |s: &State, i| {
                                    s.areas
                                        .editor
                                        .as_ref()
                                        .and_then(|e| {
                                            e.rows
                                                .get(index)
                                                .and_then(|r| r.visible_items.get(i))
                                                .and_then(|j| e.inventory.references.get(*j))
                                        })
                                        .map(|r| r.key.as_str())
                                        .unwrap_or("missing-item")
                                        .into()
                                },
                                move |s: &State, i| item(s, index, i, p),
                            ),
                    ),
            );
    }
    content
}
pub fn pane(s: &State, k: &Kit, p: Palette, narrow: bool) -> El {
    let Some(e) = s.areas.editor.as_ref() else {
        return col();
    };
    let busy = s.areas.pending.is_some();
    let tasks = e.inventory.counts[1];
    let header = col()
        .w_full()
        .gap(8.0)
        .px(if narrow { 16.0 } else { 32.0 })
        .py(14.0)
        .child(roles::SECTION.wrap("Review project migration", p.ink))
        .child(roles::SMALL.wrap(
            format!(
                "{} projects · {tasks} tasks. Nothing changes until you confirm.",
                e.rows.len()
            ),
            p.soft,
        ))
        .child(roles::META.wrap(
            "Set each area and its default. Choose per item only for exceptions.",
            p.faint,
        ))
        .child(
            TextInput::new(&e.search)
                .plain()
                .placeholder("Find a project or area")
                .on_edit(|s: &mut State| match s.areas.editor.as_mut() {
                    Some(e) => &mut e.search,
                    None => &mut s.areas.input_sink,
                })
                .on_change(|s: &mut State, _| project_search(s))
                .height(34.0)
                .build(k)
                .key("migration-project-search")
                .w_full()
                .border(1.0, p.line)
                .radius(6.0),
        );
    let mut rows = col()
        .key("migration-project-rows")
        .w_full()
        .growth(GrowthSpec::default())
        .virtual_rows(
            e.visible_projects.len(),
            if narrow { 154.0 } else { 140.0 },
            |s: &State, i| {
                s.areas
                    .editor
                    .as_ref()
                    .and_then(|e| e.visible_projects.get(i).and_then(|j| e.rows.get(*j)))
                    .map(|r| r.project_id.as_str())
                    .unwrap_or("missing-project")
                    .into()
            },
            move |s: &State, i| {
                let Some(index) = s
                    .areas
                    .editor
                    .as_ref()
                    .and_then(|e| e.visible_projects.get(i))
                    .copied()
                else {
                    return col();
                };
                project(s, index, p, narrow)
            },
        );
    if e.visible_projects.is_empty() {
        rows = roles::SMALL
            .wrap("No projects match this search.", p.soft)
            .key("migration-empty-search");
    }
    let mut footer = col()
        .w_full()
        .gap(8.0)
        .px(if narrow { 16.0 } else { 32.0 })
        .py(12.0)
        .fill(p.card)
        .border(1.0, p.line);
    if let Some(preview) = &s.areas.preview {
        footer = footer.child(roles::GROUP.wrap(
            format!(
                "Reviewed {} projects · {} linked items · one backup on confirm",
                preview.project_count,
                preview.references.len()
            ),
            p.ink,
        ));
    }
    if let Some(notice) = &e.notice {
        footer = footer.child(roles::SMALL.wrap(notice, p.warn));
    }
    footer = footer.child(area::notices(s, p)).child(
        row()
            .wrap()
            .gap(8.0)
            .child(
                super::tasks::button(
                    if s.areas.preview.is_some() {
                        "Confirm migration"
                    } else {
                        "Review mapping"
                    },
                    "migration-review-confirm",
                    true,
                    p,
                )
                .button_disabled(busy)
                .on_press(|s: &mut State| {
                    if s.areas.preview.is_some() {
                        s.confirm_area();
                    } else {
                        s.preview_area();
                    }
                }),
            )
            .child(
                area::button("Cancel", "migration-cancel", p)
                    .button_disabled(busy)
                    .on_press(|s: &mut State| {
                        s.cancel_area();
                    }),
            ),
    );
    col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .min_w(0.0)
        .fill(p.bg)
        .child(header)
        .child(
            col()
                .key("migration-project-scroll")
                .grow(1.0)
                .w_full()
                .min_h(0.0)
                .px(if narrow { 12.0 } else { 28.0 })
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(rows),
        )
        .child(footer)
}
