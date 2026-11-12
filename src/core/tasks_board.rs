//! The Tasks board: four columns drawn over the tasks the list already shows. A column is not a
//! stored value. To do, Waiting on and In review are the existing `TaskStatus`; Done is the
//! existing tick. Nothing here adds a status or touches a sync file.
use super::{DoneFilter, TaskFilter, TaskItem, TaskStatus};

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
}
