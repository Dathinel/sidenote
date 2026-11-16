//! Opt-in synthetic completion fixture; never invokes a host helper.
use super::{
    Helper, JobState, NotesSettings,
    helper::{Failure, Run},
    queue,
};
use crate::core;
use std::{collections::VecDeque, fs, io, time::Duration};

pub fn seed() -> io::Result<()> {
    let root = core::state_dir();
    let meetings = core::meetings_dir();
    for path in [&root, &meetings, &core::peers_dir()] {
        if !core::is_proof_path(path, "sidenote-idle-cpu-proof")
            && !core::is_proof_path(path, "idle-cpu-proof")
        {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
    }
    let settings = NotesSettings {
        helper: Some(Helper::Claude),
        enabled_at_ms: Some(1),
        ..NotesSettings::default()
    };
    let mut helper = Synthetic(VecDeque::new());
    let mut ids = Vec::new();
    for entry in fs::read_dir(&meetings)? {
        let id = entry?.file_name().to_string_lossy().into_owned();
        queue::enqueue_in(&root, &meetings, &id, None, &settings)?;
        helper.0.push_back(serde_json::json!({"title":"Synthetic production completion", "context":[], "topics":[], "todo":[], "to_settle":[]}).to_string());
        helper
            .0
            .push_back(r#"{"meeting_project":null,"tasks":[]}"#.into());
        queue::drain_locked(&root, &meetings, &mut helper, || Ok(settings.clone()))?;
        ids.push(id);
    }
    queue::drain_locked(&root, &meetings, &mut helper, || Ok(settings.clone()))?;
    for id in &ids {
        if !matches!(
            queue::read(&root, id)?.map(|job| job.state),
            Some(JobState::Done { .. })
        ) || !meetings.join(id).join("english-summary.json").is_file()
        {
            return Err(io::Error::other(
                "synthetic completion did not retain Done notes",
            ));
        }
    }
    if !helper.0.is_empty() {
        return Err(io::Error::other("synthetic helper replies unused"));
    }
    println!(
        "production fixture: {} retained Done jobs; synthetic notes and task steps",
        ids.len()
    );
    Ok(())
}

struct Synthetic(VecDeque<String>);
impl Run for Synthetic {
    fn installed(&self, _: Helper) -> bool {
        true
    }
    fn run(&mut self, _: Helper, _: &str, _: String, _: Duration) -> Result<String, Failure> {
        self.0.pop_front().ok_or(Failure::BadOutput)
    }
}
