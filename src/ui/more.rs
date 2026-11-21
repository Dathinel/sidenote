//! The "..." menu on a meeting page and on a Home row: copy the notes, the decisions or the
//! to-dos, open the meeting's folder, or export the meeting as Markdown.
//!
//! Copying uses the window's own clipboard request (`Tx::copy`). The folder opens through
//! File Explorer with the path as one argument, and an export is written into the app's
//! `exports` folder, never into a meeting folder, so a meeting from another device is only read.
use super::{State, theme::Palette};
use rune::{El, Tx};
use rune_kit::{Kit, Menu, MenuItem, Placement};
use sidenote::{
    core::{Meeting, export},
    platform,
};
use std::time::{Duration, Instant};

/// How long "Copied summary" stays on screen.
const TOAST_FOR: Duration = Duration::from_secs(2);

#[derive(Clone, Copy)]
pub(super) enum Part {
    Notes,
    Decisions,
    Todos,
}

impl Part {
    fn text(self, m: &Meeting) -> String {
        match self {
            Self::Notes => export::notes_markdown(m),
            Self::Decisions => export::decisions_markdown(m),
            Self::Todos => export::todos_markdown(m),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Notes => "summary",
            Self::Decisions => "decisions",
            Self::Todos => "tasks",
        }
    }
}

impl State {
    /// A short confirmation near the top of the window. `poll` takes it down after two seconds.
    pub fn show_toast(&mut self, text: impl Into<String>, tx: &mut Tx) {
        self.memory.settle_toast_action = None;
        self.toast = Some((text.into(), Instant::now() + TOAST_FOR));
        tx.wake_after(TOAST_FOR + Duration::from_millis(60));
    }

    /// Drops a toast that has run out. True when it did, so the window draws again.
    pub fn expire_toast(&mut self) -> bool {
        if self
            .toast
            .as_ref()
            .is_some_and(|(_, end)| Instant::now() >= *end)
        {
            self.toast = None;
            return true;
        }
        false
    }

    fn meeting_by_id(&self, id: &str) -> Option<&Meeting> {
        self.meetings.iter().find(|m| m.id == id)
    }

    /// Puts one part of a meeting on the clipboard as Markdown.
    pub(super) fn copy_part_of(&mut self, id: &str, part: Part, tx: &mut Tx) {
        self.more_menu = None;
        let Some(text) = self.meeting_by_id(id).map(|m| part.text(m)) else {
            return;
        };
        if text.trim().is_empty() {
            self.show_toast(format!("No {} to copy", part.name()), tx);
            return;
        }
        tx.copy(text);
        self.show_toast(format!("Copied {}", part.name()), tx);
    }

    /// Puts the whole transcript on the clipboard, as plain lines.
    pub fn copy_transcript(&mut self, id: &str, tx: &mut Tx) {
        let Some(text) = self.meeting_by_id(id).map(|meeting| {
            if self.transcript_original
                && let Some(routed) = &meeting.routed
            {
                let mut original = meeting.clone();
                original.lines = routed.original_lines();
                return export::transcript_text(&original);
            }
            export::transcript_text(meeting)
        }) else {
            return;
        };
        if text.trim().is_empty() {
            self.show_toast("No transcript to copy", tx);
            return;
        }
        tx.copy(text);
        self.show_toast("Copied the transcript", tx);
    }

    /// Opens the folder that holds the meeting's files.
    pub fn open_meeting_folder(&mut self, id: &str) {
        self.more_menu = None;
        let Some(folder) = self.meeting_by_id(id).map(|m| m.folder.clone()) else {
            return;
        };
        if let Err(error) = platform::open_folder(&folder) {
            self.notice = Some(format!("Could not open the meeting folder: {error}"));
        }
    }

    /// Writes this meeting, and only this one, as a Markdown file in the app's `exports`
    /// folder, then shows the file in File Explorer.
    pub fn export_meeting(&mut self, id: &str, tx: &mut Tx) {
        self.more_menu = None;
        let dir = self.store.root.join("exports");
        let Some(result) = self.meeting_by_id(id).map(|m| export::export_to(&dir, m)) else {
            return;
        };
        match result {
            Ok(path) => {
                // Showing the file is a courtesy; the export already worked without it.
                let _ = platform::reveal_file(&path);
                self.show_toast("Exported as Markdown", tx);
            }
            Err(error) => {
                self.notice = Some(format!("Could not export this meeting: {error}"));
            }
        }
    }
}

/// The "..." button and its menu for one meeting. `scope` tells the meeting page's button from
/// each Home row's, so every menu has its own key.
pub fn menu(s: &State, k: &Kit, m: &Meeting, scope: &str, p: Palette, layer: &mut Vec<El>) -> El {
    let key = format!("more-{scope}-{}", m.id);
    let open = s.more_menu.as_deref() == Some(key.as_str());
    let toggle = key.clone();
    let trigger = if scope == "row" {
        super::widgets::quiet_button("More", &format!("{key}-button"), p)
    } else {
        let button = super::widgets::icon_button(
            &super::icons::MORE,
            "More actions",
            &format!("{key}-button"),
            super::widgets::ButtonRole::Ghost,
            p,
        );
        if scope == "meeting" {
            button.size(28.0, 28.0).px(6.0)
        } else {
            button
        }
    }
    .on_press(move |s: &mut State| {
        let same = s.more_menu.as_deref() == Some(toggle.as_str());
        s.more_menu = (!same).then(|| toggle.clone());
    });
    let mut menu = Menu::new(key)
        .trigger(trigger)
        .open(open)
        .placement(Placement::BottomEnd)
        .min_width(250.0);
    if open {
        let has_decisions = !export::decisions_markdown(m).is_empty();
        let has_todos = !m.summary.todo.is_empty();
        menu = menu.on_close(|s: &mut State| s.more_menu = None);
        let copy = |label_: &str, part: Part, enabled: bool, none: &str| {
            let id = m.id.clone();
            let item = MenuItem::new(label_.to_string())
                .disabled(!enabled)
                .on_select_tx(move |s: &mut State, tx| s.copy_part_of(&id, part, tx));
            if enabled {
                item
            } else {
                item.description(none.to_string())
            }
        };
        let folder_id = m.id.clone();
        let export_id = m.id.clone();
        menu = menu
            .item(copy("Copy summary", Part::Notes, true, ""))
            .item(copy(
                "Copy decisions",
                Part::Decisions,
                has_decisions,
                "None in this meeting",
            ))
            .item(copy(
                "Copy tasks",
                Part::Todos,
                has_todos,
                "None in this meeting",
            ))
            .separator()
            .item(
                MenuItem::new("Open meeting folder")
                    .on_select(move |s: &mut State| s.open_meeting_folder(&folder_id)),
            )
            .item(
                MenuItem::new("Export this meeting (Markdown)")
                    .description("Summary and your notes, no transcript")
                    .on_select_tx(move |s: &mut State, tx| s.export_meeting(&export_id, tx)),
            );
        if scope == "meeting" {
            menu = super::meeting::reading::menu_items(s, m, menu);
        }
    }
    menu.build(k, layer)
}
