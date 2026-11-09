use super::{
    State,
    area_editor::{self, Editor},
    nav::Filter,
    roles,
    theme::Palette,
    widgets::{pressable, scroll},
};
use rune::{El, col, row};
use rune_kit::Kit;
use sidenote::core::{
    self, Projects,
    area_ops::{Action, Preview, Receipt},
    areas::{Activity, Catalog},
};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct AreaUi {
    pub open_task_after_refresh: Option<(String, usize)>,
    pub meeting_projects: BTreeMap<String, super::project::MeetingProject>,
    pub project_labels: BTreeMap<String, String>,
    pub catalog: Catalog,
    pub activity: Activity,
    pub receipts: BTreeMap<String, Receipt>,
    pub editor: Option<Editor>,
    pub preview: Option<Preview>,
    pub error: Option<String>,
    pub pending: Option<&'static str>,
    pub generation: u64,
    pub refresh_generation: u64,
    pub refresh_pending: bool,
    pub refresh_again: bool,
    pub alias_names: std::collections::BTreeSet<String>,
    pub own_device: String,
    pub redirect_after_refresh: Option<String>,
    pub discard_action: Option<Option<Action>>,
    pub input_sink: rune::TextEdit,
    pub catalog_revision: String,
    pub notice: Option<String>,
    pub undo_available: std::collections::BTreeSet<String>,
    pub merge_undo_available: std::collections::BTreeSet<String>,
    pub receipt_devices: BTreeMap<String, String>,
    pub show_all: bool,
}
pub enum Completion {
    Refreshed(Box<Snapshot>),
    Preview(Box<(Preview, core::area_ops::Inventory)>),
    Inventory(core::area_ops::Inventory),
    Changed(Projects),
}
pub struct Snapshot {
    mutation_generation: u64,
    revision: String,
    inventory: Option<core::area_ops::Inventory>,
    undo_available: std::collections::BTreeSet<String>,
    merge_undo_available: std::collections::BTreeSet<String>,
    receipt_devices: BTreeMap<String, String>,
    checked: std::collections::HashMap<String, std::collections::BTreeSet<String>>,
    all_tasks: Vec<core::TaskItem>,
    task_items: Vec<core::TaskItem>,
    task_names: Vec<String>,
    task_counts: core::TaskCounts,
    meeting_projects: BTreeMap<String, super::project::MeetingProject>,
    project_labels: BTreeMap<String, String>,
    alias_names: std::collections::BTreeSet<String>,
    own_device: String,
    catalog: Catalog,
    activity: Activity,
    receipts: BTreeMap<String, Receipt>,
}
enum Job {
    Inventory,
    Preview(Action),
    Apply(Preview),
    Undo(String),
}
impl State {
    pub fn refresh_areas(&mut self) {
        if self.areas.refresh_pending {
            self.areas.refresh_again = true;
            return;
        }
        self.areas.refresh_generation += 1;
        self.areas.refresh_pending = true;
        let generation = self.areas.refresh_generation;
        let mutation_generation = core::mutation_generation();
        let (projects, meetings, pipeline, overrides, checked, cutoff, show_cleared) = (
            self.tasks.projects.clone(),
            self.meetings
                .iter()
                .map(|m| core::Meeting {
                    id: m.id.clone(),
                    title: m.title.clone(),
                    started: m.started,
                    summary: core::Summary {
                        todo: m.summary.todo.clone(),
                        ..Default::default()
                    },
                    folder: m.folder.clone(),
                    device: m.device.clone(),
                    summary_raw: String::new(),
                    transcript_raw: String::new(),
                    routed: None,
                    routed_revision: String::new(),
                    notes: String::new(),
                    lines: Vec::new(),
                    minutes: m.minutes,
                    export_path: None,
                    search: String::new(),
                })
                .collect::<Vec<_>>(),
            self.tasks.pipeline.clone(),
            self.tasks.overrides.clone(),
            self.tasks.checked.clone(),
            self.tasks.cutoff,
            self.tasks.show_cleared,
        );
        let running: std::collections::BTreeSet<_> = self
            .asks
            .iter()
            .filter(|(_, r)| r.live)
            .map(|(id, _)| id.clone())
            .collect();
        let (store, events, waker) = (
            self.tasks.store.clone(),
            self.events_tx.clone(),
            self.waker.clone(),
        );
        let migration_open = self
            .areas
            .editor
            .as_ref()
            .is_some_and(|e| matches!(e.action, Action::Migrate { .. }));
        let result = std::thread::Builder::new()
            .name("area-activity".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(|| -> std::io::Result<Completion> {
                    let mut running = running;
                    core::area_ops::proof_boundary(&store.root, "activity-worker")?;
                    running.extend(core::areas::running_meetings(&store.root).unwrap_or_default());
                    let mut catalog = projects.area_catalog()?;
                    catalog.problems.extend(store.catalog_problems());
                    let alias_names = projects
                        .projects
                        .iter()
                        .chain(projects.aliases.keys())
                        .filter(|n| projects.is_area_alias(n))
                        .map(|n| core::project_merge::normalized_name(n))
                        .chain(
                            catalog
                                .legacy_aliases
                                .keys()
                                .map(|n| core::project_merge::normalized_name(n)),
                        )
                        .collect();
                    let own_device = core::DeviceSettings::load()?.id;
                    let mut checked = checked;
                    for m in &meetings {
                        checked
                            .entry(m.id.clone())
                            .or_insert_with(|| store.load_ticks(&m.id));
                    }
                    let tasks =
                        core::build_tasks(&meetings, &pipeline, &projects, &overrides, &checked);
                    let kept = core::without_cleared(tasks.clone(), cutoff);
                    let task_counts = core::count_tasks(&kept);
                    let task_names = core::project_names(&projects, &tasks);
                    let project_labels = catalog
                        .projects
                        .values()
                        .map(|p| {
                            (
                                p.name.clone(),
                                p.converted_to
                                    .as_ref()
                                    .or(p.area_id.as_ref())
                                    .map(|id| {
                                        catalog.label(&core::areas::Target {
                                            area_id: id.clone(),
                                            project_id: p
                                                .converted_to
                                                .is_none()
                                                .then(|| p.id.clone()),
                                        })
                                    })
                                    .unwrap_or_else(|| p.name.clone()),
                            )
                        })
                        .collect();
                    let activity = core::areas::activity(
                        &projects,
                        &meetings,
                        &pipeline,
                        &tasks,
                        &running,
                        chrono::Utc::now(),
                    )?;
                    let mut meeting_projects =
                        super::project::cache(&projects, &pipeline, &overrides, &tasks, &meetings);
                    for (id, target) in &activity.meetings {
                        if let Some(chip) = meeting_projects.get_mut(id) {
                            chip.name = target
                                .project_id
                                .as_ref()
                                .and_then(|id| catalog.projects.get(id))
                                .map(|p| p.name.clone());
                        }
                    }
                    let receipts = projects.area_receipts()?;
                    let merge_undo_available = projects
                        .merges
                        .iter()
                        .filter(|(from, merge)| {
                            merge.active && store.local_project_merge_backup(from, merge).is_ok()
                        })
                        .map(|(_, merge)| merge.id.clone())
                        .collect();
                    let undo_available = receipts
                        .iter()
                        .filter(|(id, r)| {
                            core::area_ops::receipt_backup(&store.root, id, r, &own_device).is_ok()
                        })
                        .map(|(id, _)| id.clone())
                        .collect();
                    let receipt_devices = receipts
                        .values()
                        .map(|r| {
                            (
                                r.device.clone(),
                                core::area_version::device_name(&store.root, &r.device),
                            )
                        })
                        .collect();
                    Ok(Completion::Refreshed(Box::new(Snapshot {
                        mutation_generation,
                        revision: blake3::hash(&serde_json::to_vec(&projects)?)
                            .to_hex()
                            .to_string(),
                        inventory: if migration_open {
                            Some(store.area_references()?)
                        } else {
                            None
                        },
                        undo_available,
                        merge_undo_available,
                        receipt_devices,
                        meeting_projects,
                        project_labels,
                        alias_names,
                        own_device,
                        catalog,
                        activity,
                        receipts: projects.area_receipts()?,
                        task_items: if show_cleared { tasks.clone() } else { kept },
                        checked,
                        all_tasks: tasks,
                        task_counts,
                        task_names,
                    })))
                })
                .map_err(|_| "Could not refresh Areas. Reload to try again.".into())
                .and_then(|r| r.map_err(area_error));
                let _ = events.send(super::Event::Area {
                    generation,
                    refresh: true,
                    worker: std::thread::current().id(),
                    result,
                });
                if let Some(waker) = waker {
                    waker();
                }
            });
        if result.is_err() {
            self.areas.refresh_pending = false;
            self.areas.error =
                Some("Could not start the Areas worker. Reload to try again.".into());
        }
    }
    pub fn start_area(&mut self, action: Action) {
        if !self.config.features.projects || self.areas.pending.is_some() {
            return;
        }
        if self.areas.editor.as_ref().is_some_and(Editor::dirty) {
            self.areas.discard_action = Some(Some(action));
            return;
        }
        self.areas.editor = Some(Editor::new(action));
        self.areas.preview = None;
        self.areas.error = None;
        if self
            .areas
            .editor
            .as_ref()
            .is_some_and(|e| matches!(e.action, Action::Migrate { .. }))
        {
            self.launch_area(Job::Inventory, "Loading project counts...");
        }
    }
    pub fn assign_area(&mut self, key: String, area: String) {
        self.set_filter(Filter::Areas);
        self.start_area(area_editor::assign(key, area));
    }
    pub fn preview_area(&mut self) {
        if let Some(e) = &self.areas.editor {
            self.launch_area(Job::Preview(e.action()), "Preparing preview...");
        }
    }
    pub fn confirm_area(&mut self) {
        if let Some(preview) = &self.areas.preview
            && self
                .areas
                .editor
                .as_ref()
                .is_none_or(|e| e.action() != preview.action)
        {
            self.areas.preview = None;
            self.areas.error =
                Some("The mapping changed. Review it again before confirming.".into());
            return;
        }
        if let Some(preview) = self.areas.preview.clone() {
            self.launch_area(Job::Apply(preview), "Applying change...");
        }
    }
    pub fn cancel_area(&mut self) {
        if self.areas.editor.as_ref().is_some_and(Editor::dirty) {
            self.areas.discard_action = Some(None);
            return;
        }
        self.areas.editor = None;
        self.areas.preview = None;
        self.areas.error = None;
    }
    pub fn undo_area(&mut self, id: String) {
        self.launch_area(Job::Undo(id), "Undoing change...");
    }
    fn launch_area(&mut self, job: Job, label: &'static str) {
        if !self.config.features.projects || self.areas.pending.is_some() {
            return;
        }
        self.areas.pending = Some(label);
        if let Some(e) = &mut self.areas.editor {
            e.picker = None;
        }
        self.areas.error = None;
        self.areas.notice = None;
        self.areas.generation += 1;
        let generation = self.areas.generation;
        let (store, events, waker) = (
            self.tasks.store.clone(),
            self.events_tx.clone(),
            self.waker.clone(),
        );
        if std::thread::Builder::new()
            .name("area-operation".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(|| -> std::io::Result<Completion> {
                    core::area_ops::proof_boundary(&store.root, "operation-worker")?;
                    match job {
                        Job::Inventory => store.area_references().map(Completion::Inventory),
                        Job::Preview(action) => store.preview_area(action).map(|preview| {
                            let inventory =
                                core::area_ops::Inventory::new(preview.references.clone());
                            Completion::Preview(Box::new((preview, inventory)))
                        }),
                        Job::Apply(preview) => store.apply_area(&preview).map(Completion::Changed),
                        Job::Undo(id) => store.undo_area(&id).map(Completion::Changed),
                    }
                })
                .map_err(|_| "Could not finish the area change. Reload before trying again.".into())
                .and_then(|r| r.map_err(area_error));
                let _ = events.send(super::Event::Area {
                    generation,
                    refresh: false,
                    worker: std::thread::current().id(),
                    result,
                });
                if let Some(waker) = waker {
                    waker();
                }
            })
            .is_err()
        {
            self.areas.pending = None;
            self.areas.error =
                Some("Could not start the Areas worker. Reload to try again.".into());
        }
    }
    pub fn area_finished(
        &mut self,
        generation: u64,
        refresh: bool,
        result: Result<Completion, String>,
    ) {
        if refresh {
            if generation != self.areas.refresh_generation {
                return;
            }
            self.areas.refresh_pending = false;
        } else {
            if generation != self.areas.generation {
                return;
            }
            self.areas.pending = None;
        }
        match result {
            Ok(Completion::Refreshed(snapshot)) => {
                if snapshot.mutation_generation != core::mutation_generation()
                    || self.areas.refresh_again
                {
                    self.areas.refresh_again = false;
                    self.refresh_areas();
                    return;
                }
                if self.areas.catalog_revision != snapshot.revision {
                    if let Some(e) = &mut self.areas.editor {
                        e.merge_catalog(&snapshot.catalog);
                        self.areas.preview = None;
                    }
                    self.areas.catalog_revision = snapshot.revision;
                }
                if let Some(inventory) = snapshot.inventory
                    && let Some(e) = &mut self.areas.editor
                {
                    e.set_inventory(inventory);
                }
                self.areas.undo_available = snapshot.undo_available;
                self.areas.merge_undo_available = snapshot.merge_undo_available;
                self.areas.receipt_devices = snapshot.receipt_devices;
                self.tasks.today = chrono::Local::now().date_naive();
                self.tasks.checked = snapshot.checked;
                self.tasks.all = snapshot.all_tasks;
                self.tasks.items = snapshot.task_items;
                self.tasks.names = snapshot.task_names;
                self.tasks.counts = snapshot.task_counts;
                self.areas.meeting_projects = snapshot.meeting_projects;
                self.areas.project_labels = snapshot.project_labels;
                self.areas.alias_names = snapshot.alias_names;
                self.areas.own_device = snapshot.own_device;
                self.areas.catalog = snapshot.catalog;
                if let Some(name) = self.areas.redirect_after_refresh.take()
                    && let Some(id) = self
                        .areas
                        .catalog
                        .project(&self.tasks.projects, &name)
                        .and_then(|p| p.converted_to.clone())
                {
                    self.filter = Filter::Area(id);
                }
                self.areas.activity = snapshot.activity;
                self.areas.receipts = snapshot.receipts;
                self.rebuild_nav();
                self.refresh_list();
                if let Some((id, index)) = self.areas.open_task_after_refresh.take() {
                    self.open_task(&id, index);
                }
            }
            Ok(Completion::Preview(data)) => {
                let (preview, inventory) = *data;
                if self
                    .areas
                    .editor
                    .as_ref()
                    .is_none_or(|e| e.action() != preview.action)
                {
                    self.areas.error =
                        Some("The mapping changed while reviewing. Review it again.".into());
                    return;
                }
                if let Some(e) = &mut self.areas.editor {
                    if !matches!(e.action, Action::Migrate { .. }) {
                        e.references = preview.references.clone();
                    }
                    if !matches!(e.action, Action::Migrate { .. }) {
                        e.set_inventory(inventory);
                    }
                }
                self.areas.preview = Some(preview);
            }
            Ok(Completion::Inventory(inventory)) => {
                if let Some(e) = &mut self.areas.editor {
                    e.set_inventory(inventory);
                }
            }
            Ok(Completion::Changed(projects)) => {
                self.tasks.projects = projects;
                self.areas.preview = None;
                self.areas.editor = None;
                if let Filter::Project(name) = &self.filter {
                    self.areas.redirect_after_refresh = Some(name.clone());
                }
                if let Some(receipt) = self.tasks.projects.area_receipts().ok().and_then(|r| {
                    r.into_values().rfind(|r| {
                        !r.active
                            && !r.skipped.is_empty()
                            && self.areas.receipts.get(&r.id).is_some_and(|old| old.active)
                    })
                }) {
                    self.areas.notice = Some(format!(
                        "{} items changed since; left as they are: {}",
                        receipt.skipped.len(),
                        receipt.skipped.join(", ")
                    ));
                }
                self.rebuild_tasks();
                self.memory.notice =
                    Some("Reload memory to see current area and project assignments.".into());
            }
            Err(error) => {
                self.areas.preview = None;
                self.areas.error = Some(error);
            }
        }
        if refresh && self.areas.refresh_again {
            self.areas.refresh_again = false;
            self.refresh_areas();
        }
    }
}
fn area_error(error: std::io::Error) -> String {
    core::validation_message(&error)
        .unwrap_or("Could not finish the area change. Your data was kept. Reload and try again.")
        .into()
}
fn undo_cards(s: &State, p: Palette) -> El {
    let mut content = col().w_full().gap(8.0);
    for receipt in s.areas.receipts.values().filter(|r| r.active).rev() {
        let id = receipt.id.clone();
        let date = id
            .split('-')
            .next()
            .and_then(|t| chrono::NaiveDateTime::parse_from_str(t, "%Y%m%dT%H%M%S").ok())
            .map(|t| t.format("%-d %b %H:%M").to_string())
            .unwrap_or_else(|| "Unknown time".into());
        let projects = match &receipt.action {
            Action::Migrate { rows } => rows.len(),
            _ => receipt
                .after
                .keys()
                .filter(|k| k.starts_with("area-project:"))
                .count(),
        };
        let title = if matches!(receipt.action, Action::Migrate { .. }) {
            "Project migration"
        } else {
            area_editor::title(&receipt.action)
        };
        let mut card = col()
            .w_full()
            .gap(6.0)
            .py(10.0)
            .child(roles::NAME.wrap(format!("{title}, {date}, {projects} projects"), p.ink));
        if receipt.device != s.areas.own_device {
            card = card.child(roles::META.wrap(
                format!(
                        "Done on {}",
                        s.areas
                            .receipt_devices
                            .get(&receipt.device)
                            .unwrap_or(&receipt.device)
                    ),
                p.faint,
            ));
        } else if s.areas.undo_available.contains(&id) {
            card = card.child(
                button("Undo change", &format!("area-undo-{id}"), p)
                    .self_start()
                    .disabled(s.areas.pending.is_some())
                    .on_press(move |s: &mut State| s.undo_area(id.clone())),
            );
        } else {
            card = card.child(roles::META.wrap("Undo backup is unavailable or expired.", p.faint));
        }
        content = content.child(card);
    }
    content
}
pub fn button(title: impl Into<String>, key: &str, p: Palette) -> El {
    pressable(roles::CONTROL.wrap(title, p.accent))
        .key(key)
        .px(10.0)
        .py(7.0)
        .radius(6.0)
        .border(1.0, p.line)
        .hover(p.chip)
}
pub fn notices(s: &State, p: Palette) -> El {
    let mut content = col().w_full().gap(8.0);
    for problem in &s.areas.catalog.problems {
        content = content.child(roles::SMALL.wrap(problem, p.warn).role("status"));
    }
    for (id, choices) in &s.areas.catalog.name_conflicts {
        for name in choices {
            let (id, name) = (id.clone(), name.clone());
            content = content.child(
                button(format!("Keep {name}"), &format!("resolve-{id}-{name}"), p).on_press(
                    move |s: &mut State| {
                        s.start_area(Action::ResolveName {
                            project_id: id.clone(),
                            name: name.clone(),
                        })
                    },
                ),
            );
        }
    }
    if s.areas.discard_action.is_some() {
        content = content
            .child(roles::SMALL.wrap("Discard your mapping choices?", p.warn))
            .child(
                row()
                    .gap(8.0)
                    .child(
                        button("Keep editing", "area-keep-editing", p)
                            .on_press(|s: &mut State| s.areas.discard_action = None),
                    )
                    .child(button("Discard choices", "area-discard", p).on_press(
                        |s: &mut State| {
                            let next = s.areas.discard_action.take();
                            s.areas.editor = None;
                            s.areas.preview = None;
                            s.areas.error = None;
                            if let Some(Some(action)) = next {
                                s.start_area(action);
                            }
                        },
                    )),
            );
    }
    if let Some(pending) = s.areas.pending {
        content = content.child(roles::SMALL.wrap(pending, p.soft).role("status"));
    }
    content
}
pub fn project_tools(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> Option<El> {
    if !s.config.features.projects {
        return None;
    }
    let Filter::Project(name) = &s.filter else {
        return None;
    };
    let project = s.areas.catalog.project(&s.tasks.projects, name)?;
    let (move_id, convert_id) = (project.id.clone(), project.id.clone());
    let mut content = col().w_full().gap(10.0).mb(12.0);
    if let Some(area) = project
        .area_id
        .as_ref()
        .and_then(|id| s.areas.catalog.areas.get(id))
    {
        let id = area.id.clone();
        content = content.child(
            pressable(roles::CONTROL.wrap(format!("{} › {}", area.name, project.name), p.accent))
                .on_press(move |s: &mut State| s.set_filter(Filter::Area(id.clone()))),
        );
    } else {
        content = content.child(roles::META.wrap("No area assigned", p.faint));
    }
    content = content.child(
        row()
            .w_full()
            .wrap()
            .gap(8.0)
            .child(
                button("Move into area", "area-move", p)
                    .disabled(s.areas.pending.is_some())
                    .on_press(move |s: &mut State| {
                        s.start_area(Action::Move {
                            project_id: move_id.clone(),
                            area_id: String::new(),
                        })
                    }),
            )
            .child(
                button("Turn into area", "area-convert", p)
                    .disabled(s.areas.pending.is_some())
                    .on_press(move |s: &mut State| {
                        s.start_area(Action::Convert {
                            project_id: convert_id.clone(),
                            area_name: String::new(),
                            children: Default::default(),
                        })
                    }),
            ),
    );
    Some(
        content
            .children(area_editor::panel(s, k, p, layer))
            .child(notices(s, p)),
    )
}
fn project_rows(s: &State, area: Option<&str>, p: Palette) -> El {
    let mut projects: Vec<_> = s
        .areas
        .catalog
        .projects
        .values()
        .filter(|project| {
            project.converted_to.is_none()
                && project.area_id.as_deref() == area
                && (s.areas.show_all || s.areas.activity.projects.contains(&project.id))
        })
        .collect();
    projects.sort_by_key(|project| {
        (
            !s.areas.activity.projects.contains(&project.id),
            project.name.to_lowercase(),
        )
    });
    if projects.is_empty() {
        return roles::SMALL.wrap("No projects to show.", p.faint);
    }
    let ids: std::sync::Arc<Vec<String>> =
        std::sync::Arc::new(projects.iter().map(|p| p.id.clone()).collect());
    let keys = ids.clone();
    col()
        .w_full()
        .h((projects.len() as f32 * 58.0).min(300.0))
        .min_h(0.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .w_full()
                .growth(rune::GrowthSpec::default())
                .virtual_rows(
                    ids.len(),
                    58.0,
                    move |_: &State, i| {
                        keys.get(i)
                            .cloned()
                            .unwrap_or_else(|| "missing-project".into())
                            .into()
                    },
                    move |s: &State, i| {
                        let Some(project) =
                            ids.get(i).and_then(|id| s.areas.catalog.projects.get(id))
                        else {
                            return col();
                        };
                        let active = s.areas.activity.projects.contains(&project.id);
                        let name = project.name.clone();
                        pressable(
                            row()
                                .w_full()
                                .gap(12.0)
                                .items_center()
                                .py(12.0)
                                .child(roles::NAME.line(&project.name, p.ink).grow(1.0))
                                .child(
                                    roles::META
                                        .wrap(if active { "Active" } else { "Inactive" }, p.faint),
                                ),
                        )
                        .key(format!("area-project-{}", project.id))
                        .hover(p.chip)
                        .radius(6.0)
                        .px(8.0)
                        .on_press(move |s: &mut State| s.set_filter(Filter::Project(name.clone())))
                    },
                ),
        )
}
fn area_tasks(s: &State, area: &str, p: Palette) -> El {
    let indices: std::sync::Arc<Vec<usize>> = std::sync::Arc::new(
        s.tasks
            .all
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                t.area_id.as_deref() == Some(area)
                    && t.project == core::UNSORTED
                    && (!t.done || s.areas.show_all)
            })
            .map(|(i, _)| i)
            .collect(),
    );
    if indices.is_empty() {
        return roles::SMALL.wrap("No open area-only tasks.", p.faint);
    }
    let keys = indices.clone();
    col()
        .w_full()
        .h((indices.len() as f32 * 76.0).min(320.0))
        .min_h(0.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .w_full()
                .growth(rune::GrowthSpec::default())
                .virtual_rows(
                    indices.len(),
                    76.0,
                    move |s: &State, i| {
                        keys.get(i)
                            .and_then(|j| s.tasks.all.get(*j))
                            .map(|t| core::areas::task_key(t).into())
                            .unwrap_or_else(|| "missing-task".into())
                    },
                    move |s: &State, i| {
                        let Some(task) = indices.get(i).and_then(|j| s.tasks.all.get(*j)) else {
                            return col();
                        };
                        let (id, index) = (task.meeting_id.clone(), task.index);
                        pressable(
                            col()
                                .w_full()
                                .py(9.0)
                                .gap(2.0)
                                .child(
                                    roles::SMALL
                                        .wrap(&task.task, if task.done { p.faint } else { p.ink }),
                                )
                                .child(
                                    roles::META.wrap(
                                        if task.done { "Done" } else { "Open task" },
                                        p.faint,
                                    ),
                                ),
                        )
                        .hover(p.chip)
                        .on_press(move |s: &mut State| {
                            s.set_view(super::tasks::View::Tasks);
                            s.open_task(&id, index);
                        })
                    },
                ),
        )
}
fn area_meetings(s: &State, area: &str, p: Palette) -> El {
    let ids: std::sync::Arc<Vec<usize>> = std::sync::Arc::new(
        s.meetings
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                s.areas
                    .activity
                    .meetings
                    .get(&m.id)
                    .is_some_and(|t| t.area_id == area && t.project_id.is_none())
            })
            .map(|(i, _)| i)
            .collect(),
    );
    if ids.is_empty() {
        return roles::SMALL.wrap("No area-only meetings.", p.faint);
    }
    let keys = ids.clone();
    col()
        .w_full()
        .h((ids.len() as f32 * 56.0).min(280.0))
        .min_h(0.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .w_full()
                .growth(rune::GrowthSpec::default())
                .virtual_rows(
                    ids.len(),
                    56.0,
                    move |s: &State, i| {
                        keys.get(i)
                            .and_then(|j| s.meetings.get(*j))
                            .map(|m| m.id.as_str())
                            .unwrap_or("missing-meeting")
                            .into()
                    },
                    move |s: &State, i| {
                        let Some(m) = ids.get(i).and_then(|j| s.meetings.get(*j)) else {
                            return col();
                        };
                        let id = m.id.clone();
                        pressable(roles::SMALL.wrap(&m.title, p.ink))
                            .py(10.0)
                            .hover(p.chip)
                            .on_press_tx(move |s: &mut State, tx| s.select(&id, tx))
                    },
                ),
        )
}
pub fn pane(s: &State, k: &Kit, p: Palette, narrow: bool, layer: &mut Vec<El>) -> El {
    if s.areas
        .editor
        .as_ref()
        .is_some_and(|e| matches!(e.action, Action::Migrate { .. }))
    {
        return super::area_migration::pane(s, k, p, narrow);
    }
    let area = match &s.filter {
        Filter::Area(id) => s.areas.catalog.areas.get(id),
        _ => None,
    };
    let title = area.map(|a| a.name.as_str()).unwrap_or("Areas");
    let mut doc = col()
        .w_full()
        .max_w(860.0)
        .px(if narrow { 20.0 } else { 40.0 })
        .pt(20.0)
        .pb(40.0)
        .gap(12.0)
        .child(super::list::text_title(
            title,
            if narrow { 28.0 } else { 34.0 },
            p,
        ))
        .child(roles::SMALL.wrap(
            if area.is_some() {
                "An ongoing domain, with projects that can finish."
            } else {
                "Keep ongoing domains here. Give each project one home."
            },
            p.soft,
        ));
    if s.areas.editor.is_none() {
        doc = doc.child(undo_cards(s, p));
    }
    if let Some(area) = area {
        let (id, name) = (area.id.clone(), area.name.clone());
        if area.status == core::areas::Status::Retired {
            let name = area.name.clone();
            doc = doc.child(
                button("Restore area", "area-restore", p)
                    .self_start()
                    .disabled(s.areas.pending.is_some())
                    .on_press(move |s: &mut State| {
                        s.start_area(Action::Create { name: name.clone() })
                    }),
            );
        }
        doc = doc.child(
            row()
                .wrap()
                .gap(8.0)
                .child(
                    button("All areas", "area-home", p)
                        .on_press(|s: &mut State| s.set_filter(Filter::Areas)),
                )
                .child(
                    button("Rename area", "area-rename", p)
                        .disabled(s.areas.pending.is_some())
                        .on_press(move |s: &mut State| {
                            s.start_area(Action::Rename {
                                area_id: id.clone(),
                                name: name.clone(),
                            })
                        }),
                ),
        );
        doc = doc
            .child(roles::section("Projects", None, p))
            .child(project_rows(s, Some(&area.id), p));
        doc = doc.child(roles::section("Area-only work", None, p));
        doc = doc.child(area_tasks(s, &area.id, p));
        doc = doc.child(roles::section("Area-only meetings", None, p));
        doc = doc.child(area_meetings(s, &area.id, p));
    } else {
        doc = doc.child(
            row()
                .wrap()
                .gap(8.0)
                .child(
                    button("Create area", "area-create", p)
                        .disabled(s.areas.pending.is_some())
                        .on_press(|s: &mut State| {
                            s.start_area(Action::Create {
                                name: String::new(),
                            })
                        }),
                )
                .child(
                    button("Review migration", "area-migrate", p)
                        .disabled(s.areas.pending.is_some())
                        .on_press(|s: &mut State| {
                            s.start_area(area_editor::migration(&s.areas.catalog))
                        }),
                ),
        );
        if s.areas.editor.is_none() {
            for area in s
                .areas
                .catalog
                .areas
                .values()
                .filter(|a| s.areas.show_all || s.areas.activity.areas.contains(&a.id))
            {
                let id = area.id.clone();
                let count = s
                    .areas
                    .catalog
                    .projects
                    .values()
                    .filter(|p| p.area_id.as_ref() == Some(&area.id) && p.converted_to.is_none())
                    .count();
                doc = doc.child(
                    pressable(
                        col()
                            .w_full()
                            .py(12.0)
                            .gap(3.0)
                            .child(roles::TITLE.wrap(&area.name, p.ink))
                            .child(roles::META.wrap(
                                format!(
                                    "{count} projects · {}",
                                    if area.status == core::areas::Status::Retired {
                                        "Retired"
                                    } else if s.areas.activity.areas.contains(&area.id) {
                                        "Active"
                                    } else {
                                        "Inactive"
                                    }
                                ),
                                p.faint,
                            )),
                    )
                    .hover(p.chip)
                    .on_press(move |s: &mut State| s.set_filter(Filter::Area(id.clone()))),
                );
            }
            doc = doc
                .child(roles::section("Projects without an area", None, p))
                .child(project_rows(s, None, p));
        }
    }
    doc = doc.child(
        button(
            if s.areas.show_all {
                "Show active"
            } else {
                "Show all, including inactive"
            },
            "area-show-all",
            p,
        )
        .self_start()
        .on_press(|s: &mut State| s.areas.show_all = !s.areas.show_all),
    );
    doc = doc
        .children(area_editor::panel(s, k, p, layer))
        .child(notices(s, p));
    col()
        .grow(1.0)
        .w_full()
        .min_w(0.0)
        .min_h(0.0)
        .fill(p.bg)
        .child(scroll(
            col().w_full().items_center().child(doc),
            "area-scroll",
        ))
}
