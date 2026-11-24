use super::super::tests::{Fakes, fake, id, ranges};
use super::signals::Signal;
use super::*;
use std::{io::BufRead, os::windows::process::CommandExt, process::Stdio};

const FIXTURE_HANG_GUARD: Duration = Duration::from_secs(60);

#[test]
fn worker_child() {
    let Some(mode) = std::env::args().skip_while(|a| a != "--skip").nth(1) else {
        return;
    };
    if mode == "grandchild" {
        std::thread::sleep(Duration::from_secs(60));
        return;
    }
    assert!(std::env::var_os("SIDENOTE_SENTINEL").is_none());
    if let Some(handle) = mode.strip_prefix("sentinel-") {
        unsafe {
            windows_sys::Win32::System::Threading::SetEvent(handle.parse::<usize>().unwrap() as _);
        }
    }
    let output = PathBuf::from(std::env::var_os("SIDENOTE_BATCH_OUTPUT").unwrap());
    if mode == "close-input" {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::CloseHandle;
        let stdin = std::io::stdin();
        let mut start = String::new();
        stdin.read_line(&mut start).unwrap();
        assert_eq!(start, "start\n");
        // The disposable fixture owns this inherited handle and never reads stdin again.
        assert_ne!(unsafe { CloseHandle(stdin.as_raw_handle()) }, 0);
        let exit = Signal::open_for_wait("fixture-exit");
        Signal::notify("fixture-ready");
        exit.wait_active();
        std::process::exit(23);
    }
    if mode.starts_with("deadline-") {
        super::lifetime_tests::drive(&mode, &output);
        return;
    }
    if mode == "orphan" {
        // Never reads stdin, so the owner's death cannot make it clean up; only the job can.
        let mut grandchild = command("grandchild").spawn().unwrap();
        store_io::write(&output.join("pid"), &grandchild.id()).unwrap();
        Signal::notify("fixture-ready");
        std::thread::sleep(Duration::from_secs(60));
        grandchild.wait().unwrap();
        return;
    }
    if mode == "owner" {
        let inner = start("orphan");
        let pid = inner.output.path().join("pid");
        let grandchild: u32 = std::fs::read_to_string(pid).unwrap().parse().unwrap();
        store_io::write(&output.join("tree"), &[inner.child.id(), grandchild]).unwrap();
        Signal::notify("fixture-ready");
        std::thread::sleep(Duration::from_secs(60));
        return;
    }
    // Deliberately spawn before reading any handshake.
    let mut grandchild = command("grandchild").spawn().unwrap();
    store_io::write(&output.join("pid"), &grandchild.id()).unwrap();
    Signal::notify("fixture-ready");
    let mut lines = std::io::stdin().lock().lines();
    assert_eq!(lines.next().unwrap().unwrap(), "start");
    for line in lines {
        assert_eq!(line.unwrap(), "range");
        let _range: Range = serde_json::from_slice(
            &store_io::read_bounded(&output.join("range.json"), 1 << 20).unwrap(),
        )
        .unwrap();
        if mode == "hang" {
            Signal::notify("fixture-active");
            std::thread::sleep(Duration::from_secs(60));
        }
        if mode == "crash-on-release" {
            Signal::open_for_wait("fixture-exit").wait_active();
            std::process::exit(7);
        }
        if mode == "crash" {
            std::process::exit(7);
        }
        store_io::write(
            &output.join("result.json"),
            &serde_json::json!({"text":"fake", "native_threads":
                std::env::var("SIDENOTE_BATCH_THREADS").unwrap().parse::<usize>().unwrap()}),
        )
        .unwrap();
    }
    grandchild.kill().unwrap();
    grandchild.wait().unwrap();
}
pub(super) fn command(mode: &str) -> Command {
    let mut c = Command::new(std::env::current_exe().unwrap());
    c.creation_flags(0x08000000)
        .args([
            "--exact",
            "core::meeting_router::batches::worker::tests::worker_child",
            "--nocapture",
            "--skip",
            mode,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    c
}
#[test]
fn cancel_timeout_drop_reap_worker_tree() {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };
    for mode in ["cancel", "timeout", "drop"] {
        let mut command = command("hang");
        let active = (mode == "cancel").then(|| Signal::attach(&mut command, "fixture-active"));
        let mut worker = start_ready(command);
        let cancel = worker.cancel.clone();
        if mode == "timeout" {
            worker.timeout = Duration::from_millis(100);
        }
        let started = Instant::now();
        let pid = worker.output.path().join("pid");
        let pid: u32 = std::fs::read_to_string(pid).unwrap().parse().unwrap();
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        assert!(!handle.is_null());
        // Keep an independent worker handle to verify termination after Drop.
        let worker_handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, worker.child.id()) };
        assert!(!worker_handle.is_null());
        if mode != "drop" {
            let cancelling = std::thread::spawn(move || {
                if let Some(active) = active {
                    active.wait_active();
                    cancel.store(true, Ordering::Release);
                }
            });
            let error = worker
                .recognize(&ranges(Path::new("unused.wav")).remove(1))
                .unwrap_err();
            if mode == "timeout" {
                assert_eq!(
                    error.downcast_ref::<Failure>(),
                    Some(&Failure::Timeout),
                    "{mode}: {error:#}"
                );
            } else {
                assert_eq!(
                    error.downcast_ref::<WorkerFailure>(),
                    Some(&WorkerFailure::Cancelled),
                    "{mode}: {error:#}"
                );
            }
            cancelling.join().unwrap();
            let exited = worker.child.try_wait().unwrap();
            eprintln!(
                "reap mode={mode} cleanup={:?} exited={exited:?} elapsed={:?}",
                worker.cleaned,
                started.elapsed()
            );
            assert!(exited.is_some(), "{mode}: cleanup={:?}", worker.cleaned);
        }
        drop(worker);
        unsafe {
            assert_eq!(WaitForSingleObject(worker_handle, 5000), 0);
            assert_eq!(WaitForSingleObject(handle, 5000), 0);
            CloseHandle(worker_handle);
            CloseHandle(handle);
        }
    }
}
#[test]
fn closed_input_before_process_exit_preserves_status() {
    let mut command = command("close-input");
    let exit = Signal::attach(&mut command, "fixture-exit");
    let mut worker = start_ready(command);
    worker.timeout = Duration::ZERO;
    // The fixture exits after the failed write and the initial process check.
    exit_at_deadline(exit);
    let error = worker
        .recognize(&ranges(Path::new("unused.wav")).remove(1))
        .unwrap_err();
    assert!(
        matches!(error.downcast_ref::<WorkerFailure>(), Some(WorkerFailure::Exited(status)) if status.code() == Some(23)),
        "{error:?}"
    );
    worker.finish().unwrap();
}

#[test]
fn process_exit_after_response_poll_preserves_status() {
    let mut command = command("crash-on-release");
    let exit = Signal::attach(&mut command, "fixture-exit");
    let mut worker = start_ready(command);
    worker.timeout = Duration::ZERO;
    // The fixture exits after the response poll saw a live process.
    exit_at_deadline(exit);
    let error = worker
        .recognize(&ranges(Path::new("unused.wav")).remove(1))
        .unwrap_err();
    assert!(
        matches!(error.downcast_ref::<WorkerFailure>(), Some(WorkerFailure::Exited(status)) if status.code() == Some(7)),
        "{error:?}"
    );
}

fn exit_at_deadline(exit: Signal) {
    AT_RESPONSE_DEADLINE.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move |child| {
            exit.set();
            assert!(
                child
                    .wait_for(FIXTURE_HANG_GUARD.as_millis() as u32)
                    .unwrap()
                    .is_some(),
                "fixture did not exit before the hang guard"
            );
        }));
    });
}
#[test]
fn process_crash_preserves_completed_receipts() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let wav = root.join("fake.wav");
    crate::core::asr::write_wav(&wav, &[0.0; 160]).unwrap();
    let plan = spool(root, ranges(&wav)).unwrap();
    fake(root, &plan, &Default::default()).unwrap();
    let inflight = id(&plan, "b");
    std::fs::remove_file(path(root, &plan, &inflight, "receipt")).unwrap();
    let mut bridge = Fakes(Default::default());
    let error = run(root, &plan, &mut Loader("crash"), &mut bridge).unwrap_err();
    assert!(
        matches!(error.downcast_ref::<WorkerFailure>(), Some(WorkerFailure::Exited(status)) if status.code() == Some(7))
    );
    for batch in plan.batches.values() {
        for id in batch.keys() {
            assert_eq!(cached(root, &plan, id), *id != inflight);
        }
    }
    run(root, &plan, &mut Loader("ok"), &mut bridge).unwrap();
    for batch in plan.batches.values() {
        for id in batch.keys() {
            assert!(cached(root, &plan, id));
        }
    }
}
struct Loader(&'static str);
#[test]
fn shutdown_terminates_child_even_when_controller_is_delayed() {
    let (release, gate) = mpsc::channel();
    super::CONTROLLER_GATE.with(|slot| *slot.borrow_mut() = Some(gate));
    let mut worker = start("hang");
    worker.timeout = Duration::from_millis(50);
    let started = Instant::now();
    let error = worker
        .recognize(&ranges(Path::new("unused.wav")).remove(1))
        .unwrap_err();
    assert_eq!(error.downcast_ref::<Failure>(), Some(&Failure::Timeout));
    assert_eq!(worker.cleaned, Some(Err(WorkerFailure::CleanupTimeout)));
    assert!(
        worker.child.try_wait().unwrap().is_some(),
        "queued controller left the child live"
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    release.send(()).unwrap();
    assert_eq!(
        worker.cleanup.recv_timeout(Duration::from_secs(1)).unwrap(),
        Ok(())
    );
}
impl NativeLoader for Loader {
    fn load(&mut self, _: NativeLanguage, _: &Range) -> Result<Box<dyn Recognizer>> {
        Ok(Box::new(start(self.0)))
    }
}

pub(super) fn start(mode: &str) -> Worker {
    start_ready(command(mode))
}
fn start_ready(mut command: Command) -> Worker {
    let ready = Signal::attach(&mut command, "fixture-ready");
    let worker = Worker::spawn(
        command,
        Arc::new(AtomicBool::new(false)),
        Duration::from_secs(120),
    )
    .unwrap();
    ready.wait(&worker.child.handle);
    worker
}
#[test]
fn worker_thread_default_and_override_reach_child() {
    let range = ranges(Path::new("unused.wav")).remove(1);
    let mut default = start("ok");
    assert_eq!(default.recognize(&range).unwrap()["native_threads"], 4);
    drop(default);
    let mut configured = Worker::spawn_with_threads(
        command("ok"),
        Arc::new(AtomicBool::new(false)),
        Duration::from_secs(10),
        NonZeroUsize::new(2).unwrap(),
    )
    .unwrap();
    assert_eq!(configured.recognize(&range).unwrap()["native_threads"], 2);
}

#[test]
fn killing_owner_reaps_eager_worker_and_grandchild() {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };
    let output = tempfile::tempdir().unwrap();
    let mut c = command("owner");
    let ready = Signal::attach(&mut c, "fixture-ready");
    c.env_clear()
        .env("SIDENOTE_BATCH_OUTPUT", output.path())
        .env("SIDENOTE_BATCH_THREADS", "4");
    for key in ["SystemRoot", "TEMP", "TMP"] {
        if let Some(v) = std::env::var_os(key) {
            c.env(key, v);
        }
    }
    // Every descendant starts after this instant, and the orphan worker and its grandchild only
    // exit by themselves 60 s after their own start. So any exit seen before `cap` is the job
    // reaping them, however slowly the loaded machine tears the owner down.
    let cap = std::time::Instant::now() + Duration::from_secs(45);
    let left = || {
        cap.saturating_duration_since(std::time::Instant::now())
            .as_millis() as u32
    };
    let owner_job = crate::platform::ProcessJob::new().unwrap();
    let owner = owner_job.spawn(&c).unwrap();
    ready.wait(&owner.handle);
    let path = output.path().join("tree");
    let pids: Vec<u32> =
        serde_json::from_slice(&store_io::read_bounded(&path, 4096).unwrap()).unwrap();
    let handles: Vec<_> = pids
        .into_iter()
        .map(|pid| unsafe {
            let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            assert!(!h.is_null());
            OwnedHandle::from_raw_handle(h)
        })
        .collect();
    owner.kill().unwrap();
    assert!(owner.wait_for(left()).unwrap().is_some());
    for handle in handles {
        assert_eq!(
            unsafe { WaitForSingleObject(handle.as_raw_handle(), left()) },
            0
        );
    }
}
#[test]
fn environment_and_inheritable_handle_sentinels_are_excluded() {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::{
        Foundation::WAIT_TIMEOUT,
        Security::SECURITY_ATTRIBUTES,
        System::Threading::{CreateEventW, WaitForSingleObject},
    };
    let security = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    let raw = unsafe { CreateEventW(&security, 1, 0, std::ptr::null()) };
    assert!(!raw.is_null());
    let sentinel = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut c = command(&format!("sentinel-{}", raw as usize));
    c.env("SIDENOTE_SENTINEL", "synthetic-marker");
    let mut worker = Worker::spawn(c, Default::default(), Duration::from_secs(10)).unwrap();
    worker
        .recognize(&ranges(Path::new("unused.wav")).remove(1))
        .unwrap();
    assert_eq!(
        unsafe { WaitForSingleObject(sentinel.as_raw_handle(), 0) },
        WAIT_TIMEOUT
    );
}
