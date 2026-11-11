//! Automatic settlements are distinct from the owner's memory decisions.
use super::*;
use crate::ui::{cards, icons};
use sidenote::core::{self, settle};

impl State {
    pub fn settle_undo(&mut self, id: String, redo: bool) {
        if self.memory.settle_busy {
            return;
        }
        self.memory.settle_busy = true;
        self.memory.errors.remove("settle");
        let events = self.events_tx.clone();
        let waker = self.waker.clone();
        std::thread::spawn(move || {
            let result = settle::Store::default().undo(&id).map_err(|e| {
                core::validation_message(&e)
                    .unwrap_or("Could not restore this change. Try again.")
                    .to_string()
            });
            let _ = events.send(crate::ui::Event::Settled { redo, result });
            if let Some(waker) = waker {
                waker();
            }
        });
    }
    pub fn settle_finished(&mut self, redo: bool, result: Result<settle::Entry, String>) {
        self.memory.settle_busy = false;
        match result {
            Ok(entry) => {
                self.memory.settle_pending_toast = Some((
                    if redo {
                        "Change restored."
                    } else {
                        "Change undone."
                    }
                    .into(),
                    entry.id,
                    !redo,
                ));
                self.refresh_memory(false);
                self.request_refresh();
            }
            Err(error) => {
                self.memory.errors.insert("settle".into(), error);
            }
        }
    }
}
fn rows(s: &State, history: Vec<&settle::Row>, p: Palette) -> El {
    let mut card = cards::panel(p).key("settle-card");
    for (i, row) in history.into_iter().enumerate() {
        if i > 0 {
            card = card.child(hairline(p));
        }
        let entry = &row.entry;
        let title = s
            .meetings
            .iter()
            .find(|m| m.id == entry.source_meeting_id)
            .map(|m| m.title.clone())
            .unwrap_or_else(|| "Source meeting".into());
        let source = match &entry.source_offset {
            Some(at) => format!("{title} at {at}"),
            None => title,
        };
        let target = match &entry.source_offset {
            Some(at) => format!("{}@{at}", entry.source_meeting_id),
            None => entry.source_meeting_id.clone(),
        };
        let origin = format!("settled-{}", entry.id);
        let from = origin.clone();
        let when = DateTime::parse_from_rfc3339(&entry.time)
            .map(|at| at.with_timezone(&Local).format("%-d %b, %H:%M").to_string())
            .unwrap_or_default();
        let action = row.undo_target.clone();
        let redo = row.undone;
        card = card.child(
            cards::row_shell()
                .key(origin)
                .items_start()
                .child(cards::icon_tile(&icons::FLAG, p))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(4.0)
                        .child(roles::NAME.wrap(&entry.label, p.ink).max_lines(2))
                        .child(roles::SMALL.wrap(&entry.reason, p.soft))
                        .child(
                            pressable(roles::META.line(source, p.accent))
                                .key(format!("settle-source-{i}"))
                                .self_start()
                                .on_press_tx(move |s: &mut State, tx| {
                                    s.open_check_citation(&target, from.clone(), tx)
                                }),
                        )
                        .child(roles::META.wrap(
                            format!("{when}{}", if row.undone { " (undone)" } else { "" }),
                            p.faint,
                        )),
                )
                .child(
                    button(
                        if redo { "Redo" } else { "Undo" },
                        format!("settle-undo-{i}"),
                        false,
                        p,
                    )
                    .on_press(move |s: &mut State| s.settle_undo(action.clone(), redo)),
                ),
        );
    }
    card
}
fn error(s: &State, p: Palette) -> Option<El> {
    s.memory
        .errors
        .get("settle")
        .map(|e| roles::SMALL.wrap(e, p.warn).role("alert"))
}
pub fn section(s: &State, p: Palette) -> El {
    let recent: Vec<_> = s
        .memory
        .settled
        .iter()
        .filter(|row| {
            DateTime::parse_from_rfc3339(&row.entry.time).is_ok_and(|at| {
                let age = Local::now().signed_duration_since(at);
                age >= chrono::Duration::zero() && age <= chrono::Duration::days(7)
            })
        })
        .collect();
    if recent.is_empty() {
        return el();
    }
    col()
        .w_full()
        .gap(10.0)
        .mt(18.0)
        .key("recently-settled")
        .child(
            row()
                .w_full()
                .items_center()
                .justify_between()
                .child(roles::GROUP.wrap("Recently settled by Sidenote", p.soft))
                .child(
                    button("See all", "settle-see-all", false, p).on_press(|s: &mut State| {
                        s.memory.settle_all = true;
                        s.memory.settle_meeting = None;
                    }),
                ),
        )
        .children(error(s, p))
        .child(rows(s, recent.into_iter().take(5).collect(), p))
        .child(roles::META.wrap("The last 7 days.", p.faint))
}
pub fn all(s: &State, p: Palette, narrow: bool) -> El {
    let history = s
        .memory
        .settled
        .iter()
        .filter(|row| {
            s.memory
                .settle_meeting
                .as_ref()
                .is_none_or(|id| &row.entry.source_meeting_id == id)
        })
        .collect();
    scroll(
        col()
            .w_full()
            .max_w(940.0)
            .px(if narrow { 16.0 } else { 40.0 })
            .pt(22.0)
            .pb(40.0)
            .gap(14.0)
            .child(
                row()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .child(text_title("Settled by Sidenote", 30.0, p))
                    .child(button("Back to Memory", "settle-back", false, p).on_press(
                        |s: &mut State| {
                            s.memory.settle_all = false;
                            s.memory.settle_meeting = None;
                        },
                    )),
            )
            .children(error(s, p))
            .child(rows(s, history, p)),
        "settle-all-scroll",
    )
}
/// The current meeting view and the dev.13 Reading rail share this count and navigation.
pub fn meeting_footer(s: &State, m: &core::Meeting, p: Palette) -> El {
    if !s.config.features.memory {
        return el();
    }
    let count = s
        .memory
        .settled
        .iter()
        .filter(|row| row.entry.source_meeting_id == m.id && !row.undone)
        .count();
    let id = m.id.clone();
    col()
        .w_full()
        .gap(4.0)
        .mt(18.0)
        .child(roles::META.wrap(
            format!(
                "Sidenote updated {count} {} from this meeting.",
                if count == 1 { "thing" } else { "things" }
            ),
            p.soft,
        ))
        .child(
            crate::ui::widgets::link("See in Memory", p)
                .key("meeting-settle-memory")
                .self_start()
                .on_press(move |s: &mut State| {
                    s.open_memory();
                    s.memory.settle_all = true;
                    s.memory.settle_meeting = Some(id.clone());
                }),
        )
}

/// Uses the same write-ahead entry points as the notes worker, only with invented fixture data.
pub fn scene(s: &mut State) {
    assert!(crate::ui::isolated(), "Settle scenes require isolated data");
    crate::ui::hex::scene(s, false);
    crate::ui::sync_scene::settle(s);
    let meeting = s
        .meetings
        .iter()
        .find(|m| m.id == "1800000000000-1")
        .expect("Settle fixture meeting");
    let id = meeting.id.clone();
    let title = meeting.title.clone();
    let folder = meeting.folder.clone();
    let store = settle::Store::new(s.store.root.clone(), folder.parent().unwrap().to_owned());
    if store.history().unwrap().is_empty() {
        let mut summary: serde_json::Value =
            serde_json::from_slice(&std::fs::read(folder.join("english-summary.json")).unwrap())
                .unwrap();
        let tasks = summary["todo"].as_array_mut().unwrap();
        for (i, task) in tasks.iter_mut().enumerate() {
            task["owner"] = serde_json::json!(if i % 2 == 0 {
                "Alex Chen"
            } else {
                "Morgan Lee"
            });
            task["due"] = serde_json::json!("2026-11-13");
        }
        let tags: Vec<_> = (0..tasks.len())
            .map(|i| serde_json::json!({"i":i,"project":"Northwind Atlas","mine":true}))
            .collect();
        store.publish_summary(&id, &summary, false).unwrap();
        store
            .publish_projects(
                &id,
                &title,
                &serde_json::json!({"meeting_project":"Northwind Atlas","tasks":tags}),
            )
            .unwrap();
    }
    s.open_memory();
    crate::ui::sync_scene::settle(s);
}
