//! Sidenote owns the meeting; Hex owns each recording part.
use super::{file_reads, store_io};
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Recording,
    #[default]
    Paused,
    Ended,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Part {
    pub id: String,
    pub started_ms: i64,
    pub duration_ms: u64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Meeting {
    pub id: String,
    pub parts: Vec<Part>,
    pub phase: Phase,
    pub notes_parts: usize,
    pub pending_start_ms: Option<i64>,
    pub requested_parts: usize,
}
fn valid(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 100
        && id.bytes().any(|b| b.is_ascii_digit())
        && id.bytes().all(|b| b.is_ascii_digit() || b == b'-')
}
fn path(root: &Path, id: &str) -> io::Result<PathBuf> {
    if !valid(id) {
        return Err(store_io::invalid("Invalid meeting id"));
    }
    Ok(root.join("local/meeting-parts").join(format!("{id}.json")))
}
pub fn read(root: &Path, id: &str) -> io::Result<Option<Meeting>> {
    let path = path(root, id)?;
    match file_reads::read(&path, file_reads::SUMMARY_MAX) {
        Ok(raw) => {
            let m: Meeting = serde_json::from_slice(&raw)?;
            if m.id != id || m.parts.is_empty() || m.parts.iter().any(|p| !valid(&p.id)) {
                return Err(store_io::invalid("Invalid meeting parts"));
            }
            Ok(Some(m))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
pub fn list(root: &Path) -> io::Result<Vec<Meeting>> {
    let entries = match file_reads::dir_iter(&root.join("local/meeting-parts")) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut result = Vec::new();
    for entry in entries.take(file_reads::ENTRIES_MAX) {
        let p = entry?.path();
        if p.extension().is_some_and(|e| e == "json") {
            let id = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if let Some(m) = read(root, id)? {
                result.push(m);
            }
        }
    }
    Ok(result)
}
pub fn owner(root: &Path, part: &str) -> io::Result<Option<Meeting>> {
    Ok(list(root)?
        .into_iter()
        .find(|m| m.parts.iter().any(|p| p.id == part)))
}
pub fn inputs(root: &Path) -> PathBuf {
    root.join("local/meeting-inputs")
}
pub fn source(root: &Path, hex: &Path, id: &str) -> PathBuf {
    if read(root, id).ok().flatten().is_some() {
        inputs(root)
    } else {
        hex.to_owned()
    }
}
impl Meeting {
    pub fn write_view(&self, root: &Path) -> io::Result<()> {
        let folder = inputs(root).join(&self.id);
        store_io::write(
            &folder.join("manifest.json"),
            &serde_json::json!({"schema_version":1,
            "status":"complete","created_at_ms":self.parts[0].started_ms,"duration_ms":self.elapsed_ms()}),
        )
    }
    pub fn save(&self, root: &Path) -> io::Result<()> {
        store_io::write(&path(root, &self.id)?, self)
    }
    pub fn begin(root: &Path, id: &str, started_ms: i64) -> io::Result<Self> {
        if let Some(m) = read(root, id)? {
            return Ok(m);
        }
        let m = Self {
            id: id.into(),
            parts: vec![Part {
                id: id.into(),
                started_ms,
                duration_ms: 0,
            }],
            phase: Phase::Recording,
            notes_parts: 0,
            pending_start_ms: None,
            requested_parts: 0,
        };
        m.save(root)?;
        m.write_view(root)?;
        Ok(m)
    }
    pub fn resume(&mut self, root: &Path, id: &str, started_ms: i64) -> io::Result<()> {
        path(root, id)?;
        if !self.parts.iter().any(|p| p.id == id) {
            self.parts.push(Part {
                id: id.into(),
                started_ms,
                duration_ms: 0,
            });
        }
        self.phase = Phase::Recording;
        self.pending_start_ms = None;
        self.save(root)
    }
    pub fn pause(&mut self, root: &Path) -> io::Result<()> {
        self.phase = Phase::Paused;
        self.save(root)?;
        self.write_view(root)
    }
    pub fn stale(&self) -> bool {
        self.notes_parts > 0 && self.parts.len() > self.notes_parts
    }
    pub fn elapsed_ms(&self) -> u64 {
        self.parts.iter().map(|p| p.duration_ms).sum()
    }
    pub fn refresh(&mut self, hex: &Path) -> io::Result<()> {
        for part in &mut self.parts {
            let raw = file_reads::read(&hex.join(&part.id).join("manifest.json"), 1024 * 1024)?;
            let m: serde_json::Value = serde_json::from_slice(&raw)?;
            if let Some(duration) = m["duration_ms"].as_u64() {
                part.duration_ms = duration;
            }
        }
        Ok(())
    }
    /// Recovery never sends a recorder command or starts notes.
    pub fn restore(mut self, root: &Path, hex: &Path) -> io::Result<Self> {
        if let Some(since) = self.pending_start_ms {
            let mut found = Vec::new();
            for entry in file_reads::dir_iter(hex)?.take(file_reads::ENTRIES_MAX) {
                let entry = entry?;
                let id = entry.file_name().to_string_lossy().into_owned();
                if !valid(&id) || self.parts.iter().any(|p| p.id == id) {
                    continue;
                }
                let raw = file_reads::read(&entry.path().join("manifest.json"), 1024 * 1024)?;
                let m: serde_json::Value = serde_json::from_slice(&raw)?;
                if m["created_at_ms"]
                    .as_i64()
                    .is_some_and(|at| at >= since && at <= since.saturating_add(25_000))
                {
                    found.push((id, m["created_at_ms"].as_i64().unwrap()));
                }
            }
            if found.len() == 1 {
                self.resume(root, &found[0].0, found[0].1)?;
            } else if found.len() > 1 {
                return Err(store_io::invalid(
                    "Could not identify the resumed recording",
                ));
            }
        }
        self.refresh(hex)?;
        self.pending_start_ms = None;
        if self.phase == Phase::Recording {
            self.phase = Phase::Paused;
        }
        self.save(root)?;
        self.write_view(root)?;
        Ok(self)
    }
}
/// Physical parts never enter automatic notes, title, tasks, routing or Memory work.
pub fn held(root: &Path, id: &str) -> io::Result<bool> {
    Ok(owner(root, id)?.is_some())
}

#[path = "meeting_parts_input.rs"]
mod input;

#[cfg(test)]
#[path = "meeting_parts_tests.rs"]
mod tests;
