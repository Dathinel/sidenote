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
    pub assignments: std::collections::BTreeMap<String, Option<String>>,
    pub expanded: bool,
    pub search: TextEdit,
    pub visible_items: Vec<usize>,
}
pub struct Editor {
    pub action: Action,
    pub name: TextEdit,
    pub rows: Vec<RowInput>,
    pub references: Vec<Reference>,
    pub inventory: sidenote::core::area_ops::Inventory,
    pub search: TextEdit,
    pub visible_projects: Vec<usize>,
    pub notice: Option<String>,
    pub picker: Option<String>,
}
impl Editor {
    pub fn set_inventory(&mut self, inventory: sidenote::core::area_ops::Inventory) {
        for row in &mut self.rows {
            row.visible_items = inventory
                .by_project
                .get(&row.project_id)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|i| {
                    row.search.text.is_empty()
                        || inventory.references[*i]
                            .label
                            .to_lowercase()
                            .contains(&row.search.text.to_lowercase())
                })
                .collect();
        }
        self.inventory = inventory;
    }
    pub fn dirty(&self) -> bool {
        self.rows.iter().any(|r| {
            r.convert
                || !r.area.text.is_empty()
                || !r.assignments.is_empty()
                || !r.children.is_empty()
        }) || !self.name.text.is_empty()
    }
    pub fn merge_catalog(&mut self, c: &Catalog) {
        if !matches!(self.action, Action::Migrate { .. }) {
            return;
        }
        let Action::Migrate { rows } = migration(c) else {
            return;
        };
        let mut added = 0;
        for row in rows {
            if !self.rows.iter().any(|r| r.project_id == row.project_id) {
                self.rows.push(RowInput {
                    project_id: row.project_id,
                    area: TextEdit::default(),
                    convert: false,
                    children: Default::default(),
                    assignments: Default::default(),
                    expanded: false,
                    search: TextEdit::default(),
                    visible_items: Vec::new(),
                });
                added += 1;
            }
        }
        if added > 0 {
            self.notice = Some(format!(
                "{added} new projects need review. Your existing choices were kept."
            ));
            self.visible_projects = (0..self.rows.len()).collect();
        }
        self.reconcile();
    }
    pub fn reconcile(&mut self) {
        let targets: std::collections::BTreeMap<_, _> = self
            .rows
            .iter()
            .filter(|r| !r.convert)
            .map(|r| {
                (
                    r.project_id.clone(),
                    sidenote::core::project_merge::normalized_name(&r.area.text),
                )
            })
            .collect();
        let mut cleared = 0;
        for row in &mut self.rows {
            let area = sidenote::core::project_merge::normalized_name(&row.area.text);
            row.children.retain(|_, id| {
                let keep = targets.get(id) == Some(&area);
                cleared += usize::from(!keep);
                keep
            });
            row.assignments.retain(|_, target| {
                let keep = target
                    .as_ref()
                    .is_none_or(|id| targets.get(id) == Some(&area));
                cleared += usize::from(!keep);
                keep
            });
        }
        if cleared > 0 {
            self.notice = Some(format!(
                "{cleared} item choices cleared because their project changed area or became an area."
            ));
        }
    }
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
                    assignments: r.assignments.clone(),
                    expanded: false,
                    search: TextEdit::default(),
                    visible_items: Vec::new(),
                })
                .collect(),
            _ => Vec::new(),
        };
        let visible_projects = (0..rows.len()).collect();
        Self {
            name: TextEdit::new(name),
            action,
            rows,
            references: Vec::new(),
            inventory: Default::default(),
            search: TextEdit::default(),
            visible_projects,
            notice: None,
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
                        assignments: r.assignments.clone(),
                    })
                    .collect(),
            },
            other => other.clone(),
        }
    }
}
pub fn title(action: &Action) -> &'static str {
    match action {
        Action::ResolveName { .. } => "Resolve project name",
        Action::Create { .. } => "Create area",
        Action::Rename { .. } => "Rename area",
        Action::Move { .. } => "Move project into area",
        Action::Convert { .. } => "Turn project into area",
        Action::Migrate { .. } => "Review project migration",
        Action::Assign { .. } => "Assign area-only work",
    }
}
pub fn migration(c: &Catalog) -> Action {
    let mut rows: Vec<_> = c
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
            assignments: Default::default(),
        })
        .collect();
    rows.sort_by_key(|r| c.projects[&r.project_id].name.to_lowercase());
    Action::Migrate { rows }
}
fn name_input(s: &State, k: &Kit, p: Palette) -> El {
    let Some(editor) = s.areas.editor.as_ref() else {
        return col();
    };
    TextInput::new(&editor.name)
        .plain()
        .placeholder("Area name")
        .on_edit(|s: &mut State| match s.areas.editor.as_mut() {
            Some(e) => &mut e.name,
            None => &mut s.areas.input_sink,
        })
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
fn mapping_item(s: &State, index: usize, project: &str, area_name: &str, p: Palette) -> El {
    let Some(e) = s.areas.editor.as_ref() else {
        return col();
    };
    let Some(r) = e.references.get(index) else {
        return col();
    };
    let Action::Convert { children, .. } = &e.action else {
        return col();
    };
    let selected = children.get(&r.key);
    let label = selected
        .and_then(|id| s.areas.catalog.projects.get(id))
        .map(|p| p.name.as_str())
        .unwrap_or("Area-only work");
    let mut layer = Vec::new();
    let k = super::theme::kit(&p);
    let mut menu = choice_menu(s, &k, p, &mut layer, &r.key, label);
    for (id, name) in std::iter::once((String::new(), "Area-only work".into()))
        .chain(child_choices(s, project, area_name))
    {
        let key = r.key.clone();
        menu = menu.item(
            MenuItem::new(name)
                .checked(selected.map(String::as_str).unwrap_or("") == id)
                .on_select(move |s: &mut State| {
                    let Some(e) = &mut s.areas.editor else {
                        return;
                    };
                    let Action::Convert { children, .. } = &mut e.action else {
                        return;
                    };
                    if id.is_empty() {
                        children.remove(&key);
                    } else {
                        children.insert(key.clone(), id.clone());
                    }
                    e.picker = None;
                    s.areas.preview = None;
                }),
        );
    }
    col()
        .w_full()
        .gap(4.0)
        .py(6.0)
        .child(roles::META.wrap(format!("{}: {}", r.kind, r.label), p.soft))
        .child(menu.build(&k, &mut layer).self_start())
        .children(layer)
}
fn mapping(
    s: &State,
    _k: &Kit,
    p: Palette,
    _layer: &mut Vec<El>,
    project: &str,
    area_name: &str,
    _row_index: Option<usize>,
) -> El {
    let Some(e) = s.areas.editor.as_ref() else {
        return col();
    };
    let indices: std::sync::Arc<Vec<usize>> = std::sync::Arc::new(
        e.references
            .iter()
            .enumerate()
            .filter(|(_, r)| r.project_id.as_deref() == Some(project))
            .map(|(i, _)| i)
            .collect(),
    );
    let count = indices.len();
    let keys = indices.clone();
    let (project, area) = (project.to_string(), area_name.to_string());
    if count == 0 {
        return roles::SMALL.wrap("Review counts to see the items in this project.", p.soft);
    }
    col()
        .w_full()
        .h(320.0)
        .min_h(0.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .w_full()
                .growth(rune::GrowthSpec::default())
                .virtual_rows(
                    count,
                    100.0,
                    move |s: &State, i| {
                        s.areas
                            .editor
                            .as_ref()
                            .and_then(|e| keys.get(i).and_then(|j| e.references.get(*j)))
                            .map(|r| r.key.as_str())
                            .unwrap_or("missing-item")
                            .into()
                    },
                    move |s: &State, i| {
                        indices
                            .get(i)
                            .map(|index| mapping_item(s, *index, &project, &area, p))
                            .unwrap_or_else(col)
                    },
                ),
        )
}
fn preview_items(s: &State, p: Palette) -> El {
    let Some(preview) = &s.areas.preview else {
        return col();
    };
    col()
        .w_full()
        .h(240.0)
        .min_h(0.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .w_full()
                .growth(rune::GrowthSpec::default())
                .virtual_rows(
                    preview.references.len(),
                    64.0,
                    |s: &State, i| {
                        s.areas
                            .preview
                            .as_ref()
                            .and_then(|p| p.references.get(i))
                            .map(|r| r.key.as_str())
                            .unwrap_or("missing-item")
                            .into()
                    },
                    move |s: &State, i| {
                        s.areas
                            .preview
                            .as_ref()
                            .and_then(|p| p.references.get(i))
                            .map(|r| roles::META.wrap(format!("{}: {}", r.kind, r.label), p.soft))
                            .unwrap_or_else(col)
                    },
                ),
        )
}
pub fn panel(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> Option<El> {
    let e = s.areas.editor.as_ref()?;
    if matches!(e.action, Action::Migrate { .. }) {
        return None;
    }
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
    if let Some(preview) = &s.areas.preview {
        let count = |kind: &str| preview.references.iter().filter(|r| r.kind == kind).count();
        card=card.child(roles::GROUP.wrap(format!("{} meetings · {} tasks · {} memory · {} proposals",count("meeting"),count("task"),count("memory"),count("proposal")),p.ink))
            .child(roles::SMALL.wrap("One verified backup is saved before this change. Undo remains available after restart. Undo leaves later changes in place.",p.soft));
        if matches!(e.action, Action::Migrate { .. }) {
            card = card.child(roles::META.wrap(
                format!("{} project rows reviewed", preview.project_count),
                p.faint,
            ));
        }
    }
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
        Action::Migrate { .. } => return None,
        Action::ResolveName { name, .. } => {
            card = card.child(roles::SMALL.wrap(
                format!("Keep the project name {name}. Its identity and work stay together."),
                p.soft,
            ));
        }
        Action::Assign { target, .. } => {
            card=card.child(roles::SMALL.wrap(format!("File this item in {}. It will have no project. Review the count before applying.",s.areas.catalog.label(target)),p.soft));
        }
    }
    if s.areas.preview.is_some() {
        if !matches!(e.action, Action::Convert { .. } | Action::Migrate { .. }) {
            card = card.child(preview_items(s, p));
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
                s.cancel_area();
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
