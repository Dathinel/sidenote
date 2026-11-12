//! Session-only encoded-frame timings. The producer belongs to the UI thread.
use std::{
    cell::{Cell, RefCell},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

const CAPACITY: usize = 2048;
const WINDOW: usize = 256;
pub const ACTIONS: [&str; 14] = [
    "Open app",
    "Open meeting",
    "Meeting tab",
    "Transcript",
    "Meeting search",
    "Open finder",
    "Finder query",
    "Tasks board",
    "Task filter",
    "Task details",
    "Settings",
    "Ask first text",
    "Annotations",
    "Stop to notes ready",
];

#[derive(Clone, Copy)]
#[repr(usize)]
pub enum Action {
    Launch,
    Meeting,
    Tab,
    Transcript,
    Search,
    Finder,
    FinderQuery,
    Tasks,
    TaskFilter,
    TaskDetails,
    Settings,
    Ask,
    Annotations,
    Notes,
}

#[derive(Clone, Default)]
pub struct Summary {
    pub count: u64,
    pub first_p50_ms: f64,
    pub first_p95_ms: f64,
    pub complete_p50_ms: f64,
    pub complete_p95_ms: f64,
    pub failed: u64,
    pub superseded: u64,
}

struct Slot {
    id: AtomicU64,
    tag: AtomicUsize,
    micros: AtomicU64,
}

impl Slot {
    fn new() -> Self {
        Self {
            id: AtomicU64::new(0),
            tag: AtomicUsize::new(0),
            micros: AtomicU64::new(0),
        }
    }
}

struct Queue {
    slots: [Slot; CAPACITY],
    head: AtomicUsize,
    tail: AtomicUsize,
    drops: AtomicU64,
    alive: AtomicBool,
    summary: Mutex<[Summary; ACTIONS.len()]>,
}

struct Producer {
    queue: Arc<Queue>,
    origin: Instant,
}

impl Drop for Producer {
    fn drop(&mut self) {
        self.queue.alive.store(false, Ordering::Release);
    }
}

thread_local! {
    static RECORDER: RefCell<Option<Producer>> = const { RefCell::new(None) };
    static NEXT: Cell<u64> = const { Cell::new(0) };
}

pub fn enabled() -> bool {
    RECORDER.with_borrow(|r| r.is_some())
}

/// Enabling/clearing is explicit; no disk, network, entity IDs, or text is accepted.
pub fn set_enabled(enabled: bool) {
    RECORDER.with_borrow_mut(|r| {
        if !enabled {
            *r = None;
            return;
        }
        if r.is_some() {
            return;
        }
        let queue = Arc::new(Queue {
            slots: std::array::from_fn(|_| Slot::new()),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            drops: AtomicU64::new(0),
            alive: AtomicBool::new(true),
            summary: Mutex::new(std::array::from_fn(|_| Summary::default())),
        });
        let worker = queue.clone();
        if std::thread::Builder::new()
            .name("response-times".into())
            .spawn(move || consume(worker))
            .is_ok()
        {
            *r = Some(Producer {
                queue,
                origin: Instant::now(),
            });
        }
    });
}

fn emit(p: &Producer, id: u64, tag: usize, at: Instant) {
    let q = &p.queue;
    let head = q.head.load(Ordering::Relaxed);
    if head.wrapping_sub(q.tail.load(Ordering::Acquire)) == CAPACITY {
        q.drops.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let slot = &q.slots[head % CAPACITY];
    slot.id.store(id, Ordering::Relaxed);
    slot.tag.store(tag, Ordering::Relaxed);
    slot.micros.store(
        at.saturating_duration_since(p.origin).as_micros() as u64,
        Ordering::Relaxed,
    );
    q.head.store(head.wrapping_add(1), Ordering::Release);
}

pub fn begin(action: Action) -> u64 {
    RECORDER.with_borrow_mut(|r| {
        let Some(p) = r else {
            return 0;
        };
        let id = NEXT.with(|n| {
            n.set(n.get() + 1);
            n.get()
        });
        emit(p, id, action as usize, Instant::now());
        id
    })
}

fn mark(id: u64, tag: usize) {
    if id != 0 {
        RECORDER.with_borrow(|r| {
            if let Some(p) = r {
                emit(p, id, tag, Instant::now());
            }
        });
    }
}

pub fn ready(id: u64) {
    mark(id, ACTIONS.len());
}
pub fn failed(id: u64) {
    mark(id, ACTIONS.len() + 1);
}
pub fn frame(at: Instant) {
    RECORDER.with_borrow(|r| {
        if let Some(p) = r {
            emit(p, u64::MAX, ACTIONS.len() + 2, at);
        }
    });
}

/// Snapshot only when Diagnostics is drawn. Never wait for the worker's lock.
pub fn snapshot() -> Option<([Summary; ACTIONS.len()], u64)> {
    RECORDER.with_borrow(|r| {
        let q = &r.as_ref()?.queue;
        Some((
            q.summary.try_lock().ok()?.clone(),
            q.drops.load(Ordering::Relaxed),
        ))
    })
}

#[derive(Clone, Copy)]
struct Pending {
    id: u64,
    start: u64,
    first: Option<u64>,
    ready: bool,
}

#[derive(Default)]
struct Samples {
    summary: Summary,
    values: Vec<(u64, u64)>,
    pending: Option<Pending>,
}

fn percentile(values: &mut [u64], percent: usize) -> f64 {
    values.sort_unstable();
    values[(values.len() * percent).div_ceil(100).saturating_sub(1)] as f64 / 1000.0
}

impl Samples {
    fn finish_frame(&mut self, at: u64) {
        let Some(p) = &mut self.pending else {
            return;
        };
        let first = *p.first.get_or_insert(at.saturating_sub(p.start));
        if !p.ready {
            return;
        }
        let complete = at.saturating_sub(p.start);
        if self.values.len() == WINDOW {
            self.values.remove(0);
        }
        self.values.push((first, complete));
        self.summary.count += 1;
        let mut first: Vec<_> = self.values.iter().map(|v| v.0).collect();
        let mut complete: Vec<_> = self.values.iter().map(|v| v.1).collect();
        self.summary.first_p50_ms = percentile(&mut first, 50);
        self.summary.first_p95_ms = percentile(&mut first, 95);
        self.summary.complete_p50_ms = percentile(&mut complete, 50);
        self.summary.complete_p95_ms = percentile(&mut complete, 95);
        self.pending = None;
    }
}

fn record(samples: &mut [Samples; ACTIONS.len()], id: u64, tag: usize, at: u64) {
    if tag < ACTIONS.len() {
        let s = &mut samples[tag];
        if s.pending.is_some() {
            s.summary.superseded += 1;
        }
        s.pending = Some(Pending {
            id,
            start: at,
            first: None,
            ready: false,
        });
    } else if tag == ACTIONS.len() + 2 {
        for s in samples {
            s.finish_frame(at);
        }
    } else if let Some(s) = samples
        .iter_mut()
        .find(|s| s.pending.is_some_and(|p| p.id == id))
    {
        if tag == ACTIONS.len() {
            if let Some(p) = &mut s.pending {
                p.ready = true;
            }
        } else {
            s.summary.failed += 1;
            s.pending = None;
        }
    }
}

fn consume(q: Arc<Queue>) {
    let mut samples: [Samples; ACTIONS.len()] = std::array::from_fn(|_| Samples {
        values: Vec::with_capacity(WINDOW),
        ..Default::default()
    });
    while q.alive.load(Ordering::Acquire) {
        let mut tail = q.tail.load(Ordering::Relaxed);
        let head = q.head.load(Ordering::Acquire);
        while tail != head {
            let slot = &q.slots[tail % CAPACITY];
            record(
                &mut samples,
                slot.id.load(Ordering::Relaxed),
                slot.tag.load(Ordering::Relaxed),
                slot.micros.load(Ordering::Relaxed),
            );
            tail = tail.wrapping_add(1);
        }
        q.tail.store(tail, Ordering::Release);
        if let Ok(mut summary) = q.summary.lock() {
            *summary = std::array::from_fn(|i| samples[i].summary.clone());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn async_completion_requires_a_later_frame_and_matches_generation() {
        let mut s = std::array::from_fn(|_| Samples::default());
        record(&mut s, 1, Action::Search as usize, 1000);
        record(&mut s, 0, ACTIONS.len() + 2, 2000);
        record(&mut s, 2, Action::Search as usize, 3000);
        record(&mut s, 1, ACTIONS.len(), 4000);
        record(&mut s, 0, ACTIONS.len() + 2, 5000);
        assert_eq!(s[Action::Search as usize].summary.count, 0);
        record(&mut s, 2, ACTIONS.len(), 6000);
        record(&mut s, 0, ACTIONS.len() + 2, 7000);
        let result = &s[Action::Search as usize].summary;
        assert_eq!((result.count, result.superseded), (1, 1));
        assert_eq!((result.first_p50_ms, result.complete_p95_ms), (2.0, 4.0));
    }

    #[test]
    fn failed_operations_do_not_become_successes_and_window_is_bounded() {
        let mut s = std::array::from_fn(|_| Samples::default());
        record(&mut s, 1, 0, 0);
        record(&mut s, 1, ACTIONS.len() + 1, 1);
        record(&mut s, 0, ACTIONS.len() + 2, 2);
        assert_eq!((s[0].summary.failed, s[0].summary.count), (1, 0));
        for id in 2..300 {
            record(&mut s, id, 0, 0);
            record(&mut s, id, ACTIONS.len(), 1);
            record(&mut s, 0, ACTIONS.len() + 2, id * 1000);
        }
        assert_eq!(s[0].values.len(), WINDOW);
        assert_eq!(s[0].summary.count, 298);
    }
}
