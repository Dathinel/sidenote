use super::*;
thread_local! { static HELD: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
pub(crate) struct IntakeTrace(Option<(&'static std::sync::Mutex<()>, PathBuf)>);
impl IntakeTrace {
    pub(crate) fn new(mutex: &'static std::sync::Mutex<()>, path: PathBuf) -> Self {
        HELD.with(|n| n.set(n.get() + 1));
        Self(
            (active()
                && crate::core::state_dir()
                    .join("check-intake-release")
                    .exists())
            .then_some((mutex, path)),
        )
    }
}
impl Drop for IntakeTrace {
    fn drop(&mut self) {
        HELD.with(|n| n.set(n.get() - 1));
        if let Some((mutex, path)) = &self.0 {
            let thread_held = matches!(mutex.try_lock(), Err(std::sync::TryLockError::WouldBlock));
            let file_released = store_io::lock(path).is_ok();
            assert!(
                thread_held && file_released,
                "intake release order: thread held={thread_held}, file released={file_released}"
            );
        }
    }
}
pub(crate) fn before_job_lock(work: &Path) {
    let root = crate::core::state_dir();
    if root.join("inspect-job-lock").exists() {
        fs::create_dir_all(work).unwrap();
        fs::write(
            root.join("job-lock-ready"),
            HELD.with(|n| if n.get() > 0 { "held" } else { "released" }),
        )
        .unwrap();
        while root.join("inspect-job-lock").exists() {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
#[test]
fn job_creation_keeps_intake_ownership_until_processing_is_locked() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = setup();
    let root = root.path();
    fs::write(root.join("inspect-job-lock"), b"pause").unwrap();
    fs::write(root.join("check-intake-release"), []).unwrap();
    let service = Service::start(root);
    let (id, piece) = service.upload(&request(), 0);
    wait(|| root.join("job-lock-ready").exists());
    assert_eq!(
        fs::read_to_string(root.join("job-lock-ready")).unwrap(),
        "held",
        "runner released intake before locking processing"
    );
    let lock = store_io::lock(&root.join("local/speech/intake.lock"));
    assert_eq!(
        lock.expect_err("job creation released intake before locking processing")
            .kind(),
        std::io::ErrorKind::WouldBlock
    );
    fs::remove_file(root.join("inspect-job-lock")).unwrap();
    wait(|| service.snapshot(&id, &piece)["state"] == "done");
}
