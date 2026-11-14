//! Shared completed-folder discovery. Configuration is explicit; no API credentials are read.
use super::{BTreeSet, CompletedMeeting, Error, Source};
use crate::core::meeting_router::inputs::MeetingScan;
use std::{
    io,
    path::{Component, PathBuf},
};

pub struct Discovery(pub(crate) PathBuf, Option<BTreeSet<String>>);
impl Discovery {
    /// An explicitly selected meetings folder, e.g. Sidenote's --meetings-dir.
    pub fn new(meetings: PathBuf) -> Result<Self, Error> {
        if !meetings.is_absolute() || meetings.components().any(|c| c == Component::ParentDir) {
            return Err(Error::InvalidInput);
        }
        Ok(Self(meetings, None))
    }

    /// Hex 7681742 stores meetings below its application support directory on both OSes.
    /// Pass the configured HEX_APPLICATION_SUPPORT_DIR, or the caller's selected data root
    /// (on Linux, XDG_DATA_HOME/hex or HOME/.local/share/hex). No environment is inferred here.
    pub fn hex_support(application_support: PathBuf) -> Result<Self, Error> {
        Self::new(application_support.join("meetings"))
    }
}
impl Source for Discovery {
    fn poll(&mut self) -> Result<Vec<CompletedMeeting>, Error> {
        let scan = MeetingScan::open(&self.0).map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => Error::Offline,
            kind => Error::Input(kind),
        })?;
        let mut completed = Vec::new();
        let mut ids = BTreeSet::new();
        for name in &scan.names {
            let Some(id) = name.to_str() else {
                continue;
            };
            ids.insert(id.to_owned());
            match CompletedMeeting::capture_in(&scan, id) {
                Ok(meeting) => completed.push(meeting),
                // A deleted, growing or temporarily locked folder cannot starve other meetings.
                Err(Error::InvalidInput | Error::Incomplete | Error::Input(_)) => {}
                Err(error) => return Err(error),
            }
        }
        completed.sort_by(|a, b| a.id().cmp(b.id()));
        self.1 = Some(ids);
        Ok(completed)
    }
    fn retained_ids(&self) -> Option<&BTreeSet<String>> {
        self.1.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::recording_source::Session;
    use std::fs;

    #[test]
    fn folder_scan_and_repeated_event_wakeups_share_the_durable_cursor() {
        let (root, meetings, cursor) = super::super::tests::fixture();
        fs::remove_file(meetings.join("99-1/transcript.ndjson")).unwrap();
        let mut session = Session::open(Discovery(meetings.clone(), None), cursor.clone()).unwrap();
        let mut queued = Vec::new();
        for _ in 0..3 {
            session
                .drain(|m| {
                    queued.push(m.id().to_owned());
                    Ok(())
                })
                .unwrap();
        }
        drop(session);
        fs::write(
            meetings.join("99-1/transcript.ndjson"),
            b"{\"text\":\"synthetic final\"}",
        )
        .unwrap();
        let mut restarted = Session::open(Discovery(meetings, None), cursor).unwrap();
        restarted
            .drain(|m| {
                queued.push(m.id().to_owned());
                Ok(())
            })
            .unwrap();
        assert_eq!(queued, ["100-1", "99-1"]);
        assert!(root.path().join("state/source.json").is_file());
    }

    #[test]
    fn unavailable_root_is_typed_and_does_not_advance_the_cursor() {
        let (root, meetings, cursor) = super::super::tests::fixture();
        let mut session = Session::open(Discovery(meetings.clone(), None), cursor.clone()).unwrap();
        fs::rename(&meetings, root.path().join("saved")).unwrap();
        assert_eq!(session.drain(|_| panic!("offline")), Err(Error::Offline));
        assert!(!cursor.exists());
        fs::rename(root.path().join("saved"), meetings).unwrap();
        let mut count = 0;
        session
            .drain(|_| {
                count += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn replay_after_enqueue_keeps_the_existing_router_job_generation() {
        use crate::core::{self, meeting_router::jobs, recording_source};
        const ID: &str = "9944771122-887766";
        struct One;
        impl Source for One {
            fn poll(&mut self) -> Result<Vec<CompletedMeeting>, Error> {
                Ok(vec![CompletedMeeting::capture(&core::meetings_dir(), ID)?])
            }
        }
        let folder = core::meetings_dir().join(ID);
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            folder.join("manifest.json"),
            br#"{"status":"complete","duration_ms":30000}"#,
        )
        .unwrap();
        fs::write(
            folder.join("transcript.ndjson"),
            serde_json::json!({"text":vec!["synthetic";25].join(" ")}).to_string(),
        )
        .unwrap();
        let root = tempfile::tempdir().unwrap();
        let cursor = root.path().join("source.json");
        let pending = core::state_dir().join(format!("local/router-queue/{ID}.json"));
        let mut first = Session::open(One, cursor.clone()).unwrap();
        assert_eq!(
            first.drain(|m| {
                jobs::enqueue(m.id()).map_err(io::Error::other)?;
                Err(io::ErrorKind::Interrupted.into())
            }),
            Err(recording_source::Error::Dispatch(
                io::ErrorKind::Interrupted
            ))
        );
        drop(first);
        let before = fs::read(&pending).unwrap();
        let mut restarted = Session::open(One, cursor).unwrap();
        restarted
            .drain(|m| jobs::enqueue(m.id()).map_err(io::Error::other))
            .unwrap();
        assert_eq!(fs::read(pending).unwrap(), before);
        fs::remove_dir_all(folder).unwrap();
    }
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests;
