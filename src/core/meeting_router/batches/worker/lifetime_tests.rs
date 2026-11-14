use super::super::tests::ranges;
use super::tests::{command, start};
use super::*;
fn wait(path: &Path, bound: Duration) {
    let deadline = Instant::now() + bound;
    while !path.exists() {
        assert!(Instant::now() < deadline, "supervised deadline");
        std::thread::sleep(Duration::from_millis(5));
    }
}
pub(super) fn drive(mode: &str, output: &Path) {
    let mut worker = start("hang");
    wait(&worker.output.path().join("pid"), Duration::from_secs(10));
    store_io::write(&output.join("ready"), &true).unwrap();
    let started = Instant::now();
    match mode {
        "deadline-cancel" => {
            worker.cancel.store(true, Ordering::Release);
            assert!(
                worker.child.wait_for(500).unwrap().is_some(),
                "idle cancellation deadline"
            );
            let error = worker
                .recognize(&ranges(Path::new("unused.wav")).remove(1))
                .unwrap_err();
            assert_eq!(
                error.downcast_ref::<WorkerFailure>(),
                Some(&WorkerFailure::Cancelled)
            );
            worker.shutdown().unwrap();
        }
        "deadline-timeout" => {
            worker.timeout = Duration::from_millis(50);
            let error = worker
                .recognize(&ranges(Path::new("unused.wav")).remove(1))
                .unwrap_err();
            assert_eq!(error.downcast_ref::<Failure>(), Some(&Failure::Timeout));
        }
        "deadline-drop" => drop(worker),
        "deadline-cleanup" => {
            drop(worker);
            let elapsed = cleanup_deadline();
            store_io::write(&output.join("elapsed"), &(elapsed.as_millis() as u64)).unwrap();
            return;
        }
        _ => panic!("unknown deadline fixture"),
    }
    assert!(started.elapsed() < Duration::from_secs(1));
    store_io::write(
        &output.join("elapsed"),
        &(started.elapsed().as_millis() as u64),
    )
    .unwrap();
}
fn cleanup_deadline() -> Duration {
    // A separate live job forces a delayed termination without weakening production cleanup.
    let blocker_job = crate::platform::ProcessJob::new().unwrap();
    let mut c = command("grandchild");
    c.env_clear()
        .env("SystemRoot", std::env::var_os("SystemRoot").unwrap());
    let child = blocker_job.spawn(&c).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let (stop, cleanup) = controller(
        crate::platform::ProcessJob::new().unwrap(),
        child.handle.try_clone().unwrap(),
        cancel.clone(),
    )
    .unwrap();
    let mut worker = Worker {
        child,
        stop,
        cleanup,
        cleaned: None,
        output: tempfile::tempdir().unwrap(),
        cancel,
        timeout: Duration::from_secs(1),
    };
    let started = Instant::now();
    let error = worker.shutdown().unwrap_err();
    let elapsed = started.elapsed();
    assert_eq!(
        error.downcast_ref::<WorkerFailure>(),
        Some(&WorkerFailure::CleanupTimeout)
    );
    assert!(elapsed < Duration::from_secs(1));
    assert!(worker.child.try_wait().unwrap().is_none());
    let completion = std::mem::replace(&mut worker.cleanup, mpsc::channel().1);
    drop(worker);
    drop(blocker_job);
    assert_eq!(
        completion.recv_timeout(Duration::from_secs(1)),
        Err(mpsc::RecvTimeoutError::Disconnected)
    );
    elapsed
}
#[test]
fn externally_supervised_idle_cancel_timeout_drop_and_deferred_reap_deadlines() {
    for mode in [
        "deadline-cancel",
        "deadline-timeout",
        "deadline-drop",
        "deadline-cleanup",
    ] {
        let owner = start(mode);
        wait(&owner.output.path().join("ready"), Duration::from_secs(10));
        let elapsed = owner.output.path().join("elapsed");
        wait(&elapsed, Duration::from_secs(2));
        let elapsed: u64 =
            serde_json::from_slice(&store_io::read_bounded(&elapsed, 4096).unwrap()).unwrap();
        assert!(elapsed < 1000, "{mode} exceeded caller deadline");
    }
}
