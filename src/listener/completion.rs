//! Source acknowledgement follows durable queue acceptance or a saved final output.
use crate::core::{
    self, Config,
    meeting_router::{inputs::Snapshot, jobs},
    notes::{self, Engine},
    recording_guard::{self, Eligibility},
    store_io,
};
use std::{io, path::Path};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Acceptance {
    Accepted,
    AlreadyAccepted,
    Deferred,
}
impl Acceptance {
    pub(crate) fn acknowledge(self) -> io::Result<()> {
        match self {
            Self::Deferred => Err(io::ErrorKind::WouldBlock.into()),
            _ => Ok(()),
        }
    }
}
#[cfg(windows)]
pub(crate) fn handoff(id: &str) -> io::Result<Acceptance> {
    let state = core::state_dir();
    let meetings = core::meetings_dir();
    handoff_in(&state, &meetings, id, Config::load(), || {
        super::pipeline_with(&state, &meetings, Some(id), true)
    })
}
fn summary(folder: &Path) -> io::Result<bool> {
    match Snapshot::read_file(
        &folder.join("english-summary.json"),
        core::file_reads::SUMMARY_MAX,
    ) {
        Ok(bytes) => {
            Ok(serde_json::from_slice::<serde_json::Value>(&bytes).is_ok_and(|v| v.is_object()))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}
fn handoff_in(
    state: &Path,
    meetings: &Path,
    id: &str,
    config: io::Result<Config>,
    dispatch: impl FnOnce() -> io::Result<()>,
) -> io::Result<Acceptance> {
    let Some(config) = store_io::defer_busy(config)? else {
        return Ok(Acceptance::Deferred);
    };
    match recording_guard::eligibility(state, meetings, id)? {
        Eligibility::Short => return Ok(Acceptance::AlreadyAccepted), // The guard persists this policy decision.
        Eligibility::Unknown => return Ok(Acceptance::Deferred),
        Eligibility::Eligible => {}
    }
    let settings = config
        .notes
        .ok_or_else(|| io::Error::other("Missing notes settings"))?;
    let accepted = || -> io::Result<bool> {
        let notes = match settings.engine() {
            Engine::App => notes::queue::accepted(state, id)? || summary(&meetings.join(id))?,
            Engine::Scripts(_) => summary(&meetings.join(id))?,
            Engine::Off => false,
        };
        Ok(notes
            && (config.pipeline.profile != "experimental"
                || jobs::automatic_accepted(id).map_err(io::Error::other)?))
    };
    if accepted()? {
        return Ok(Acceptance::AlreadyAccepted);
    }
    let result = dispatch();
    if accepted()? {
        return Ok(Acceptance::Accepted);
    }
    store_io::defer_busy(result)?;
    Ok(Acceptance::Deferred)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::recording_source::{CompletedMeeting, Error, Session, Source};
    use std::fs;
    struct Fixture {
        root: tempfile::TempDir,
        meetings: std::path::PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let meetings = root.path().join("meetings");
            fs::create_dir_all(meetings.join("100-1")).unwrap();
            store_io::write(
                &meetings.join("100-1/manifest.json"),
                &serde_json::json!({"status":"complete","duration_ms":30000}),
            )
            .unwrap();
            fs::write(
                meetings.join("100-1/transcript.ndjson"),
                serde_json::json!({"text":vec!["synthetic";25].join(" ")}).to_string(),
            )
            .unwrap();
            Self { root, meetings }
        }
        fn config(&self) -> Config {
            let mut config = Config::default();
            config.notes = Some(notes::NotesSettings {
                engine: "scripts".into(),
                script: Some(self.root.path().join("synthetic.ps1")),
                ..Default::default()
            });
            config
        }
        fn handoff(
            &self,
            config: io::Result<Config>,
            dispatch: impl FnOnce() -> io::Result<()>,
        ) -> io::Result<Acceptance> {
            handoff_in(self.root.path(), &self.meetings, "100-1", config, dispatch)
        }
    }
    struct One(std::path::PathBuf);
    impl Source for One {
        fn poll(&mut self) -> Result<Vec<CompletedMeeting>, Error> {
            Ok(vec![CompletedMeeting::capture(&self.0, "100-1")?])
        }
    }
    #[test]
    fn recordings_started_during_install_are_handed_off_after_restart_without_events() {
        let f = Fixture::new();
        let cursor = f.root.path().join("source.json");
        let discovery = || core::recording_source::folders::Discovery(f.meetings.clone());
        let mut before_install = Session::open(discovery(), cursor.clone()).unwrap();
        before_install.drain(|_| Ok(())).unwrap();
        drop(before_install);
        for (id, status) in [("100-2", "recording"), ("100-3", "complete")] {
            let folder = f.meetings.join(id);
            fs::create_dir(&folder).unwrap();
            store_io::write(
                &folder.join("manifest.json"),
                &serde_json::json!({"status":status,"duration_ms":30000}),
            )
            .unwrap();
            fs::write(
                folder.join("transcript.ndjson"),
                serde_json::json!({"text":vec!["synthetic";25].join(" ")}).to_string(),
            )
            .unwrap();
        }
        let queued = std::cell::RefCell::new(Vec::new());
        let handoff = |meeting: &CompletedMeeting| {
            handoff_in(
                f.root.path(),
                &f.meetings,
                meeting.id(),
                Ok(f.config()),
                || {
                    queued.borrow_mut().push(meeting.id().to_owned());
                    store_io::write(
                        &f.meetings.join(meeting.id()).join("english-summary.json"),
                        &serde_json::json!({"synthetic":"complete"}),
                    )
                },
            )?
            .acknowledge()
        };
        let mut restarted = Session::open(discovery(), cursor.clone()).unwrap();
        restarted.drain(handoff).unwrap();
        assert_eq!(*queued.borrow(), ["100-3"]);
        store_io::write(
            &f.meetings.join("100-2/manifest.json"),
            &serde_json::json!({"status":"complete","duration_ms":30000}),
        )
        .unwrap();
        restarted.drain(handoff).unwrap();
        assert_eq!(*queued.borrow(), ["100-3", "100-2"]);
        drop(restarted);
        Session::open(discovery(), cursor)
            .unwrap()
            .drain(|_| panic!("accepted recording was replayed"))
            .unwrap();
    }
    #[test]
    fn recording_source_busy_settings_script_contention_and_failed_spawn_do_not_acknowledge() {
        let f = Fixture::new();
        let settings_lock = store_io::lock(&f.root.path().join("local/config.lock")).unwrap();
        assert_eq!(
            f.handoff(Config::load_in(f.root.path()), || panic!(
                "busy settings dispatched"
            ))
            .unwrap(),
            Acceptance::Deferred
        );
        drop(settings_lock);
        let path = f.root.path().join("local/notes-script.lock");
        let held = store_io::lock(&path).unwrap();
        assert_eq!(
            f.handoff(Ok(f.config()), || match store_io::lock(&path) {
                Err(e) if matches!(e.raw_os_error(), Some(32 | 33)) => Ok(()),
                result => result.map(drop),
            })
            .unwrap(),
            Acceptance::Deferred
        );
        drop(held);
        let cursor = f.root.path().join("source.json");
        let mut session = Session::open(One(f.meetings.clone()), cursor.clone()).unwrap();
        assert_eq!(
            session.drain(|_| f
                .handoff(Ok(f.config()), || {
                    // The legacy asynchronous pipeline reports success even when spawn failed.
                    assert!(
                        std::process::Command::new(f.root.path().join("missing-helper"))
                            .spawn()
                            .is_err()
                    );
                    Ok(())
                })?
                .acknowledge()),
            Ok(())
        );
        assert!(!cursor.exists());
        drop(session);
        Session::open(One(f.meetings.clone()), cursor)
            .unwrap()
            .drain(|_| {
                f.handoff(Ok(f.config()), || {
                    store_io::write(
                        &f.meetings.join("100-1/english-summary.json"),
                        &serde_json::json!({"synthetic":"complete"}),
                    )
                })?
                .acknowledge()
            })
            .unwrap();
        assert_eq!(
            f.handoff(Ok(f.config()), || panic!("accepted work dispatched twice"))
                .unwrap(),
            Acceptance::AlreadyAccepted
        );
    }
    #[test]
    fn recording_source_deferred_first_id_does_not_block_later_id() {
        let f = Fixture::new();
        let later = f.meetings.join("100-2");
        fs::create_dir_all(&later).unwrap();
        for name in ["manifest.json", "transcript.ndjson"] {
            fs::copy(f.meetings.join("100-1").join(name), later.join(name)).unwrap();
        }
        store_io::write(
            &f.meetings.join("100-1/manifest.json"),
            &serde_json::json!({"status":"complete"}),
        )
        .unwrap();
        let cursor = f.root.path().join("source.json");
        let mut session = Session::open(
            core::recording_source::folders::Discovery(f.meetings.clone()),
            cursor.clone(),
        )
        .unwrap();
        let mut queued = Vec::new();
        session
            .drain(|meeting| {
                handoff_in(
                    f.root.path(),
                    &f.meetings,
                    meeting.id(),
                    Ok(f.config()),
                    || {
                        queued.push(meeting.id().to_owned());
                        store_io::write(
                            &f.meetings.join(meeting.id()).join("english-summary.json"),
                            &serde_json::json!({"synthetic":"complete"}),
                        )
                    },
                )?
                .acknowledge()
            })
            .unwrap();
        assert_eq!(queued, ["100-2"]);
        drop(session);
        let saved: serde_json::Value = store_io::read(&cursor).unwrap();
        assert_eq!(saved["delivered"], serde_json::json!(["100-2"]));
        Session::open(
            core::recording_source::folders::Discovery(f.meetings.clone()),
            cursor,
        )
        .unwrap()
        .drain(|meeting| {
            assert_eq!(meeting.id(), "100-1", "accepted later ID was replayed");
            Err(io::ErrorKind::WouldBlock.into())
        })
        .unwrap();
    }
    #[test]
    fn recording_source_crash_before_acceptance_remains_replayable() {
        let f = Fixture::new();
        let cursor = f.root.path().join("source.json");
        let mut session = Session::open(One(f.meetings.clone()), cursor.clone()).unwrap();
        assert_eq!(
            session.drain(|_| f
                .handoff(Ok(f.config()), || Err(io::ErrorKind::Interrupted.into()))?
                .acknowledge()),
            Err(Error::Dispatch(io::ErrorKind::Interrupted))
        );
        assert!(!cursor.exists());
    }
}
