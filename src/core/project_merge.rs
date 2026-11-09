//! Project merges are one atomic register in the existing device journal. No source is edited.
use super::{Meeting, Projects, TaskStore, memory::MemoryFile, store_io, sync_state};
use serde::{Deserialize, Serialize};
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
}
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
fn digest(value: &impl Serialize) -> io::Result<String> {
    Ok(blake3::hash(&serde_json::to_vec(value)?)
        .to_hex()
        .to_string())
}
fn scan() -> io::Result<Vec<Meeting>> {
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
        self.merge_inventory(&self.load_projects()?, from, into)
    }
    fn merge_inventory(&self, p: &Projects, from: &str, into: &str) -> io::Result<MergePreview> {
        let from = p.resolve(from);
        let into = p.resolve(into);
        let names = super::project_names(p, &[]);
        if from == into || !names.contains(&from) || !names.contains(&into) {
            return Err(invalid("Choose two different existing projects"));
        }
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
        let affected = tasks.iter().filter(|t| t.project == from);
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
            .map(|m| m.title.clone())
            .collect();
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
        Ok(MergePreview {
            from,
            into,
            meetings: meeting_items,
            tasks: task_items,
            memory: memory_items,
            fingerprint: digest(&(p, overrides, memory, sources, pipeline))?,
        })
    }
    pub fn merge_projects(&self, preview: &MergePreview) -> io::Result<Projects> {
        sync_state::update_checked_validated(
            &self.root,
            "projects.json",
            |p: &mut Projects| {
                if self.merge_inventory(p, &preview.from, &preview.into)? != *preview {
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
    pub fn undo_project_merge(&self, from: &str, expected_id: &str) -> io::Result<Projects> {
        let dependencies = self.undo_dependencies()?;
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
                let backup = self
                    .root
                    .join("local/project-merges")
                    .join(&merge.id)
                    .join("backup.json");
                if merge.backup != backup {
                    return Err(invalid("Backup location does not match this device"));
                }
                let snapshot: Backup = serde_json::from_slice(&fs::read(&backup)?)?;
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
                Ok(())
            },
            |_| {
                if self.undo_dependencies()? != dependencies {
                    return Err(invalid("Items changed before undo; reload and try again"));
                }
                Ok(())
            },
        )
    }
    fn undo_dependencies(&self) -> io::Result<String> {
        let meetings = scan()?;
        let pipeline: BTreeMap<_, _> = super::load_pipeline(&meetings).into_iter().collect();
        for m in &meetings {
            let path = m.folder.join("tasks.json");
            if path.try_exists()? {
                let _: super::TasksFile = store_io::read(&path)?;
            }
        }
        let sources: Vec<_> = meetings.iter().map(|m| (&m.id, &m.summary)).collect();
        let memory: MemoryFile = sync_state::load(&self.root, "memory/records.json")?;
        digest(&(self.load_overrides()?, memory, sources, pipeline))
    }
}
fn project_files(root: &Path) -> io::Result<BTreeMap<PathBuf, Vec<u8>>> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) -> io::Result<()> {
        if !dir.try_exists()? {
            return Ok(());
        }
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                visit(root, &entry.path(), out)?;
            } else if entry.file_type()?.is_file()
                && entry.path().extension().is_some_and(|e| e == "json")
            {
                let bytes = fs::read(entry.path())?;
                let value: serde_json::Value =
                    serde_json::from_slice(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes))?;
                if value.get("store").and_then(|s| s.as_str()) == Some("projects.json") {
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
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    let state = root.join("state");
    if state.try_exists()? {
        let project_store = blake3::hash(b"projects.json").to_hex().to_string();
        for owner in fs::read_dir(state)? {
            let owner = owner?;
            if owner.file_type()?.is_dir() {
                visit(root, &owner.path().join(&project_store), &mut out)?;
            }
        }
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type()?.is_file()
            && (name == "projects.json" || name.starts_with("projects.sync-conflict-"))
        {
            out.insert(PathBuf::from(name), fs::read(entry.path())?);
        }
    }
    Ok(out)
}
fn verify_backup(path: &Path, expected: &Backup) -> io::Result<()> {
    let bytes = fs::read(path)?;
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
