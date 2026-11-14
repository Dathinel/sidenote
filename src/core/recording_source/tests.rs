use super::*;
use std::{cell::Cell, fs, rc::Rc};

struct Fixture {
    meetings: PathBuf,
    stopped: Rc<Cell<usize>>,
}
impl Source for Fixture {
    fn poll(&mut self) -> Result<Vec<CompletedMeeting>, Error> {
        Ok(["100-1", "100-1", "99-1"]
            .into_iter()
            .filter_map(|id| CompletedMeeting::capture(&self.meetings, id).ok())
            .collect())
    }
    fn stop(&mut self) {
        self.stopped.set(self.stopped.get() + 1);
    }
}
pub(super) fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let meetings = root.path().join("meetings");
    for id in ["100-1", "99-1"] {
        let folder = meetings.join(id);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("manifest.json"), br#"{"status":"complete"}"#).unwrap();
        fs::write(
            folder.join("transcript.ndjson"),
            b"{\"text\":\"Synthetic fixture\"}\n",
        )
        .unwrap();
    }
    let cursor = root.path().join("state/source.json");
    (root, meetings, cursor)
}
fn source(meetings: &Path) -> Fixture {
    Fixture {
        meetings: meetings.into(),
        stopped: Rc::new(Cell::new(0)),
    }
}

#[test]
fn restart_deduplicates_and_retries_partial_older_meetings() {
    let (_root, meetings, cursor) = fixture();
    let partial = meetings.join("99-1/transcript.ndjson");
    fs::write(&partial, b"{\"text\":\"unfinished").unwrap();
    let mut jobs = Vec::new();
    let mut first = Session::open(source(&meetings), cursor.clone()).unwrap();
    first
        .drain(|m| {
            jobs.push(m.id().to_owned());
            Ok(())
        })
        .unwrap();
    drop(first);
    assert_eq!(jobs, ["100-1"]);
    let mut restarted = Session::open(source(&meetings), cursor).unwrap();
    restarted
        .drain(|_| panic!("partial input after restart"))
        .unwrap();
    fs::write(partial, b"{\"text\":\"Synthetic complete\"}\n").unwrap();
    for _ in 0..2 {
        restarted
            .drain(|m| {
                jobs.push(m.id().to_owned());
                Ok(())
            })
            .unwrap();
    }
    assert_eq!(jobs, ["100-1", "99-1"]);
}

#[test]
fn crash_after_durable_enqueue_replays_without_duplicate_job() {
    let (root, meetings, cursor) = fixture();
    let job = root.path().join("state/job.json");
    let enqueue = |m: &CompletedMeeting| -> io::Result<()> {
        let mut jobs: BTreeSet<String> = store_io::read(&job)?;
        jobs.insert(m.id().into());
        store_io::write(&job, &jobs)
    };
    let mut first = Session::open(source(&meetings), cursor.clone()).unwrap();
    assert_eq!(
        first.drain(|m| {
            enqueue(m)?;
            Err(io::ErrorKind::Interrupted.into())
        }),
        Err(Error::Dispatch(io::ErrorKind::Interrupted))
    );
    drop(first);
    let mut restarted = Session::open(source(&meetings), cursor).unwrap();
    restarted.drain(enqueue).unwrap();
    let jobs: BTreeSet<String> = store_io::read(&job).unwrap();
    assert_eq!(jobs, BTreeSet::from(["100-1".into(), "99-1".into()]));
}

#[test]
fn sessions_opened_before_ack_reload_the_locked_cursor() {
    let (_root, meetings, cursor) = fixture();
    let mut first = Session::open(source(&meetings), cursor.clone()).unwrap();
    let mut second = Session::open(source(&meetings), cursor).unwrap();
    let mut calls = 0;
    first
        .drain(|_| {
            calls += 1;
            Ok(())
        })
        .unwrap();
    second
        .drain(|_| panic!("stale cursor created a duplicate"))
        .unwrap();
    assert_eq!(calls, 2);
}

#[test]
fn silence_is_complete_but_missing_and_partial_transcripts_are_not() {
    let (_root, meetings, cursor) = fixture();
    let path = meetings.join("100-1/transcript.ndjson");
    for bytes in [b"".as_slice(), b" \r\n\t", b"\xef\xbb\xbf\n"] {
        fs::write(&path, bytes).unwrap();
        assert_eq!(
            CompletedMeeting::capture(&meetings, "100-1").unwrap().id(),
            "100-1"
        );
    }
    let mut session = Session::open(source(&meetings), cursor.clone()).unwrap();
    let mut calls = 0;
    session
        .drain(|_| {
            calls += 1;
            Ok(())
        })
        .unwrap();
    assert_eq!(calls, 2);
    drop(session);
    Session::open(source(&meetings), cursor)
        .unwrap()
        .drain(|_| panic!("silent completion was not acknowledged"))
        .unwrap();
    fs::remove_file(&path).unwrap();
    assert!(matches!(
        CompletedMeeting::capture(&meetings, "100-1"),
        Err(Error::Input(_))
    ));
    fs::write(path, b"{\"text\":\"partial").unwrap();
    assert!(matches!(
        CompletedMeeting::capture(&meetings, "100-1"),
        Err(Error::Incomplete)
    ));
}

#[test]
fn lifecycle_corruption_deleted_and_incomplete_inputs_fail_closed() {
    let (_root, meetings, cursor) = fixture();
    for id in ["../100-1", "missing", "999-1"] {
        assert!(CompletedMeeting::capture(&meetings, id).is_err());
    }
    fs::write(
        meetings.join("99-1/manifest.json"),
        br#"{"status":"recording"}"#,
    )
    .unwrap();
    assert!(matches!(
        CompletedMeeting::capture(&meetings, "99-1"),
        Err(Error::Incomplete)
    ));
    let fixture = source(&meetings);
    let stopped = fixture.stopped.clone();
    let mut session = Session::open(fixture, cursor.clone()).unwrap();
    session.stop();
    session.stop();
    assert_eq!(session.drain(|_| panic!("stopped")), Err(Error::Stopped));
    drop(session);
    assert_eq!(stopped.get(), 1);
    fs::create_dir_all(cursor.parent().unwrap()).unwrap();
    fs::write(&cursor, b"partial cursor").unwrap();
    assert!(matches!(
        Session::open(source(&meetings), cursor),
        Err(Error::Cursor(_))
    ));
}
