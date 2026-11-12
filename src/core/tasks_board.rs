//! The Tasks board: four columns drawn over the tasks the list already shows. A column is not a
//! stored value. To do, Waiting on and In review are the existing `TaskStatus`; Done is the
//! existing tick. Nothing here adds a status or touches a sync file.
use super::{
    DoneFilter, EditView, Overrides, Saved, StateStore, TaskFilter, TaskItem, TaskStatus,
    TaskStore, TaskV2Edit,
};
use std::{io, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BoardColumn {
    Todo,
    Waiting,
    Review,
    Done,
}

impl BoardColumn {
    pub const ALL: [Self; 4] = [Self::Todo, Self::Waiting, Self::Review, Self::Done];

    /// The full name, as the status menu and the column heading say it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Todo => "To do",
            Self::Waiting => "Waiting on",
            Self::Review => "In review",
            Self::Done => "Done",
        }
    }

    /// Short enough for four tabs across a phone-width window.
    pub fn short_label(self) -> &'static str {
        match self {
            Self::Todo => "To do",
            Self::Waiting => "Waiting",
            Self::Review => "Review",
            Self::Done => "Done",
        }
    }

    /// Where a task sits: Done wins over whatever status is left behind.
    pub fn of(t: &TaskItem) -> Self {
        if t.done {
            return Self::Done;
        }
        match t.status {
            TaskStatus::Todo => Self::Todo,
            TaskStatus::Waiting => Self::Waiting,
            TaskStatus::Review => Self::Review,
        }
    }

    /// The status a card takes when moved here. Done has none: it is a tick.
    pub fn status(self) -> Option<TaskStatus> {
        match self {
            Self::Todo => Some(TaskStatus::Todo),
            Self::Waiting => Some(TaskStatus::Waiting),
            Self::Review => Some(TaskStatus::Review),
            Self::Done => None,
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }

    /// The column `delta` places over, or None past either end. Cards do not wrap.
    pub fn neighbour(self, delta: isize) -> Option<Self> {
        let at = self.index() as isize + delta;
        usize::try_from(at)
            .ok()
            .and_then(|i| Self::ALL.get(i).copied())
    }
}

/// The columns of one board, as indexes into the task slice.
#[derive(Clone, Debug, Default)]
pub struct BoardView {
    pub columns: [Vec<usize>; 4],
    /// With the Open filter the Done column shows a count and waits for "Show".
    pub done_folded: bool,
}

impl BoardView {
    pub fn column(&self, c: BoardColumn) -> &[usize] {
        &self.columns[c.index()]
    }

    /// Every card the keyboard can reach, column by column. The Done column counts only while
    /// it is open.
    pub fn rows(&self, done_open: bool) -> Vec<usize> {
        let mut out = Vec::new();
        for c in BoardColumn::ALL {
            if c == BoardColumn::Done && self.done_folded && !done_open {
                continue;
            }
            out.extend(&self.columns[c.index()]);
        }
        out
    }
}

/// Lays the tasks that pass `filter` into the four columns. Every column is always present, so
/// an empty one can be drawn. Mine come first in a column, then by date, then in meeting order.
pub fn board_columns(tasks: &[TaskItem], filter: &TaskFilter) -> BoardView {
    board_columns_by(tasks, filter, BoardColumn::of)
}

/// The same, with the column of each task given by `column_of`, so cards that have a move
/// saving can already sit where they are going.
pub fn board_columns_by(
    tasks: &[TaskItem],
    filter: &TaskFilter,
    column_of: impl Fn(&TaskItem) -> BoardColumn,
) -> BoardView {
    let mut view = BoardView {
        done_folded: filter.done == DoneFilter::Open,
        ..Default::default()
    };
    for (i, t) in tasks.iter().enumerate() {
        if !filter.passes_rest(t) {
            continue;
        }
        let column = column_of(t);
        if filter.done == DoneFilter::Done && column != BoardColumn::Done {
            continue;
        }
        view.columns[column.index()].push(i);
    }
    let due = |t: &TaskItem| (t.due_date.is_none(), t.due_date);
    for c in BoardColumn::ALL {
        view.columns[c.index()].sort_by_key(|&i| {
            let t = &tasks[i];
            if c == BoardColumn::Done {
                (0, (true, None), t.meeting_rank, t.index)
            } else {
                (super::tasks::mine_rank(t), due(t), t.meeting_rank, t.index)
            }
        });
    }
    view
}

/// What a move asks for, as values to set, never as a flip. A move to Done sets the tick and
/// leaves the status alone; a move to any other column sets that status and clears the tick.
/// Two intents on one card merge into one (`then`): the later choice wins field by field, and a
/// field the later one does not name keeps the earlier value. So Review, then Done, then Waiting
/// on ends as "Waiting on, not done", and a card moved to Done and nowhere else keeps the status
/// it had. Intents carry no snapshot of the card, so a queued one cannot undo a newer save.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MoveIntent {
    /// Set the status. Waiting on is set with no one named, as the panel does.
    pub status: Option<TaskStatus>,
    /// Set the done tick to this value.
    pub done: Option<bool>,
}

impl MoveIntent {
    pub fn to(target: BoardColumn) -> Self {
        match target.status() {
            None => Self {
                status: None,
                done: Some(true),
            },
            Some(status) => Self {
                status: Some(status),
                done: Some(false),
            },
        }
    }

    /// Set the tick and nothing else.
    pub fn tick(on: bool) -> Self {
        Self {
            status: None,
            done: Some(on),
        }
    }

    /// This intent, then `next`.
    pub fn then(self, next: Self) -> Self {
        Self {
            status: next.status.or(self.status),
            done: next.done.or(self.done),
        }
    }

    /// The column a card is in once this is saved: the intent over the card's own values.
    pub fn column(self, t: &TaskItem) -> BoardColumn {
        if self.done.unwrap_or(t.done) {
            return BoardColumn::Done;
        }
        match self.status.unwrap_or(t.status) {
            TaskStatus::Todo => BoardColumn::Todo,
            TaskStatus::Waiting => BoardColumn::Waiting,
            TaskStatus::Review => BoardColumn::Review,
        }
    }
}

/// How many versions of this task the sync is holding for the user to choose from, if any.
/// A hand-made task is matched by its id, a meeting task by its edit entry in the overrides
/// file or by its tick in its meeting's file. The task panel and the cards share this.
pub fn task_conflict_versions(
    conflicts: &[super::relay_preview::Conflict],
    t: &TaskItem,
) -> Option<usize> {
    conflicts
        .iter()
        .filter(|c| conflict_is_for(c, t))
        .map(|c| c.choices.len())
        .max()
}

fn conflict_is_for(c: &super::relay_preview::Conflict, t: &TaskItem) -> bool {
    if t.manual {
        let id = t
            .meeting_id
            .strip_prefix("manual/")
            .unwrap_or(&t.meeting_id);
        return c.task_id.strip_prefix("$manual:").unwrap_or(&c.task_id) == id;
    }
    let path: Vec<String> = serde_json::from_str(&c.field).unwrap_or_default();
    let name = |at: usize| path.get(at).map(|p| p.strip_prefix("o:").unwrap_or(p));
    if c.task_id == "task-overrides.json" {
        return path.len() >= 3
            && name(0) == Some("tasks")
            && name(1) == Some(t.meeting_id.as_str())
            && name(2)
                == Some(
                    super::task_override_key(&t.meeting_id, &t.stable_id, &t.task, t.index)
                        .as_str(),
                );
    }
    c.task_id == format!("{}.json", t.meeting_id)
        && path.len() == 2
        && name(0) == Some("checked")
        && (name(1) == Some(t.task.as_str())
            || (!t.stable_id.is_empty()
                && name(1) == Some(super::task_tick_key(&t.stable_id).as_str())))
}

/// What a finished move leaves for the page to take in: the new tick file, the new overrides,
/// or both.
#[derive(Default)]
pub struct Moved {
    pub ticks: Option<EditView<Saved>>,
    pub overrides: Option<EditView<Overrides>>,
}

/// Runs a move: the tick first, then the status. Each step is a set, so a retry after the
/// second step failed finishes the job and a repeat changes nothing. Steps that need no change
/// are not skipped on a guess from an old snapshot; the sets are cheap and exact. The views of the steps
/// that ran come back even when a later step failed, so the page shows what really happened.
pub fn run_move(
    store: &TaskStore,
    ticks: &StateStore,
    task: &TaskItem,
    source: &Path,
    intent: MoveIntent,
) -> (Moved, Option<io::Error>) {
    let mut moved = Moved::default();
    if let Some(on) = intent.done {
        let step = if task.manual {
            let id = task
                .meeting_id
                .strip_prefix("manual/")
                .unwrap_or(&task.meeting_id);
            store
                .set_manual_done_view(id, on)
                .map(|v| moved.overrides = Some(v))
        } else {
            ticks
                .set_task_tick_view(&task.meeting_id, &task.task, &task.stable_id, on)
                .map(|v| moved.ticks = Some(v))
        };
        if let Err(e) = step {
            return (moved, Some(e));
        }
    }
    if let Some(status) = intent.status {
        match store.edit_v2(task, source, TaskV2Edit::Status(status, None)) {
            Ok(v) => moved.overrides = Some(v),
            Err(e) => return (moved, Some(e)),
        }
    }
    (moved, None)
}

/// The column a dragged card is over. `x_in_column` is where the pointer went down, measured
/// from the left edge of the card's own column; `dx` is how far it has moved since. Columns
/// are `width` wide with `gap` between, so this is plain arithmetic and needs no hit test.
pub fn drop_column(
    from: BoardColumn,
    x_in_column: f32,
    dx: f32,
    width: f32,
    gap: f32,
) -> BoardColumn {
    let step = (width + gap).max(1.0);
    let shift = ((x_in_column + dx) / step).floor() as isize;
    let at = (from.index() as isize + shift).clamp(0, 3) as usize;
    BoardColumn::ALL[at]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neighbours_stop_at_the_ends_and_do_not_wrap() {
        use BoardColumn::{Done, Review, Todo, Waiting};
        assert_eq!(Todo.neighbour(-1), None);
        assert_eq!(Todo.neighbour(1), Some(Waiting));
        assert_eq!(Waiting.neighbour(1), Some(Review));
        assert_eq!(Review.neighbour(1), Some(Done));
        assert_eq!(Done.neighbour(1), None);
        assert_eq!(Done.neighbour(-1), Some(Review));
        assert_eq!(
            BoardColumn::ALL.map(BoardColumn::label),
            ["To do", "Waiting on", "In review", "Done"]
        );
        assert!(Done.status().is_none() && Review.status() == Some(TaskStatus::Review));
        // The column labels match the status labels the task panel uses.
        for c in BoardColumn::ALL {
            if let Some(status) = c.status() {
                assert_eq!(c.label(), status.label());
            }
        }
    }

    #[test]
    fn a_drag_lands_in_the_column_under_the_pointer_and_stays_on_the_board() {
        use BoardColumn::{Done, Review, Todo, Waiting};
        // Columns 200 wide with 12 between: a step of 212. The pointer went down 30 in.
        let drop = |from, dx| drop_column(from, 30.0, dx, 200.0, 12.0);
        assert_eq!(drop(Todo, 0.0), Todo);
        assert_eq!(drop(Todo, 100.0), Todo);
        assert_eq!(drop(Todo, 200.0), Waiting);
        assert_eq!(drop(Todo, 420.0), Review);
        assert_eq!(drop(Todo, 5000.0), Done);
        assert_eq!(drop(Review, -200.0), Waiting);
        assert_eq!(drop(Review, -5000.0), Todo);
        // Left of the board, or a zero-width layout, never panics.
        assert_eq!(drop_column(Done, 0.0, -1.0, 0.0, 0.0), Review);
        assert_eq!(drop(Waiting, 150.0), Waiting);
    }

    #[test]
    fn rapid_ticks_alternate_from_what_the_card_will_be_and_end_on_the_last_press() {
        // The saved card is open. Each press sets the opposite of what the card will be once the
        // earlier presses are saved, so none reads a stale value and none is lost.
        let mut pending = MoveIntent::default();
        let mut after = Vec::new();
        for _ in 0..5 {
            let will_be = pending.done.unwrap_or(false);
            pending = pending.then(MoveIntent::tick(!will_be));
            after.push(pending.done);
        }
        assert_eq!(
            after,
            [Some(true), Some(false), Some(true), Some(false), Some(true)]
        );
        // An even number of presses ends open, and the status behind the tick is never touched.
        let off = pending.then(MoveIntent::tick(false));
        assert_eq!((off.done, off.status), (Some(false), None));
    }

    #[test]
    fn intents_merge_field_by_field_and_the_last_choice_wins() {
        use BoardColumn::{Done, Review, Todo, Waiting};
        let through_done = MoveIntent::to(Review).then(MoveIntent::to(Done));
        // Done leaves the status Review in place.
        assert_eq!(
            through_done,
            MoveIntent {
                status: Some(TaskStatus::Review),
                done: Some(true)
            }
        );
        // Done, then on to Waiting on: the final column is the one chosen last.
        let on = through_done.then(MoveIntent::to(Waiting));
        assert_eq!(
            on,
            MoveIntent {
                status: Some(TaskStatus::Waiting),
                done: Some(false)
            }
        );
        // And back to Done, then To do, ends in To do.
        let again = on.then(MoveIntent::to(Done)).then(MoveIntent::to(Todo));
        assert_eq!(again, MoveIntent::to(Todo));
        // Merging with nothing changes nothing.
        assert_eq!(on.then(MoveIntent::default()), on);
    }
}
