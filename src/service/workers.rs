//! In-process queue owners share the service replica; only inference/CLI children fork.
use std::{io, sync::Mutex, thread::JoinHandle};

struct Worker {
    name: &'static str,
    handle: JoinHandle<()>,
}
static WORKERS: Mutex<Vec<Worker>> = Mutex::new(Vec::new());

pub(crate) fn start(name: &'static str, run: impl FnOnce() + Send + 'static) -> io::Result<()> {
    start_in(&WORKERS, name, run)
}
fn start_in(
    workers: &Mutex<Vec<Worker>>,
    name: &'static str,
    run: impl FnOnce() + Send + 'static,
) -> io::Result<()> {
    let mut workers = workers
        .lock()
        .map_err(|_| io::Error::other("WORKER_FAILED"))?;
    if crate::service::stopping() {
        return Ok(());
    }
    for index in (0..workers.len()).rev() {
        if workers[index].handle.is_finished() {
            workers
                .swap_remove(index)
                .handle
                .join()
                .map_err(|_| io::Error::other("WORKER_FAILED"))?;
        }
    }
    if workers.iter().any(|worker| worker.name == name) {
        return Ok(());
    }
    workers.push(Worker {
        name,
        handle: std::thread::Builder::new().name(name.into()).spawn(run)?,
    });
    Ok(())
}
pub(crate) fn stop() {
    let workers = WORKERS
        .lock()
        .map(|mut workers| std::mem::take(&mut *workers));
    if let Ok(workers) = workers {
        for worker in workers {
            let _ = worker.handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_ticks_share_one_worker_and_finished_work_can_restart() {
        let workers = Mutex::new(Vec::new());
        let (release, blocked) = std::sync::mpsc::channel();
        start_in(&workers, "notes", move || blocked.recv().unwrap()).unwrap();
        start_in(&workers, "notes", || panic!("duplicate worker")).unwrap();
        assert_eq!(workers.lock().unwrap().len(), 1);
        release.send(()).unwrap();
        let worker = workers.lock().unwrap().pop().unwrap();
        worker.handle.join().unwrap();
        start_in(&workers, "notes", || {}).unwrap();
        workers
            .lock()
            .unwrap()
            .pop()
            .unwrap()
            .handle
            .join()
            .unwrap();
    }
}
