use super::{State, area, roles, theme::Palette};
use rune::{El, TextEdit, col};
use rune_kit::{Kit, Menu, MenuItem, Placement, TextInput};
use sidenote::core::{
    area_ops::{Action, MigrationRow, Reference},
    areas::{Catalog, Target},
};

pub struct RowInput {
    pub project_id: String,
    pub area: TextEdit,
    pub convert: bool,
    pub children: std::collections::BTreeMap<String, String>,
}
pub struct Editor {
    pub action: Action,
    pub name: TextEdit,
    pub rows: Vec<RowInput>,
    pub references: Vec<Reference>,
    pub picker: Option<String>,
}
impl Editor {
    pub fn new(action: Action) -> Self {
        let name = match &action {
            Action::Create { name } | Action::Rename { name, .. } => name.as_str(),
            Action::Convert { area_name, .. } => area_name,
            _ => "",
        };
        let rows = match &action {
            Action::Migrate { rows } => rows
                .iter()
                .map(|r| RowInput {
                    project_id: r.project_id.clone(),
                    area: TextEdit::new(&r.area_name),
                    convert: r.convert,
                    children: r.children.clone(),
                })
                .collect(),
            _ => Vec::new(),
        };
        Self {
            name: TextEdit::new(name),
            action,
            rows,
            references: Vec::new(),
            picker: None,
        }
    }
    pub fn action(&self) -> Action {
        match &self.action {
            Action::Create { .. } => Action::Create {
                name: self.name.text.clone(),
            },
            Action::Rename { area_id, .. } => Action::Rename {
                area_id: area_id.clone(),
                name: self.name.text.clone(),
            },
            Action::Convert {
                project_id,
                children,
                ..
            } => Action::Convert {
                project_id: project_id.clone(),
                area_name: self.name.text.clone(),
                children: children.clone(),
            },
            Action::Migrate { .. } => Action::Migrate {
                rows: self
                    .rows
                    .iter()
                    .map(|r| MigrationRow {
                        project_id: r.project_id.clone(),
                        area_name: r.area.text.clone(),
                        convert: r.convert,
                        children: r.children.clone(),
                    })
                    .collect(),
            },
            other => other.clone(),
        }
    }
}
pub fn title(action: &Action) -> &'static str {
    match action {
        Action::Create { .. } => "Create area",
        Action::Rename { .. } => "Rename area",
        Action::Move { .. } => "Move project into area",
        Action::Convert { .. } => "Turn project into area",
        Action::Migrate { .. } => "Review project migration",
        Action::Assign { .. } => "Assign area-only work",
    }
}
pub fn migration(c: &Catalog) -> Action {
    Action::Migrate {
        rows: c
            .projects
            .values()
            .filter(|p| p.converted_to.is_none())
            .map(|p| MigrationRow {
                project_id: p.id.clone(),
                area_name: p
                    .area_id
                    .as_ref()
                    .and_then(|id| c.areas.get(id))
                    .map(|a| a.name.clone())
                    .unwrap_or_default(),
                convert: false,
                children: Default::default(),
            })
            .collect(),
    }
}
fn name_input(s: &State, k: &Kit, p: Palette) -> El {
    let editor = s.areas.editor.as_ref().unwrap();
    TextInput::new(&editor.name)
        .plain()
        .placeholder("Area name")
        .on_edit(|s: &mut State| &mut s.areas.editor.as_mut().unwrap().name)
        .on_change(|s: &mut State, _| s.areas.preview = None)
        .height(38.0)
        .build(k)
        .key("area-name")
        .label("Area name")
        .w_full()
        .min_w(0.0)
        .border(1.0, p.line)
        .radius(6.0)
        .fill(p.card)
        .disabled(s.areas.pending.is_some())
}
fn choice_menu(
    s: &State,
    k: &Kit,
    p: Palette,
    layer: &mut Vec<El>,
    key: &str,
    title: &str,
) -> Menu {
    let is_open = s
        .areas
        .editor
        .as_ref()
        .is_some_and(|e| e.picker.as_deref() == Some(key));
    let key_owned = key.to_string();
    let _ = (k, layer);
    Menu::new(format!("area-pick-{key}"))
        .trigger(
            area::button(title, &format!("area-pick-{key}"), p)
                .disabled(s.areas.pending.is_some())
                .on_press(move |s: &mut State| {
                    if let Some(e) = &mut s.areas.editor {
                        e.picker = if e.picker.as_deref() == Some(&key_owned) {
                            None
                        } else {
                            Some(key_owned.clone())
                        };
                    }
                }),
        )
        .open(is_open)
        .placement(Placement::BottomStart)
        .on_close(|s: &mut State| {
            if let Some(e) = &mut s.areas.editor {
                e.picker = None;
            }
        })
}
fn child_choices(s: &State, project: &str, area_name: &str) -> Vec<(String, String)> {
    if let Some(e) = &s.areas.editor
        && matches!(e.action, Action::Migrate { .. })
    {
        return e
            .rows
            .iter()
            .filter(|r| {
                r.project_id != project
                    && !r.convert
                    && sidenote::core::project_merge::normalized_name(&r.area.text)
                        == sidenote::core::project_merge::normalized_name(area_name)
            })
            .filter_map(|r| {
                s.areas
                    .catalog
                    .projects
                    .get(&r.project_id)
                    .map(|p| (p.id.clone(), p.name.clone()))
            })
            .collect();
    }
    let matched = s.areas.catalog.area_matches(area_name);
    let area = matched.first().map(|a| a.id.as_str());
    s.areas
        .catalog
        .projects
        .values()
        .filter(|p| {
            p.id != project
                && p.converted_to.is_none()
                && p.area_id.as_deref() == area
                && area.is_some()
        })
        .map(|p| (p.id.clone(), p.name.clone()))
        .collect()
}
fn mapping(
    s: &State,
    k: &Kit,
    p: Palette,
    layer: &mut Vec<El>,
    project: &str,
    area_name: &str,
    row_index: Option<usize>,
) -> El {
    let e = s.areas.editor.as_ref().unwrap();
    let mut content = col().w_full().gap(8.0);
    let choices = child_choices(s, project, area_name);
    for r in e
        .references
        .iter()
        .filter(|r| r.project_id.as_deref() == Some(project))
    {
        let children = match (&e.action, row_index) {
            (_, Some(i)) => &e.rows[i].children,
            (Action::Convert { children, .. }, None) => children,
            _ => continue,
        };
        let selected = children.get(&r.key);
        let label = selected
            .and_then(|id| s.areas.catalog.projects.get(id))
            .map(|p| p.name.as_str())
            .unwrap_or("Area-only work");
        let mut menu = choice_menu(s, k, p, layer, &r.key, label);
        for (id, name) in
            std::iter::once((String::new(), "Area-only work".into())).chain(choices.iter().cloned())
        {
            let key = r.key.clone();
            menu = menu.item(
                MenuItem::new(name)
                    .checked(selected.map(String::as_str).unwrap_or("") == id)
                    .on_select(move |s: &mut State| {
                        if let Some(e) = &mut s.areas.editor {
                            let children = match (&mut e.action, row_index) {
                                (_, Some(i)) => &mut e.rows[i].children,
                                (Action::Convert { children, .. }, None) => children,
                                _ => return,
                            };
                            if id.is_empty() {
                                children.remove(&key);
                            } else {
                                children.insert(key.clone(), id.clone());
                            }
                            e.picker = None;
                            s.areas.preview = None;
                        }
                    }),
            );
        }
        content = content.child(
            col()
                .w_full()
                .gap(4.0)
                .child(roles::META.wrap(format!("{} · {}", r.kind, r.label), p.soft))
                .child(menu.build(k, layer).self_start()),
        );
    }
    content
}
pub fn panel(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> Option<El> {
    let e = s.areas.editor.as_ref()?;
    let busy = s.areas.pending.is_some();
    let mut card = col()
        .w_full()
        .gap(12.0)
        .px(16.0)
        .py(16.0)
        .radius(8.0)
        .border(1.0, p.line)
        .fill(p.card)
        .child(roles::TITLE.wrap(title(&e.action), p.ink));
    match &e.action {
        Action::Create { .. } | Action::Rename { .. } | Action::Convert { .. } => {
            card=card.child(roles::SMALL.wrap(if matches!(e.action,Action::Convert{..}) {"Choose an area name. Review every reference as area-only work or choose a child project below."}else{"Areas hold ongoing work. They do not have a finish date."},p.soft)).child(name_input(s,k,p));
            if let Action::Convert { project_id, .. } = &e.action {
                card = card.child(mapping(s, k, p, layer, project_id, &e.name.text, None));
            }
        }
        Action::Move {
            project_id,
            area_id,
        } => {
            let project = s.areas.catalog.projects.get(project_id)?;
            let old = project
                .area_id
                .as_ref()
                .and_then(|id| s.areas.catalog.areas.get(id))
                .map(|a| a.name.as_str())
                .unwrap_or("No area");
            card = card.child(roles::SMALL.wrap(
                format!(
                    "{} · currently {old}. Its project ID stays the same.",
                    project.name
                ),
                p.soft,
            ));
            let name = s
                .areas
                .catalog
                .areas
                .get(area_id)
                .map(|a| a.name.as_str())
                .unwrap_or("Choose an area");
            let mut menu = choice_menu(s, k, p, layer, "parent", name);
            for area in s.areas.catalog.areas.values() {
                let id = area.id.clone();
                menu = menu.item(
                    MenuItem::new(&area.name)
                        .checked(area.id == *area_id)
                        .on_select(move |s: &mut State| {
                            if let Some(e) = &mut s.areas.editor {
                                if let Action::Move { area_id, .. } = &mut e.action {
                                    *area_id = id.clone();
                                }
                                e.picker = None;
                                s.areas.preview = None;
                            }
                        }),
                );
            }
            card = card.child(menu.build(k, layer).self_start());
        }
        Action::Migrate { .. } => {
            card=card.child(roles::SMALL.wrap("Review every current project. Enter the area for each row. Shared area names create one area. Turn a broad label into an area only when you choose it.",p.soft));
            for (i, r) in e.rows.iter().enumerate() {
                let project = s.areas.catalog.projects.get(&r.project_id)?;
                let input = TextInput::new(&r.area)
                    .plain()
                    .placeholder("Enter proposed area")
                    .on_edit(move |s: &mut State| {
                        &mut s.areas.editor.as_mut().unwrap().rows[i].area
                    })
                    .on_change(|s: &mut State, _| s.areas.preview = None)
                    .height(36.0)
                    .build(k)
                    .key(format!("migration-area-{i}"))
                    .label(format!("Proposed area for {}", project.name))
                    .w_full()
                    .min_w(0.0)
                    .border(1.0, p.line)
                    .radius(6.0)
                    .fill(p.bg)
                    .disabled(busy);
                let mut item = col()
                    .w_full()
                    .gap(6.0)
                    .py(10.0)
                    .child(roles::NAME.wrap(&project.name, p.ink))
                    .child(input)
                    .child(
                        area::button(
                            if r.convert {
                                "Turn into area ✓"
                            } else {
                                "Keep as project"
                            },
                            &format!("migration-kind-{i}"),
                            p,
                        )
                        .self_start()
                        .disabled(busy)
                        .on_press(move |s: &mut State| {
                            if let Some(e) = &mut s.areas.editor {
                                e.rows[i].convert = !e.rows[i].convert;
                                e.rows[i].children.clear();
                                s.areas.preview = None;
                            }
                        }),
                    );
                if r.convert {
                    item = item.child(mapping(
                        s,
                        k,
                        p,
                        layer,
                        &r.project_id,
                        &r.area.text,
                        Some(i),
                    ));
                }
                card = card.child(item);
            }
        }
        Action::Assign { key, target } => {
            card=card.child(roles::SMALL.wrap(format!("File this item in {}. It will have no project. Review the count before applying.",s.areas.catalog.label(target)),p.soft));
            card = card.child(roles::META.wrap(key, p.faint));
        }
    }
    if let Some(preview) = &s.areas.preview {
        let count = |kind: &str| preview.references.iter().filter(|r| r.kind == kind).count();
        card=card.child(roles::GROUP.wrap(format!("{} meetings · {} tasks · {} memory · {} proposals",count("meeting"),count("task"),count("memory"),count("proposal")),p.ink))
            .child(roles::SMALL.wrap("One verified backup is saved before this change. Undo remains available after restart. Later moves can block Undo.",p.soft));
        if !matches!(e.action, Action::Convert { .. } | Action::Migrate { .. }) {
            for r in &preview.references {
                card = card.child(roles::META.wrap(format!("{} · {}", r.kind, r.label), p.soft));
            }
        }
        card = card.child(
            super::tasks::button("Confirm change", "area-confirm", true, p)
                .self_start()
                .disabled(busy)
                .on_press(|s: &mut State| s.confirm_area()),
        );
    } else {
        card = card.child(
            super::tasks::button("Preview counts", "area-preview", true, p)
                .self_start()
                .disabled(busy)
                .on_press(|s: &mut State| s.preview_area()),
        );
    }
    card = card.child(
        area::button("Cancel", "area-cancel", p)
            .self_start()
            .disabled(busy)
            .on_press(|s: &mut State| {
                s.areas.editor = None;
                s.areas.preview = None;
                s.areas.error = None;
            }),
    );
    Some(card)
}
pub fn assign(key: String, area_id: String) -> Action {
    Action::Assign {
        key,
        target: Target {
            area_id,
            project_id: None,
        },
    }
}
