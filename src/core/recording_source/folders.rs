//! Shared completed-folder discovery. Configuration is explicit; no API credentials are read.
use super::{BTreeMap, BTreeSet, CatchUp, CompletedMeeting, Error, Source, revision};
use crate::core::meeting_router::inputs::MeetingScan;
use std::{
    io,
    path::{Component, PathBuf},
};

pub struct Discovery {
    root: PathBuf,
    state: Option<PathBuf>,
    validated: BTreeMap<String, (String, bool)>,
}
impl Discovery {
    pub(crate) fn configured(root: PathBuf) -> Self {
        Self {
            root,
            state: None,
            validated: BTreeMap::new(),
        }
    }
    #[cfg(any(windows, test))]
    pub(crate) fn suppress_in(mut self, state: PathBuf) -> Self {
        self.state = Some(state);
        self
    }

    /// An explicitly selected meetings folder, e.g. Sidenote's --meetings-dir.
    pub fn new(meetings: PathBuf) -> Result<Self, Error> {
        if !meetings.is_absolute() || meetings.components().any(|c| c == Component::ParentDir) {
            return Err(Error::InvalidInput);
        }
        Ok(Self::configured(meetings))
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
        self.catch_up().map(|(completed, _)| completed)
    }
    fn catch_up(&mut self) -> Result<CatchUp, Error> {
        self.catch_up_excluding(&BTreeMap::new())
    }
    fn catch_up_excluding(
        &mut self,
        delivered: &BTreeMap<String, String>,
    ) -> Result<CatchUp, Error> {
        let _stage = crate::core::idle_pass::Span::begin("source_discovery");
        let suppressed = self
            .state
            .as_deref()
            .map(crate::core::meeting_delete::suppressed_ids)
            .transpose()
            .map_err(|e| Error::Input(e.kind()))?
            .unwrap_or_default();
        let scan = MeetingScan::open(&self.root).map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => Error::Offline,
            kind => Error::Input(kind),
        })?;
        let root_meta = std::fs::metadata(&self.root).map_err(|e| Error::Input(e.kind()))?;
        let mut completed = Vec::new();
        let mut ids = BTreeSet::new();
        for name in &scan.names {
            let Some(id) = name.to_str() else {
                continue;
            };
            ids.insert(id.to_owned());
            if suppressed.contains(id) {
                continue;
            }
            let before = if let Some(directory) = scan.metadata.get(name) {
                revision::input_with_metadata(&self.root.join(id), &root_meta, directory)
            } else {
                revision::input_with_root(&self.root.join(id), &root_meta)
            };
            let Ok(before) = before else {
                continue;
            };
            if delivered.get(id) == Some(&before) {
                continue;
            }
            let folder = self.root.join(id);
            let Ok(key) = revision::validation(&folder, &before) else {
                continue;
            };
            let cached = self
                .validated
                .get(id)
                .filter(|(revision, _)| revision == &key)
                .map(|(_, valid)| *valid);
            let valid = match cached {
                Some(valid) => valid,
                None => match CompletedMeeting::capture_in(&scan, id) {
                    Ok(_) => true,
                    Err(Error::InvalidInput | Error::Incomplete) => false,
                    Err(Error::Input(_)) => continue,
                    Err(error) => return Err(error),
                },
            };
            if cached.is_some()
                || (revision::input_with_root(&folder, &root_meta)
                    .is_ok_and(|after| after == before)
                    && revision::validation(&folder, &before).is_ok_and(|after| after == key))
            {
                if self.validated.len() < crate::core::file_reads::ENTRIES_MAX
                    || self.validated.contains_key(id)
                {
                    self.validated.insert(id.to_owned(), (key, valid));
                }
                if valid {
                    completed.push(CompletedMeeting(id.to_owned(), before));
                }
            }
        }
        if !scan
            .root_matches(&self.root)
            .map_err(|e| Error::Input(e.kind()))?
        {
            return Err(Error::Input(io::ErrorKind::Interrupted));
        }
        self.validated.retain(|id, _| ids.contains(id));
        completed.sort_by(|a, b| a.id().cmp(b.id()));
        Ok((completed, Some(ids)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::recording_source::Session;
    use std::fs;

    #[test]
    fn suppressed_hex_input_is_not_rediscovered_until_restore() {
        use crate::core::meeting_delete::{Eligibility, Storage};
        let root = tempfile::tempdir().unwrap();
        let meetings = root.path().join("meetings");
        let state = root.path().join("state");
        let folder = meetings.join("100-1");
        fs::create_dir_all(&folder).unwrap();
        fs::create_dir(&state).unwrap();
        fs::write(folder.join("manifest.json"), br#"{"status":"complete"}"#).unwrap();
        fs::write(folder.join("transcript.ndjson"), br#"{"text":"Synthetic"}"#).unwrap();
        let mut discovery = Discovery::configured(meetings.clone()).suppress_in(state.clone());
        assert_eq!(discovery.poll().unwrap().len(), 1);
        let storage = Storage::new(&meetings, &state).unwrap();
        let transaction = storage
            .delete(
                "100-1",
                String::new(),
                1,
                |_| Ok(Eligibility::Ready),
                |_| Ok(()),
            )
            .unwrap();
        assert!(folder.join("transcript.ndjson").exists());
        assert!(discovery.poll().unwrap().is_empty());
        storage.restore(&transaction).unwrap();
        assert_eq!(discovery.poll().unwrap().len(), 1);
    }

    #[test]
    fn unchanged_unacknowledged_folders_do_not_reread_inputs() {
        let root = tempfile::tempdir().unwrap();
        let meetings = root.path().join("meetings");
        let folder = meetings.join("100-1");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("manifest.json"), br#"{"status":"complete"}"#).unwrap();
        fs::write(
            folder.join("transcript.ndjson"),
            br#"{"text":"Synthetic words"}"#,
        )
        .unwrap();
        let mut session = Session::open(
            Discovery::configured(meetings),
            root.path().join("cursor.json"),
        )
        .unwrap();
        let calls = std::cell::Cell::new(0);
        let mut defer = |_: &CompletedMeeting| {
            calls.set(calls.get() + 1);
            Err(io::ErrorKind::WouldBlock.into())
        };
        session.drain(&mut defer).unwrap();
        let reads = crate::core::meeting_router::inputs::body_reads();
        session.drain(&mut defer).unwrap();
        assert_eq!(calls.get(), 2);
        assert_eq!(crate::core::meeting_router::inputs::body_reads(), reads);
        fs::write(
            folder.join("audio-evidence.json"),
            b"invalid synthetic JSON",
        )
        .unwrap();
        session.drain(&mut defer).unwrap();
        assert_eq!(calls.get(), 2);
        fs::remove_file(folder.join("audio-evidence.json")).unwrap();
        fs::write(folder.join("transcript.ndjson"), b"invalid synthetic JSON").unwrap();
        session.drain(&mut defer).unwrap();
        let reads = crate::core::meeting_router::inputs::body_reads();
        session.drain(&mut defer).unwrap();
        assert_eq!(calls.get(), 2);
        assert_eq!(crate::core::meeting_router::inputs::body_reads(), reads);
        fs::write(
            folder.join("transcript.ndjson"),
            br#"{"text":"Synthetic replacement"}"#,
        )
        .unwrap();
        session
            .drain(|_| {
                calls.set(calls.get() + 1);
                Ok(())
            })
            .unwrap();
        assert_eq!(calls.get(), 3);
        session
            .drain(|_| panic!("acknowledged input replayed"))
            .unwrap();
    }

    #[test]
    fn same_name_rewrite_and_restore_are_delivered_after_restart() {
        let (root, meetings, cursor) = super::super::tests::fixture();
        let open =
            || Session::open(Discovery::configured(meetings.clone()), cursor.clone()).unwrap();
        open().drain(|_| Ok(())).unwrap();
        for restore in [false, true] {
            if restore {
                fs::rename(meetings.join("100-1"), root.path().join("saved")).unwrap();
                fs::create_dir(meetings.join("100-1")).unwrap();
                fs::copy(
                    root.path().join("saved/manifest.json"),
                    meetings.join("100-1/manifest.json"),
                )
                .unwrap();
            }
            fs::write(
                meetings.join("100-1/transcript.ndjson"),
                serde_json::json!({"text":format!("Synthetic replacement {restore}")}).to_string(),
            )
            .unwrap();
            let mut delivered = Vec::new();
            let mut restarted = open();
            restarted
                .drain(|m| {
                    delivered.push(m.id().to_owned());
                    Ok(())
                })
                .unwrap();
            assert_eq!(
                delivered,
                ["100-1"],
                "same-name input skipped, restore={restore}"
            );
            restarted
                .drain(|_| panic!("unchanged input replayed"))
                .unwrap();
        }
    }

    #[test]
    fn folder_scan_and_repeated_event_wakeups_share_the_durable_cursor() {
        let (root, meetings, cursor) = super::super::tests::fixture();
        fs::remove_file(meetings.join("99-1/transcript.ndjson")).unwrap();
        let mut session =
            Session::open(Discovery::configured(meetings.clone()), cursor.clone()).unwrap();
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
        let mut restarted = Session::open(Discovery::configured(meetings), cursor).unwrap();
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
        let mut session =
            Session::open(Discovery::configured(meetings.clone()), cursor.clone()).unwrap();
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
