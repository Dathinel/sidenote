use super::{State, nav::Filter, roles, theme::Palette, widgets::pressable};
use rune::{El, col, row};
use rune_kit::{Kit, Menu, MenuItem, Placement};
use sidenote::core::project_merge::MergePreview;

#[derive(Default)]
pub struct MergeUi {
    pub from: Option<String>,
    pub preview: Option<MergePreview>,
    pub error: Option<String>,
    pub menu: bool,
}
impl State {
    pub fn start_project_merge(&mut self, from: &str) {
        if !self.config.features.projects {
            return;
        }
        self.tasks.rename = None;
        self.project_merge = MergeUi {
            from: Some(from.into()),
            ..Default::default()
        };
        self.go_home();
        self.filter = Filter::Project(from.into());
        self.refresh_list();
    }
    pub fn preview_project_merge(&mut self, into: &str) {
        let Some(from) = &self.project_merge.from else {
            return;
        };
        match self.tasks.store.preview_merge(from, into) {
            Ok(preview) => {
                self.project_merge.preview = Some(preview);
                self.project_merge.error = None;
            }
            Err(e) => self.project_merge.error = Some(e.to_string()),
        }
    }
    fn confirm_project_merge(&mut self) {
        let Some(preview) = &self.project_merge.preview else {
            return;
        };
        let into = preview.into.clone();
        match self.tasks.store.merge_projects(preview) {
            Ok(projects) => self.refresh_project_merge(projects, into),
            Err(e) => {
                self.project_merge.error = Some(e.to_string());
                self.project_merge.preview = None;
            }
        }
    }
    fn undo_project_merge(&mut self, from: &str, id: &str) {
        match self.tasks.store.undo_project_merge(from, id) {
            Ok(projects) => self.refresh_project_merge(projects, from.into()),
            Err(e) => self.project_merge.error = Some(e.to_string()),
        }
    }
    fn refresh_project_merge(&mut self, projects: sidenote::core::Projects, selected: String) {
        self.tasks.projects = projects;
        self.project_merge = MergeUi::default();
        self.rebuild_tasks();
        self.filter = Filter::Project(selected);
        self.refresh_list();
        if self.config.features.memory {
            self.memory.refresh(false);
        }
    }
}
fn button(title: impl Into<String>, key: &str, p: Palette) -> El {
    pressable(roles::CONTROL.wrap(title, p.accent))
        .key(key)
        .px(10.0)
        .py(7.0)
        .radius(6.0)
        .border(1.0, p.line)
        .hover(p.chip)
}
pub fn panel(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> Option<El> {
    if !s.config.features.projects {
        return None;
    }
    let Filter::Project(name) = &s.filter else {
        return None;
    };
    let start = name.clone();
    let menu_start = name.clone();
    let mut menu = Menu::new("project-actions")
        .trigger(
            button("Project menu", "project-actions", p)
                .on_press(|s: &mut State| s.project_merge.menu = !s.project_merge.menu),
        )
        .open(s.project_merge.menu)
        .placement(Placement::BottomStart);
    if s.project_merge.menu {
        menu = menu
            .on_close(|s: &mut State| s.project_merge.menu = false)
            .item(
                MenuItem::new("Merge into...")
                    .on_select(move |s: &mut State| s.start_project_merge(&menu_start)),
            );
    }
    let mut content = col().w_full().gap(10.0).mb(16.0).child(
        row()
            .gap(8.0)
            .child(
                button("Merge into...", "project-merge", p)
                    .on_press(move |s: &mut State| s.start_project_merge(&start)),
            )
            .child(menu.build(k, layer)),
    );
    for (from, merge) in &s.tasks.projects.merges {
        if !merge.active || s.tasks.projects.resolve(&merge.into) != *name {
            continue;
        }
        let (from, id) = (from.clone(), merge.id.clone());
        content = content.child(
            col()
                .w_full()
                .gap(6.0)
                .px(12.0)
                .py(12.0)
                .radius(8.0)
                .fill(p.okbg)
                .child(roles::NAME.wrap(format!("Merged {from} into {name}"), p.ink))
                .child(roles::SMALL.wrap(
                    "The old name is an alias. Notes and source files are preserved.",
                    p.soft,
                ))
                .child(
                    pressable(roles::META.wrap("Readable backup saved locally", p.faint))
                        .help(merge.backup.display().to_string()),
                )
                .child(
                    button("Undo merge", "project-undo", p)
                        .self_start()
                        .on_press(move |s: &mut State| s.undo_project_merge(&from, &id)),
                ),
        );
    }
    if let Some(from) = &s.project_merge.from {
        let mut card = col()
            .w_full()
            .gap(10.0)
            .px(14.0)
            .py(14.0)
            .radius(8.0)
            .border(1.0, p.line)
            .fill(p.card)
            .child(roles::TITLE.wrap(format!("Merge {from} into..."), p.ink));
        if let Some(preview) = &s.project_merge.preview {
            card = card.child(roles::NAME.wrap(format!("Keep {}", preview.into), p.ink))
                .child(roles::SMALL.wrap("The old name becomes an alias. A dated, readable backup is saved before applying. Undo remains available after restart.", p.soft));
            for (label, items) in [
                ("Meetings", &preview.meetings),
                ("Tasks", &preview.tasks),
                ("Memory items and proposals", &preview.memory),
            ] {
                card = card.child(roles::GROUP.wrap(format!("{label} ({})", items.len()), p.soft));
                for item in items {
                    card = card.child(roles::SMALL.wrap(item, p.ink));
                }
            }
            card = card.child(
                button("Confirm merge", "project-merge-confirm", p)
                    .self_start()
                    .on_press(|s: &mut State| s.confirm_project_merge()),
            );
        } else {
            card = card.child(roles::SMALL.wrap(
                "Choose the project to keep. Review the affected items before confirming.",
                p.soft,
            ));
            for into in &s.tasks.names {
                if into == from {
                    continue;
                }
                let into = into.clone();
                card = card.child(
                    button(&into, &format!("merge-target-{into}"), p)
                        .self_start()
                        .on_press(move |s: &mut State| s.preview_project_merge(&into)),
                );
            }
        }
        content = content.child(
            card.child(
                button("Cancel", "project-merge-cancel", p)
                    .self_start()
                    .on_press(|s: &mut State| s.project_merge = MergeUi::default()),
            ),
        );
    }
    if let Some(error) = &s.project_merge.error {
        content = content.child(roles::SMALL.wrap(error, p.warn).role("alert"));
    }
    Some(content)
}

pub fn scene(s: &mut State, action: &str) {
    s.go_home();
    let from = "Northwind Field Guide";
    let into = "Northwind Atlas";
    s.tasks.projects = s.tasks.store.add_project(from).unwrap();
    s.tasks.projects = s.tasks.store.add_project(into).unwrap();
    let id = s.meetings[0].id.clone();
    s.set_meeting_project(&id, from);
    s.filter = Filter::Project(from.into());
    s.refresh_list();
    if action != "page" {
        s.start_project_merge(from);
    }
    if action == "preview" || action == "done" {
        s.preview_project_merge(into);
        assert!(
            s.project_merge.preview.is_some(),
            "merge preview: {:?}",
            s.project_merge.error
        );
    }
    if action == "done" {
        s.confirm_project_merge();
        assert!(s.project_merge.error.is_none(), "merge failed");
        s.tasks.projects = s.tasks.store.load_projects().unwrap();
        assert_eq!(s.tasks.projects.resolve(from), into);
    }
}
