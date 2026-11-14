use super::super::tests::{Fakes, fake, id, ranges};
use super::*;
use std::{io::BufRead, os::windows::process::CommandExt};
#[test]
fn worker_child() {
    let Some(mode) = std::env::var_os("BATCH_TEST_MODE") else {
        return;
    };
    if mode == "grandchild" {
        std::thread::sleep(Duration::from_secs(60));
        return;
    }
    let mut lines = std::io::stdin().lock().lines();
    assert_eq!(lines.next().unwrap().unwrap(), "start");
    let mut grandchild = command("grandchild").spawn().unwrap();
    let output = PathBuf::from(std::env::var_os("SIDENOTE_BATCH_OUTPUT").unwrap());
    store_io::write(&output.join("pid"), &grandchild.id()).unwrap();
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
        ])
        .env("BATCH_TEST_MODE", mode)
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
