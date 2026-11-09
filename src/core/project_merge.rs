//! Project merges are one atomic register in the existing device journal. No source is edited.
use super::{Meeting, Projects, TaskStore, memory::MemoryFile, store_io, sync_state};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

pub fn normalized_name(name: &str) -> String {
    name.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| {
            if w.len() > 4 && w.ends_with("ies") {
                format!("{}y", &w[..w.len() - 3])
            } else if w.len() > 3 && w.ends_with('s') && !w.ends_with("ss") && !w.ends_with("us") {
                w[..w.len() - 1].into()
            } else {
                w.into()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectMerge {
    pub id: String,
    pub into: String,
    pub active: bool,
    pub backup: PathBuf,
    pub checksum: String,
    pub previous: Option<Box<ProjectMerge>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignment_baseline: Option<BTreeMap<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_version: Option<u32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub area_changes: BTreeMap<String, (Option<Value>, Option<Value>)>,
}
const MOVED_AFTER_MERGE: &str = "Some items were moved after the merge, so Undo would not restore them. Move them back by hand or keep the merge.";
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MergePreview {
    pub from: String,
    pub into: String,
    pub meetings: Vec<String>,
    pub tasks: Vec<String>,
    pub memory: Vec<String>,
    pub fingerprint: String,
}
#[derive(Serialize, Deserialize)]
struct Backup {
    projects: Projects,
    preview: MergePreview,
    files: BTreeMap<PathBuf, Vec<u8>>,
}
fn invalid(message: &str) -> io::Error {
    store_io::invalid(message)
}
pub(super) fn digest(value: &impl Serialize) -> io::Result<String> {
    Ok(blake3::hash(&serde_json::to_vec(value)?)
        .to_hex()
        .to_string())
}
pub(super) fn scan() -> io::Result<Vec<Meeting>> {
    let (meetings, errors) = super::scan_sources(&super::meetings_dir(), &super::peers_dir());
    if !errors.is_empty() {
        return Err(invalid(
            "Some meetings could not be read; merge was not applied",
        ));
    }
    Ok(meetings)
}
impl TaskStore {
    pub fn preview_merge(&self, from: &str, into: &str) -> io::Result<MergePreview> {
        super::area_version::ensure(&self.root)?;
        self.merge_inventory(&self.load_projects()?, from, into)
    }
    fn merge_inventory(&self, p: &Projects, from: &str, into: &str) -> io::Result<MergePreview> {
        self.merge_snapshot(p, from, into)
            .map(|(preview, _)| preview)
    }
    fn merge_snapshot(
        &self,
        p: &Projects,
        from: &str,
        into: &str,
    ) -> io::Result<(MergePreview, BTreeMap<String, Value>)> {
        let from = p.resolve(from);
        let into = p.resolve(into);
        let names = super::project_names(p, &[]);
        if from == into || p.area_catalog()?.project(p, &from).is_none() || !names.contains(&into) {
            return Err(invalid("Choose two different existing projects"));
        }
        let mut planned = p.clone();
        planned.prepare_area_merge(&from, &into)?;
        let meetings = scan()?;
        let pipeline = super::load_pipeline(&meetings);
        // A malformed tag file must never be mistaken for an empty inventory.
        for m in &meetings {
            let path = m.folder.join("tasks.json");
            if path.try_exists()? {
                let _: super::TasksFile = store_io::read(&path)?;
            }
        }
        let overrides = self.load_overrides()?;
        let memory: MemoryFile = sync_state::load(&self.root, "memory/records.json")?;
        let tasks = super::build_tasks(&meetings, &pipeline, p, &overrides, &Default::default());
        let mut assignments = raw_assignments(p, &meetings, &pipeline, &overrides, &memory);
        let mut affected_keys = std::collections::BTreeSet::new();
        let affected = tasks.iter().filter(|t| t.project == from);
        for task in affected.clone() {
            affected_keys.insert(if task.manual {
                json!(["manual", task.meeting_id]).to_string()
            } else {
                json!(["task", task.meeting_id, task.index, task.task]).to_string()
            });
        }
        let task_items = affected
            .clone()
            .map(|t| format!("{}: {}", t.meeting_title, t.task))
            .collect();
        let meeting_items = meetings
            .iter()
            .filter(|m| {
                let assigned = p
                    .meeting_projects
                    .get(&m.id)
                    .map(|n| p.resolve(n))
                    .or_else(|| {
                        super::dominant_project(tasks.iter().filter(|t| t.meeting_id == m.id))
                    })
                    .or_else(|| {
                        pipeline
                            .get(&m.id)
                            .and_then(|f| f.meeting_project.as_ref())
                            .map(|n| p.resolve(n))
                    });
                assigned.as_deref() == Some(&from) || affected.clone().any(|t| t.meeting_id == m.id)
            })
            .map(|m| {
                affected_keys.insert(json!(["meeting", m.id]).to_string());
                m.title.clone()
            })
            .collect();
        for record in &memory.records {
            if record
                .project
                .as_deref()
                .is_some_and(|n| p.resolve(n) == from)
            {
                affected_keys.insert(json!(["memory", record.id]).to_string());
            }
        }
        for proposal in &memory.proposals {
            if proposal
                .proposal
                .project
                .as_deref()
                .is_some_and(|n| p.resolve(n) == from)
            {
                affected_keys.insert(json!(["proposal", proposal.proposal.id]).to_string());
            }
        }
        assignments.retain(|key, _| affected_keys.contains(key));
        let memory_items = memory
            .records
            .iter()
            .filter(|r| r.project.as_deref().is_some_and(|n| p.resolve(n) == from))
            .map(|r| {
                format!(
                    "{}: {}",
                    r.id,
                    r.text.as_deref().unwrap_or("Retained record")
                )
            })
            .chain(
                memory
                    .proposals
                    .iter()
                    .filter(|r| {
                        r.proposal
                            .project
                            .as_deref()
                            .is_some_and(|n| p.resolve(n) == from)
                    })
                    .map(|r| format!("Proposal: {}", r.proposal.text)),
            )
            .collect();
        let sources: Vec<_> = meetings.iter().map(|m| (&m.id, &m.summary)).collect();
        // BTreeMap makes the fingerprint stable across independent scans and restarts.
        let pipeline: BTreeMap<_, _> = pipeline.into_iter().collect();
        Ok((
            MergePreview {
                from,
                into,
                meetings: meeting_items,
                tasks: task_items,
                memory: memory_items,
                fingerprint: digest(&(p, overrides, memory, sources, pipeline))?,
            },
            assignments,
        ))
    }
    pub fn merge_projects(&self, preview: &MergePreview) -> io::Result<Projects> {
        sync_state::update_checked_validated(
            &self.root,
            "projects.json",
            |p: &mut Projects| {
                let (current, assignments) =
                    self.merge_snapshot(p, &preview.from, &preview.into)?;
                if current != *preview {
                    return Err(invalid("Items changed; review a fresh preview"));
                }
                let id = format!(
                    "{}-{}",
                    chrono::Utc::now().format("%Y%m%dT%H%M%S"),
                    sync_state::new_id(&self.root)?
                );
                let backup = self
                    .root
                    .join("local/project-merges")
                    .join(&id)
                    .join("backup.json");
                let files = project_files(&self.root)?;
                let snapshot = Backup {
                    projects: p.clone(),
                    preview: preview.clone(),
                    files,
                };
                store_io::write(&backup, &snapshot)?;
                verify_backup(&backup, &snapshot)?;
                // The isolated proof can terminate after backup, before the sole commit point.
                if super::project_merge::crash_requested(&self.root) {
                    std::process::exit(86);
                }
                if self.merge_inventory(p, &preview.from, &preview.into)? != *preview {
                    return Err(invalid(
                        "Items changed during backup; review a fresh preview",
                    ));
                }
                let previous = p.merges.get(&preview.from).cloned().map(Box::new);
                let prior = p.other.clone();
                p.prepare_area_merge(&preview.from, &preview.into)?;
                let keys: std::collections::BTreeSet<_> = prior
                    .keys()
                    .chain(p.other.keys())
                    .filter(|k| !k.starts_with("area-identity:"))
                    .cloned()
                    .collect();
                let area_changes = keys
                    .into_iter()
                    .filter(|k| prior.get(k) != p.other.get(k))
                    .map(|k| {
                        (
                            k.clone(),
                            (prior.get(&k).cloned(), p.other.get(&k).cloned()),
                        )
                    })
                    .collect();
                p.aliases.insert(preview.from.clone(), preview.into.clone());
                p.merges.insert(
                    preview.from.clone(),
                    ProjectMerge {
                        id,
                        into: preview.into.clone(),
                        active: true,
                        checksum: digest(&snapshot)?,
                        backup,
                        previous,
                        assignment_baseline: Some(assignments),
                        baseline_version: Some(2),
                        area_changes,
                    },
                );
                Ok(())
            },
            |before| {
                if self.merge_inventory(before, &preview.from, &preview.into)? != *preview {
                    return Err(invalid(
                        "Items changed before publication; review a fresh preview",
                    ));
                }
                Ok(())
            },
        )
    }
    pub fn local_project_merge_backup(
        &self,
        from: &str,
        merge: &ProjectMerge,
    ) -> io::Result<PathBuf> {
        let own = super::DeviceSettings::read_in(&self.root)?.id;
        if !super::area_ops::valid_receipt_id(&merge.id, &own)
            || sync_state::writer(
                &self.root,
                "projects.json",
                vec!["merges".into(), from.into()],
            )?
            .as_deref()
                != Some(own.as_str())
        {
            return Err(invalid(
                "Undo is available only on the device that wrote this merge",
            ));
        }
        let base = self.root.join("local/project-merges");
        let backup = base.join(&merge.id).join("backup.json");
        // Old local receipts stored this absolute path; never use the peer-provided path.
        if merge.backup != backup {
            return Err(invalid("Backup location does not match this device"));
        }
        store_io::reject_reparse(&backup)?;
        if !backup.canonicalize()?.starts_with(base.canonicalize()?) {
            return Err(invalid("Invalid Undo backup location"));
        }
        Ok(backup)
    }
    pub fn undo_project_merge(&self, from: &str, expected_id: &str) -> io::Result<Projects> {
        let projects = self.load_projects()?;
        let merge = projects
            .merges
            .get(from)
            .filter(|m| m.active && m.id == expected_id)
            .ok_or_else(|| invalid("Merge changed; reload before undoing"))?;
        self.local_project_merge_backup(from, merge)?;
        if merge.assignment_baseline.is_none() {
            return Err(invalid(MOVED_AFTER_MERGE));
        }
        let dependencies = self.undo_state(&projects)?.0;
        sync_state::update_checked_validated(
            &self.root,
            "projects.json",
            |p: &mut Projects| {
                let merge = p
                    .merges
                    .get(from)
                    .filter(|m| m.active && m.id == expected_id)
                    .cloned()
                    .ok_or_else(|| invalid("Merge changed; reload before undoing"))?;
                if p.aliases.get(from) != Some(&merge.into) {
                    return Err(invalid(
                        "This project was renamed after the merge. Undo was not applied.",
                    ));
                }
                verify_assignments(&merge, &self.undo_state(p)?.1)?;
                if merge
                    .area_changes
                    .iter()
                    .any(|(key, (_, after))| p.other.get(key) != after.as_ref())
                {
                    return Err(invalid(
                        "Area assignments changed after the merge. Undo was not applied.",
                    ));
                }
                let backup = self.local_project_merge_backup(from, &merge)?;
                let snapshot: Backup =
                    serde_json::from_slice(&store_io::read_bounded(&backup, 16 * 1024 * 1024)?)?;
                if digest(&snapshot)? != merge.checksum {
                    return Err(invalid("Backup checksum changed; undo was not applied"));
                }
                verify_backup(&backup, &snapshot)?;
                if snapshot.preview.from != from || snapshot.preview.into != merge.into {
                    return Err(invalid("Backup does not match this merge"));
                }
                match snapshot.projects.aliases.get(from) {
                    Some(alias) => {
                        p.aliases.insert(from.into(), alias.clone());
                    }
                    None => {
                        p.aliases.remove(from);
                    }
                }
                match merge.previous {
                    Some(previous) => {
                        p.merges.insert(from.into(), *previous);
                    }
                    None => {
                        p.merges.remove(from);
                    }
                }
                for (key, (before, _)) in merge.area_changes {
                    match before {
                        Some(value) => {
                            p.other.insert(key, value);
                        }
                        None => {
                            p.other.remove(&key);
                        }
                    }
                }
                Ok(())
            },
            |before| {
                let (current, assignments) = self.undo_state(before)?;
                if current != dependencies {
                    return Err(invalid("Items changed before undo; reload and try again"));
                }
                verify_assignments(&before.merges[from], &assignments)
            },
        )
    }
    pub(super) fn undo_state(
        &self,
        projects: &Projects,
    ) -> io::Result<(String, BTreeMap<String, Value>)> {
        let meetings = scan()?;
        let pipeline = super::load_pipeline(&meetings);
        for m in &meetings {
            let path = m.folder.join("tasks.json");
            if path.try_exists()? {
                let _: super::TasksFile = store_io::read(&path)?;
            }
        }
        let sources: Vec<_> = meetings.iter().map(|m| (&m.id, &m.summary)).collect();
        let memory: MemoryFile = sync_state::load(&self.root, "memory/records.json")?;
        let overrides = self.load_overrides()?;
        let assignments = raw_assignments(projects, &meetings, &pipeline, &overrides, &memory);
        let pipeline: BTreeMap<_, _> = pipeline.into_iter().collect();
        Ok((
            digest(&(overrides, memory, sources, pipeline))?,
            assignments,
        ))
    }
}
fn verify_assignments(merge: &ProjectMerge, current: &BTreeMap<String, Value>) -> io::Result<()> {
    let baseline = merge
        .assignment_baseline
        .as_ref()
        .ok_or_else(|| invalid(MOVED_AFTER_MERGE))?;
    if baseline.iter().any(|(key, value)| {
        let Some(current) = current.get(key) else {
            return true;
        };
        if merge.baseline_version.unwrap_or(1) == 1
            && key.starts_with("[\"task\",")
            && value.as_array().is_some_and(|v| v.len() == 3)
        {
            return current
                .as_array()
                .is_none_or(|v| v.len() < 3 || &v[..3] != value.as_array().unwrap().as_slice());
        }
        current != value
    }) {
        return Err(invalid(MOVED_AFTER_MERGE));
    }
    Ok(())
}
pub(super) fn raw_assignments(
    p: &Projects,
    meetings: &[Meeting],
    pipeline: &super::Pipeline,
    overrides: &super::Overrides,
    memory: &MemoryFile,
) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    for meeting in meetings {
        let file = pipeline.get(&meeting.id);
        out.insert(
            json!(["meeting", meeting.id]).to_string(),
            super::areas::meeting_baseline(p, pipeline, &meeting.id),
        );
        for (index, todo) in meeting.summary.todo.iter().enumerate() {
            let edit = overrides.get(&meeting.id, index);
            out.insert(
                json!(["task", meeting.id, index, todo.task]).to_string(),
                super::areas::assignment_baseline(
                    json!([
                        file.and_then(|f| f.tasks.iter().find(|t| t.i == index))
                            .map(|t| &t.project),
                        edit.and_then(|o| o.project.as_ref()),
                        edit.filter(|o| o.project.is_some())
                            .and_then(|o| o.task.as_ref()),
                        file.and_then(|f| f.meeting_project.as_ref())
                    ]),
                    edit.and_then(|o| o.assignment.as_ref()),
                ),
            );
        }
    }
    for task in &overrides.manual {
        out.insert(
            json!(["manual", format!("manual/{}", task.id)]).to_string(),
            super::areas::assignment_baseline(json!(task.project), task.assignment.as_ref()),
        );
    }
    for record in &memory.records {
        out.insert(
            json!(["memory", record.id]).to_string(),
            super::areas::assignment_baseline(json!(record.project), record.assignment.as_ref()),
        );
    }
    for proposal in &memory.proposals {
        out.insert(
            json!(["proposal", proposal.proposal.id]).to_string(),
            json!(proposal.proposal.project),
        );
    }
    out
}
pub(super) fn project_files(root: &Path) -> io::Result<BTreeMap<PathBuf, Vec<u8>>> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) -> io::Result<()> {
        if store_io::reject_reparse(dir).is_err() {
            return Ok(());
        }
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        for entry in entries {
            let Ok(entry) = entry else {
                continue;
            };
            if !entry.file_type()?.is_file() || entry.path().extension().is_none_or(|e| e != "json")
            {
                continue;
            }
            let Ok(bytes) = store_io::read_bounded(&entry.path(), 4 * 1024 * 1024) else {
                continue;
            };
            let Ok(value) = serde_json::from_slice::<Value>(
                bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes),
            ) else {
                continue;
            };
            if value.get("store").and_then(Value::as_str) == Some("projects.json") {
                out.insert(
                    entry
                        .path()
                        .strip_prefix(root)
                        .map_err(|_| invalid("Invalid backup source"))?
                        .into(),
                    bytes,
                );
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    let state = root.join("state");
    if state.try_exists()? {
        let project_store = blake3::hash(b"projects.json").to_hex().to_string();
        for owner in fs::read_dir(state)? {
            let owner = owner?;
            if owner.file_type()?.is_dir()
                && super::device::valid_id(&owner.file_name().to_string_lossy())
            {
                visit(root, &owner.path().join(&project_store), &mut out)?;
            }
        }
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type()?.is_file()
            && (name == "projects.json" || name.starts_with("projects.sync-conflict-"))
            && let Ok(bytes) = store_io::read_bounded(&entry.path(), 16 * 1024 * 1024)
            && serde_json::from_slice::<Value>(
                bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes),
            )
            .is_ok()
        {
            out.insert(PathBuf::from(name), bytes);
        }
    }
    Ok(out)
}
fn verify_backup(path: &Path, expected: &Backup) -> io::Result<()> {
    let bytes = store_io::read_bounded(path, 16 * 1024 * 1024)?;
    let backup: Backup = serde_json::from_slice(&bytes)?;
    if digest(&backup)? != digest(expected)? {
        return Err(invalid("Backup verification failed"));
    }
    for bytes in backup.files.values() {
        let _: serde_json::Value =
            serde_json::from_slice(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes))?;
    }
    Ok(())
}
fn crash_requested(root: &Path) -> bool {
    // Deliberately unavailable in installed/release builds or outside the isolated proof folder.
    cfg!(debug_assertions)
        && root.starts_with(Path::new(
            "E:/Personal-Projects/reviews/project-merge-proof",
        ))
        && std::env::var_os("SIDENOTE_MERGE_CRASH_AFTER_BACKUP").is_some()
}
/// Isolated debug proof rendezvous. Release builds and ordinary data folders never wait.
pub fn proof_pause(root: &Path, stage: &str) -> io::Result<()> {
    if !cfg!(debug_assertions)
        || !root.starts_with(Path::new(
            "E:/Personal-Projects/reviews/project-merge-proof",
        ))
    {
        return Ok(());
    }
    let gate = root.join(format!("local/proof-{stage}-pause"));
    if !gate.try_exists()? {
        return Ok(());
    }
    fs::write(gate.with_extension("reached"), b"ready")?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while gate.try_exists()? {
        if std::time::Instant::now() >= deadline {
            return Err(invalid("Proof rendezvous timed out"));
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    Ok(())
}
