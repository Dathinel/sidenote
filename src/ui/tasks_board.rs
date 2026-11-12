//! The board layout of the Tasks page: four columns over the same tasks, one card each. Every
//! way of moving a card (the Move menu, the keyboard) ends in `State::move_card`, which saves
//! through the same status edit the task panel uses. The page and the panel are `tasks.rs`;
//! this file only lays the cards out. It is a child of that module, so it reads its helpers.
use super::{
    Panel, RowId, State, avatar, button, cards, chevron, date_chip, empty_note, owner_name,
    pressable, project_dot, roles, shown_title, status_dot,
};
use crate::ui::{edit_worker::Job, theme::Palette};
use rune::{El, Key, KeyEvent, Tx, col, row};
use rune_kit::{Kit, Menu, MenuItem, Placement, SegmentedControl};
use sidenote::core::{self, BoardColumn, BoardView, TaskItem};
use std::time::{Duration, Instant};

fn row_of(t: &TaskItem) -> RowId {
    (t.meeting_id.clone(), t.index)
}

/// Narrowest a column can be before the board falls back to one column at a time.
const COLUMN_MIN: f32 = 220.0;
const GAP: f32 = 12.0;
/// The list width the four columns need, side padding included.
pub const WIDE_MIN: f32 = 4.0 * COLUMN_MIN + 3.0 * GAP + 48.0;
/// Cards drawn per column before "Show more". The page scrolls as one, so this bounds the work.
const PAGE: usize = 25;
const DONE_PAGE: usize = 10;
const LIVE_FOR: Duration = Duration::from_secs(4);

pub fn first_page(c: BoardColumn) -> usize {
    if c == BoardColumn::Done {
        DONE_PAGE
    } else {
        PAGE
    }
}

impl State {
    /// The task as it will be once every save still waiting has run.
    fn effective(&self, t: &TaskItem) -> TaskItem {
        let mut eff = t.clone();
        match self.tasks.board_pending.get(&row_of(t)) {
            Some(BoardColumn::Done) => eff.done = true,
            Some(c) => {
                eff.done = false;
                if let Some(status) = c.status() {
                    eff.status = status;
                }
            }
            None => {}
        }
        eff
    }

    /// Sets the done tick. It is a set, not a flip: nothing is saved when the card already is
    /// that way, so two quick clicks cannot cancel each other or run twice.
    fn set_done(&mut self, t: &TaskItem, on: bool) {
        if t.done == on {
            return;
        }
        let job = if t.manual {
            Job::ManualTick {
                store: self.tasks.store.clone(),
                id: t
                    .meeting_id
                    .strip_prefix("manual/")
                    .unwrap_or(&t.meeting_id)
                    .into(),
            }
        } else {
            Job::Tick {
                store: self.store.clone(),
                id: t.meeting_id.clone(),
                task: t.task.clone(),
                stable_id: t.stable_id.clone(),
                on,
            }
        };
        self.publish_edit(job);
    }

    /// Takes a finished task out of Done. The status left behind comes back with it.
    fn reopen(&mut self, t: &TaskItem) {
        let title = shown_title(self, t).to_string();
        let mut open = t.clone();
        open.done = false;
        self.set_done(t, false);
        self.tasks
            .board_pending
            .insert(row_of(t), BoardColumn::of(&open));
        self.say(format!("Marked \u{201c}{title}\u{201d} not done"));
    }

    /// Moves one card to `target`: the status edit the panel makes, or the done tick.
    pub(super) fn move_card(&mut self, id: &str, index: usize, target: BoardColumn) {
        self.tasks.board_menu = None;
        let Some(t) = self.tasks.item(id, index).cloned() else {
            return;
        };
        let eff = self.effective(&t);
        let plan = core::move_plan(&eff, target);
        self.tasks.cursor = Some((id.to_string(), index));
        let title = shown_title(self, &t).to_string();
        if plan.is_noop() {
            self.say(format!(
                "\u{201c}{title}\u{201d} is already in {}",
                target.label()
            ));
            return;
        }
        // Out of Done is the tick going back, which restores the status left behind. The card
        // lands in that column, so the line says "not done" rather than naming one.
        if plan.tick == Some(false) {
            return self.reopen(&eff);
        }
        if let Some(on) = plan.tick {
            self.set_done(&eff, on);
        } else if let Some(status) = plan.status {
            self.edit_task_v2(id, index, core::TaskV2Edit::Status(status, None));
        }
        self.tasks.board_pending.insert(row_of(&t), target);
        self.say(format!(
            "Moved \u{201c}{title}\u{201d} to {}",
            target.label()
        ));
    }

    /// The tick, from the card's checkbox or the menu.
    fn toggle_done(&mut self, id: &str, index: usize) {
        self.tasks.board_menu = None;
        let Some(t) = self.tasks.item(id, index).cloned() else {
            return;
        };
        self.tasks.cursor = Some((id.to_string(), index));
        let eff = self.effective(&t);
        if eff.done {
            self.reopen(&eff);
        } else {
            self.move_card(id, index, BoardColumn::Done);
        }
    }

    /// A short polite line under the toolbar. It goes after a few seconds or the next key.
    pub(super) fn say(&mut self, line: String) {
        self.tasks.board_live = Some((line, Instant::now() + LIVE_FOR));
    }

    /// Shift+Left and Shift+Right: the card under the cursor one column over.
    fn move_cursor_card(&mut self, delta: isize) {
        let Some((id, index)) = self.cursor_row() else {
            return;
        };
        let Some(from) = self.tasks.item(&id, index).map(BoardColumn::of) else {
            return;
        };
        match from.neighbour(delta) {
            Some(to) => self.move_card(&id, index, to),
            None => self.say(format!(
                "Nothing {} of {}",
                if delta < 0 { "left" } else { "right" },
                from.label()
            )),
        }
    }

    /// Left and Right: the cursor to the nearest card in the next column that has one.
    fn step_cursor_column(&mut self, delta: isize, tx: &mut Tx) {
        let view = self.tasks.board();
        let at = self.cursor_row().and_then(|(id, index)| {
            let i = self
                .tasks
                .items
                .iter()
                .position(|t| t.meeting_id == id && t.index == index)?;
            BoardColumn::ALL
                .into_iter()
                .find_map(|c| view.column(c).iter().position(|&r| r == i).map(|n| (c, n)))
        });
        let mut column = at.map_or(BoardColumn::Todo, |(c, _)| c);
        let row = at.map_or(0, |(_, n)| n);
        loop {
            let Some(next) = (if at.is_none() {
                Some(column)
            } else {
                column.neighbour(delta)
            }) else {
                return;
            };
            column = next;
            let cards = view.column(column);
            let hidden = column == BoardColumn::Done && view.done_folded && !self.tasks.done_open;
            if !cards.is_empty() && !hidden {
                let i = cards[row.min(cards.len() - 1)];
                let (id, index) = (
                    self.tasks.items[i].meeting_id.clone(),
                    self.tasks.items[i].index,
                );
                self.tasks.board_tab = column;
                self.tasks.keyboard = true;
                if matches!(self.tasks.panel, Some(Panel::Task(_))) && !self.open_task(&id, index) {
                    return;
                }
                tx.scroll_into_view(format!("trow-{id}-{index}"));
                self.tasks.cursor = Some((id, index));
                return;
            }
            if at.is_none() {
                column = column.neighbour(1).unwrap_or(column);
                if column == BoardColumn::Done {
                    return;
                }
            }
        }
    }
}

/// The board's own keys, tried before the list's. Returns whether the key was used.
pub fn on_key(s: &mut State, e: &KeyEvent, tx: &mut Tx) -> bool {
    let shift = e.mods.shift;
    match &e.key {
        Key::Left | Key::Right => {
            let delta = if e.key == Key::Left { -1 } else { 1 };
            if shift {
                s.move_cursor_card(delta);
            } else {
                s.step_cursor_column(delta, tx);
            }
            true
        }
        Key::Text(t) => match t.as_str() {
            "[" => {
                s.move_cursor_card(-1);
                true
            }
            "]" => {
                s.move_cursor_card(1);
                true
            }
            "h" => {
                s.step_cursor_column(-1, tx);
                true
            }
            "l" => {
                s.step_cursor_column(1, tx);
                true
            }
            "m" => match s.cursor_row() {
                Some(row) if s.tasks.keyboard => {
                    s.tasks.board_menu = Some(row);
                    true
                }
                _ => false,
            },
            _ => false,
        },
        _ => false,
    }
}

/// Clears the live line when its time is up. Returns whether the page changed.
pub fn live_tick(s: &mut State, tx: &mut Tx) -> bool {
    let Some((_, until)) = &s.tasks.board_live else {
        return false;
    };
    let left = until.saturating_duration_since(Instant::now());
    if left.is_zero() {
        s.tasks.board_live = None;
        return true;
    }
    tx.wake_after(left);
    false
}

fn area_label(s: &State, t: &TaskItem) -> String {
    t.area_id
        .as_ref()
        .and_then(|id| s.areas.catalog.areas.get(id))
        .map(|area| area.name.clone())
        .unwrap_or_else(|| {
            s.areas
                .project_labels
                .get(&t.project)
                .cloned()
                .unwrap_or_else(|| t.project.clone())
        })
}

fn column_icon(c: BoardColumn) -> &'static rune::Icon {
    match c {
        BoardColumn::Todo => &crate::ui::icons::CIRCLE,
        BoardColumn::Waiting => &crate::ui::icons::WAITING,
        BoardColumn::Review => &crate::ui::icons::EYE,
        BoardColumn::Done => &crate::ui::icons::DONE,
    }
}

/// The Move menu: the same choices the task panel's status menu has, for one card. It is the
/// path for a screen reader, for narrow windows and for anyone who does not drag.
fn move_menu(s: &State, k: &Kit, t: &TaskItem, p: Palette, layer: &mut Vec<El>) -> El {
    let (id, index) = (t.meeting_id.clone(), t.index);
    let me: RowId = (id.clone(), index);
    let open = s.tasks.board_menu.as_ref() == Some(&me);
    let here = BoardColumn::of(t);
    let toggle = {
        let me = me.clone();
        move |s: &mut State| {
            let on = s.tasks.board_menu.as_ref() != Some(&me);
            s.tasks.close_menus();
            s.tasks.board_menu = on.then(|| me.clone());
        }
    };
    let trigger = crate::ui::widgets::button(
        "Move",
        &format!("tmove-{id}-{index}"),
        crate::ui::widgets::ButtonRole::Secondary,
        p,
    )
    .label(format!(
        "Move task: {}. Now in {}.",
        shown_title(s, t),
        here.label()
    ))
    .h(28.0)
    .self_start()
    .on_press(toggle);
    let mut menu = Menu::new(format!("tmove-menu-{id}-{index}"))
        .trigger(trigger)
        .open(open)
        .placement(Placement::BottomEnd)
        .min_width(180.0);
    if open {
        menu = menu.on_close(|s: &mut State| s.tasks.board_menu = None);
        for c in BoardColumn::ALL
            .into_iter()
            .filter(|c| c.status().is_some())
        {
            let id = id.clone();
            menu = menu.item(
                MenuItem::new(c.label())
                    .checked(here == c)
                    .on_select(move |s: &mut State| s.move_card(&id, index, c)),
            );
        }
        let id = id.clone();
        menu = menu.separator().item(
            MenuItem::new(if t.done { "Mark not done" } else { "Mark done" })
                .on_select(move |s: &mut State| s.toggle_done(&id, index)),
        );
    }
    menu.build(k, layer)
}

fn card(s: &State, k: &Kit, t: &TaskItem, p: Palette, layer: &mut Vec<El>) -> El {
    let (id, index) = (t.meeting_id.clone(), t.index);
    let me: RowId = (id.clone(), index);
    let tasks = &s.tasks;
    let on = t.done;
    let selected = matches!(&tasks.panel, Some(Panel::Task(r)) if *r == me);
    let at_cursor = tasks.keyboard && tasks.cursor.as_ref() == Some(&me) && !selected;
    let tick = {
        let id = id.clone();
        pressable(status_dot(on, 22.0, p))
            .role("checkbox")
            .checked(on)
            .label(t.task.clone())
            .key(format!("ttick-{id}-{index}"))
            .radius(11.0)
            .on_press(move |s: &mut State| s.toggle_done(&id, index))
    };
    let title = if on {
        let shown = shown_title(s, t);
        roles::SMALL
            .wrap(shown, p.faint)
            .max_lines(3)
            .strike(0, shown.len(), None)
    } else {
        roles::SMALL
            .bold()
            .wrap(shown_title(s, t), p.ink)
            .max_lines(3)
    };
    let open_title = {
        let id = id.clone();
        pressable(
            col().min_w(0.0).gap(2.0).child(title).children(
                (t.status == core::TaskStatus::Waiting && !on)
                    .then(|| t.waiting_label.clone())
                    .flatten()
                    .map(|w| roles::META.wrap(format!("Waiting on: {w}"), p.soft)),
            ),
        )
        .key(format!("ttask-{id}-{index}"))
        .label(format!("Open the task: {}", t.task))
        .grow(1.0)
        .min_w(0.0)
        .on_press(move |s: &mut State| {
            s.open_task(&id, index);
        })
    };
    let owner = match owner_name(t) {
        Some((name, me)) => row()
            .gap(6.0)
            .items_center()
            .min_w(0.0)
            .child(avatar(&name, me, 20.0, p))
            .child(roles::META.line(name, p.soft)),
        None => row()
            .min_w(0.0)
            .child(roles::META.line("No owner", p.faint)),
    };
    let when = date_chip(t, tasks.today, p);
    let source = if t.manual {
        roles::META.line("Added by hand", p.faint)
    } else {
        let (id, stamp) = (id.clone(), t.stamp.clone());
        pressable(roles::META.line(format!("{} \u{203a}", t.meeting_title), p.soft))
            .key(format!("tsource-{id}-{index}"))
            .label(format!("Open meeting: {}", t.meeting_title))
            .min_w(0.0)
            .on_press_tx(move |s: &mut State, tx| match &stamp {
                Some(stamp) => s.open_meeting_at(&id, index, stamp, tx),
                None => s.open_meeting(&id, tx),
            })
    };
    let area = row()
        .gap(5.0)
        .items_center()
        .min_w(0.0)
        .label(format!("Project: {}", t.project))
        .child(project_dot(&t.project, p))
        .child(roles::META.line(area_label(s, t), p.faint));
    let conflicts = s
        .relay_ui
        .conflicts
        .iter()
        .filter(|c| {
            Some(c.task_id.strip_prefix("$manual:").unwrap_or(&c.task_id))
                == id.strip_prefix("manual/")
        })
        .map(|c| c.choices.len())
        .max();
    let toggle = (!t.subtasks.is_empty()).then(|| {
        let expanded = tasks.expanded_subtasks.contains(&me);
        let row_id = me.clone();
        pressable(
            row()
                .gap(4.0)
                .items_center()
                .child(chevron(expanded, p))
                .child(roles::META.line(format!("Subtasks {}", t.progress()), p.soft)),
        )
        .key(format!("subtasks-toggle-{id}-{index}"))
        .label(format!(
            "{} subtasks: {}",
            if expanded { "Hide" } else { "Show" },
            t.title
        ))
        .expanded(expanded)
        .on_press(move |s: &mut State| {
            if !s.tasks.expanded_subtasks.remove(&row_id) {
                s.tasks.expanded_subtasks.insert(row_id.clone());
            }
        })
    });
    let mut body = col()
        .w_full()
        .min_w(0.0)
        .gap(6.0)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .items_start()
                .child(tick)
                .child(open_title),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .items_center()
                .justify_between()
                .child(owner.grow(1.0))
                .children(when),
        )
        .child(source)
        .child(area);
    if let Some(count) = conflicts {
        body = body.child(
            roles::SMALL
                .wrap(format!("{count} versions"), p.warn)
                .role("status"),
        );
    }
    body = body.child(
        row()
            .w_full()
            .items_center()
            .justify_between()
            .gap(8.0)
            .child(row().grow(1.0).min_w(0.0).children(toggle))
            .child(move_menu(s, k, t, p, layer)),
    );
    for sub in t
        .subtasks
        .iter()
        .filter(|_| tasks.expanded_subtasks.contains(&me))
    {
        let (id, sub_id, done) = (id.clone(), sub.id.clone(), sub.done);
        body = body.child(
            pressable(
                row()
                    .w_full()
                    .gap(8.0)
                    .items_center()
                    .child(status_dot(done, 18.0, p))
                    .child(
                        roles::META
                            .wrap(&sub.title, if done { p.faint } else { p.soft })
                            .max_lines(2)
                            .grow(1.0)
                            .min_w(0.0),
                    ),
            )
            .key(format!("subtick-{id}-{index}-{sub_id}"))
            .label(format!("Subtask: {}", sub.title))
            .role("checkbox")
            .checked(done)
            .radius(5.0)
            .hover(p.chip)
            .on_press(move |s: &mut State| {
                let stable = s
                    .tasks
                    .item(&id, index)
                    .map(|t| t.stable_id.clone())
                    .unwrap_or_default();
                s.edit_task_v2(
                    &id,
                    index,
                    core::TaskV2Edit::SubtaskTick {
                        key: core::subtask_tick_key(&stable, &sub_id),
                        done: !done,
                    },
                )
            }),
        );
    }
    let mut shell = cards::panel(p)
        .key(format!("trow-{id}-{index}"))
        // When the card changes column it is rebuilt elsewhere. Focus follows it by key.
        .focus_return(format!("ttask-{id}-{index}"))
        .padding(11.0)
        .child(body);
    if selected {
        shell = shell.fill(p.sel).border(1.0, p.accent);
    } else if at_cursor {
        shell = shell.outline(1.0, 0.0, p.accent);
    }
    shell
}

fn empty_column(p: Palette) -> El {
    col()
        .w_full()
        .px(10.0)
        .py(14.0)
        .radius(10.0)
        .border(1.0, p.line)
        .child(roles::META.wrap(
            "Nothing here. Use Move on a card to bring it here.",
            p.faint,
        ))
}

/// The cards of one column, a page at a time.
fn column_cards(
    s: &State,
    k: &Kit,
    view: &BoardView,
    c: BoardColumn,
    p: Palette,
    layer: &mut Vec<El>,
) -> El {
    let rows = view.column(c);
    let tasks = &s.tasks;
    let mut out = col().w_full().min_w(0.0).gap(8.0);
    if c == BoardColumn::Done && view.done_folded && !tasks.done_open {
        if !rows.is_empty() {
            out = out.child(
                button(
                    format!("Show {} done", rows.len()),
                    "tasks-board-done-show",
                    false,
                    p,
                )
                .label(format!("Show {} done tasks", rows.len()))
                .on_press(|s: &mut State| s.tasks.done_open = true),
            );
        }
        return if rows.is_empty() {
            out.child(empty_column(p))
        } else {
            out
        };
    }
    if rows.is_empty() {
        return out.child(empty_column(p));
    }
    let shown = tasks.board_shown[c.index()].max(first_page(c));
    for &i in rows.iter().take(shown) {
        out = out.child(card(s, k, &tasks.items[i], p, layer));
    }
    if rows.len() > shown {
        let more = (rows.len() - shown).min(PAGE);
        out = out.child(
            button(
                format!("Show {more} more"),
                format!("tasks-board-more-{}", c.index()),
                false,
                p,
            )
            .label(format!("Show {more} more in {}", c.label()))
            .on_press(move |s: &mut State| {
                let at = &mut s.tasks.board_shown[c.index()];
                *at = (*at).max(first_page(c)) + PAGE;
            }),
        );
    }
    out
}

fn heading(view: &BoardView, c: BoardColumn, s: &State, p: Palette) -> El {
    let mut r = row()
        .w_full()
        .gap(8.0)
        .items_center()
        .child(crate::ui::icons::draw(column_icon(c), 16.0, p.soft))
        .child(roles::GROUP.line(c.label(), p.ink).grow(1.0))
        .child(roles::META.line(view.column(c).len().to_string(), p.soft));
    if c == BoardColumn::Done && view.done_folded && s.tasks.done_open {
        r = r.child(
            pressable(roles::META.line("Hide", p.soft))
                .key("tasks-board-done-hide")
                .label("Hide the done tasks")
                .on_press(|s: &mut State| s.tasks.done_open = false),
        );
    }
    r.label(format!("{}, {} tasks", c.label(), view.column(c).len()))
        .key(format!("tasks-board-heading-{}", c.index()))
}

/// The whole board for one list width: four columns when they fit, otherwise one at a time
/// under four tabs.
pub fn board(s: &State, k: &Kit, p: Palette, list_w: f32, side: f32, layer: &mut Vec<El>) -> El {
    let view = s.tasks.board();
    if view.columns.iter().all(Vec::is_empty) {
        return empty_note(s, p);
    }
    if list_w >= WIDE_MIN {
        let avail = list_w - 2.0 * side - 3.0 * GAP;
        let width = (avail / 4.0).max(COLUMN_MIN);
        let mut out = row()
            .key("tasks-board")
            .gap(GAP)
            .items_start()
            .role("group")
            .label("Tasks board");
        for c in BoardColumn::ALL {
            out = out.child(
                col()
                    .key(format!("tasks-board-column-{}", c.index()))
                    .role("group")
                    .label(c.label())
                    .w(width)
                    .gap(8.0)
                    .padding(8.0)
                    .radius(12.0)
                    .fill(p.chip.alpha(0.5))
                    .child(heading(&view, c, s, p))
                    .child(column_cards(s, k, &view, c, p, layer)),
            );
        }
        return out;
    }
    let tab = s.tasks.board_tab;
    let labels: Vec<String> = BoardColumn::ALL
        .iter()
        .map(|c| format!("{} {}", c.short_label(), view.column(*c).len()))
        .collect();
    let names: Vec<&str> = labels.iter().map(String::as_str).collect();
    let mut selected = k.clone();
    selected.c.bg_base = p.sel;
    selected.c.border_strong = p.accent;
    col()
        .key("tasks-board")
        .w_full()
        .gap(10.0)
        .child(
            SegmentedControl::new(&names, Some(tab.index()))
                .label("Board column")
                .width(Some((list_w - 2.0 * side).max(240.0)))
                .on_pick::<State>(|s, index, _| {
                    if let Some(c) = BoardColumn::ALL.get(index) {
                        s.tasks.board_tab = *c;
                    }
                })
                .build(&selected)
                .key("tasks-board-tabs"),
        )
        .child(heading(&view, tab, s, p))
        .child(column_cards(s, k, &view, tab, p, layer))
}

/// The polite line under the toolbar.
pub fn live_line(s: &State, p: Palette) -> Option<El> {
    let (line, _) = s.tasks.board_live.as_ref()?;
    Some(
        roles::SMALL
            .wrap(line, p.soft)
            .role("status")
            .key("tasks-board-live")
            .mb(8.0),
    )
}

/// The List / Board switch in the toolbar.
pub fn layout_toggle(s: &State, k: &Kit, p: Palette) -> El {
    let mut selected = k.clone();
    selected.c.bg_base = p.sel;
    selected.c.border_strong = p.accent;
    SegmentedControl::new(
        &["List", "Board"],
        Some(usize::from(s.tasks.layout == core::TasksLayout::Board)),
    )
    .label("Tasks layout")
    .width(Some(132.0))
    .on_pick::<State>(|s, index, _| {
        s.set_layout(if index == 0 {
            core::TasksLayout::List
        } else {
            core::TasksLayout::Board
        })
    })
    .build(&selected)
    .key("tasks-layout")
}
