//! The To do section of the reading column. Ticks write the same file the Tasks view reads.
use super::{
    State, roles,
    theme::Palette,
    widgets::{label, pressable, stamp},
};
use rune::{El, TextStyle, col, el, row, text};
use sidenote::core::{Meeting, Todo, nice_date};

fn checkbox(task: &str, on: bool, index: usize, p: Palette) -> El {
    let task_owned = task.to_string();
    pressable(
        col()
            .size(18.0, 18.0)
            .radius(5.0)
            .center()
            .border(1.5, if on { p.ok } else { p.faint })
            .fill(if on { p.okbg } else { p.bg })
            .child(if on {
                label("\u{2713}", 12.0, 700.0, p.ok).font(TextStyle::new(12.0, 700.0).lh(14.0))
            } else {
                el()
            }),
    )
    .role("checkbox")
    .checked(on)
    .label(task)
    .key(format!("tick-{index}"))
    .mt(4.0)
    .on_press(move |s: &mut State| s.toggle_task(&task_owned))
}

fn item(s: &State, index: usize, t: &Todo, p: Palette) -> El {
    let on = sidenote::core::todo_done(t, Some(&s.saved.checked));
    let owner = t
        .owner
        .as_deref()
        .filter(|o| !o.is_empty())
        .unwrap_or("Owner to confirm");
    let mut meta = row()
        .flex_wrap()
        .items_center()
        .gaps(2.0, 6.0)
        .mt(2.0)
        .child(
            roles::Role {
                weight: 500.0,
                ..roles::META
            }
            .wrap(owner, p.soft),
        );
    if let Some(due) = t.due.as_deref().filter(|d| !d.is_empty()) {
        let default = if t.due_default { " (default)" } else { "" };
        meta = meta.child(roles::meta(
            format!("\u{b7} by {}{default}", nice_date(due)),
            p,
        ));
    }
    let key = format!("todo-{index}");
    let from = s.back.as_deref() == Some(key.as_str());
    if let Some(s) = stamp(&t.at, &key, p) {
        meta = meta.child(s);
    }
    let task = t.text().to_string();
    // Next steps read in bold, as the actions of the meeting; a finished one steps back.
    let task_text = text(t.title().to_string())
        .font(if on { roles::READ } else { roles::READ.bold() }.style())
        .color(if on { p.faint } else { p.ink })
        .wrap();
    let task_text = if on {
        task_text.strike(0, t.title().len(), None)
    } else {
        task_text
    };
    let mut r = row()
        .key(key)
        .w_full()
        .gap(12.0)
        .py(6.0)
        .pr(6.0)
        .radius(8.0)
        .child(
            col()
                .w(22.0)
                .items_center()
                .child(checkbox(t.text(), on, index, p)),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .child(
                    pressable(task_text)
                        .key(format!("task-{index}"))
                        .on_press(move |s: &mut State| s.toggle_task(&task)),
                )
                .child(meta),
        );
    if from {
        r = r.fill(p.hit);
    }
    r
}

pub fn section(s: &State, m: &Meeting, p: Palette) -> El {
    let mut out = col().key("todo-list").w_full().gap(2.0);
    for (i, t) in m.summary.todo.iter().enumerate() {
        out = out.child(item(s, i, t, p));
    }
    out.child(
        roles::meta(
            if s.config.features.tasks {
                "Ticks are saved on this computer and show in Tasks too."
            } else {
                "Ticks are saved on this computer."
            },
            p,
        )
        .pl(34.0)
        .pt(6.0),
    )
}
