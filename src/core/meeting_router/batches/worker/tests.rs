use super::super::tests::{Fakes, fake, id, ranges};
use super::*;
use std::{io::BufRead, os::windows::process::CommandExt, process::Stdio};
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
    if mode == "owner" {
        let inner = start("hang");
        let pid = inner.output.path().join("pid");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !pid.exists() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        let grandchild: u32 = std::fs::read_to_string(pid).unwrap().parse().unwrap();
        store_io::write(&output.join("tree"), &[inner.child.id(), grandchild]).unwrap();
        std::thread::sleep(Duration::from_secs(60));
        return;
    }
    // Deliberately spawn before reading any handshake.
    let mut grandchild = command("grandchild").spawn().unwrap();
    store_io::write(&output.join("pid"), &grandchild.id()).unwrap();
    let mut lines = std::io::stdin().lock().lines();
    assert_eq!(lines.next().unwrap().unwrap(), "start");
    for line in lines {
        assert_eq!(line.unwrap(), "range");
        let _range: Range = serde_json::from_slice(
            &store_io::read_bounded(&output.join("range.json"), 1 << 20).unwrap(),
        )
        .unwrap();
        if mode == "hang" {
            std::fs::write(output.join("active"), b"").unwrap();
            std::thread::sleep(Duration::from_secs(60));
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
fn command(mode: &str) -> Command {
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
        let mut worker = start("hang");
        let cancel = worker.cancel.clone();
        if mode == "timeout" {
            worker.timeout = Duration::from_millis(100);
        }
        let started = Instant::now();
        let pid = worker.output.path().join("pid");
        while !pid.exists() {
            assert!(started.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(10));
        }
        let pid: u32 = std::fs::read_to_string(pid).unwrap().parse().unwrap();
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        assert!(!handle.is_null());
        // Keep an independent worker handle to verify termination after Drop.
        let worker_handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, worker.child.id()) };
        assert!(!worker_handle.is_null());
        if mode != "drop" {
            let active = worker.output.path().join("active");
            let cancelling = std::thread::spawn(move || {
                if mode == "cancel" {
                    while !active.exists() {
                        assert!(started.elapsed() < Duration::from_secs(10));
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    cancel.store(true, Ordering::Release);
                }
            });
            let error = worker
                .recognize(&ranges(Path::new("unused.wav")).remove(1))
                .unwrap_err();
            if mode == "timeout" {
                assert_eq!(error.downcast_ref::<Failure>(), Some(&Failure::Timeout));
            }
            cancelling.join().unwrap();
            assert!(worker.child.try_wait().unwrap().is_some());
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
    assert!(run(root, &plan, &mut Loader("crash"), &mut bridge).is_err());
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
impl NativeLoader for Loader {
    fn load(&mut self, _: NativeLanguage, _: &Range) -> Result<Box<dyn Recognizer>> {
        Ok(Box::new(start(self.0)))
    }
}

fn start(mode: &str) -> Worker {
    Worker::spawn(
        command(mode),
        Arc::new(AtomicBool::new(false)),
        Duration::from_secs(120),
    )
    .unwrap()
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
    let owner = start("owner");
    let path = owner.output.path().join("tree");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
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
    owner.child.kill().unwrap();
    for handle in handles {
        assert_eq!(
            unsafe { WaitForSingleObject(handle.as_raw_handle(), 5000) },
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
