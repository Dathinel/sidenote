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
    let mut view = BoardView {
        done_folded: filter.done == DoneFilter::Open,
        ..Default::default()
    };
    for (i, t) in tasks.iter().enumerate() {
        if !filter.passes_rest(t) {
            continue;
        }
        let column = BoardColumn::of(t);
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

/// What moving a card needs saved. Each step is a "set", never a flip, so a retry after a
/// half-finished move, or a move queued behind another save, ends in the same place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MovePlan {
    /// Set the done tick to this value.
    pub tick: Option<bool>,
    /// Set the status. Waiting on is set with no one named, as the panel does.
    pub status: Option<TaskStatus>,
}

impl MovePlan {
    pub fn is_noop(self) -> bool {
        self.tick.is_none() && self.status.is_none()
    }
}

pub fn move_plan(t: &TaskItem, target: BoardColumn) -> MovePlan {
    match target.status() {
        // Done is the tick alone. The status behind it stays as it was.
        None => MovePlan {
            tick: (!t.done).then_some(true),
            status: None,
        },
        Some(status) => MovePlan {
            tick: t.done.then_some(false),
            status: (t.status != status).then_some(status),
        },
    }
}

/// What a finished move leaves for the page to take in: the new tick file, the new overrides,
/// or both.
#[derive(Default)]
pub struct Moved {
    pub ticks: Option<EditView<Saved>>,
    pub overrides: Option<EditView<Overrides>>,
}

/// Runs a move: the tick first, then the status. Each step is a set, so a retry after the
/// second step failed finishes the job and a repeat changes nothing. The views of the steps
/// that ran come back even when a later step failed, so the page shows what really happened.
pub fn run_move(
    store: &TaskStore,
    ticks: &StateStore,
    task: &TaskItem,
    source: &Path,
    plan: MovePlan,
) -> (Moved, Option<io::Error>) {
    let mut moved = Moved::default();
    if let Some(on) = plan.tick {
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
    if let Some(status) = plan.status {
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
}
