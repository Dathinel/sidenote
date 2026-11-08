//! Durable v2 triage in checks/triage.json, with date-scoped v1 marks in checks/state.json.
use super::{
    check_v2::evidence_fingerprint,
    daily_check::{DailyCheck, Finding},
    state_dir, store_io,
};
use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, path::PathBuf, sync::Mutex};

static WRITES: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Applied,
    Dismissed,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mark {
    pub status: Status,
    /// RFC 3339, local time.
    pub at: String,
}
/// Marks of one report, by finding key.
pub type Marks = BTreeMap<String, Mark>;
/// Report date (`YYYY-MM-DD`) to its marks.
type File = BTreeMap<String, Marks>;

/// A finding has no id of its own, so its place in the report and its title stand in. A rerun
/// that changes either starts that finding unmarked.
pub fn finding_key(index: usize, title: &str) -> String {
    format!("{index}|{title}")
}

fn read() -> io::Result<File> {
    super::sync_state::load(&state_dir(), "checks/state.json")
}

pub fn load(date: NaiveDate) -> io::Result<Marks> {
    let report = super::load_check(date)?;
    if report.schema_version == 1 {
        return Ok(read()?.remove(&date.to_string()).unwrap_or_default());
    }
    TriageStore::default().marks_for_report(&report, Local::now().date_naive())
}

/// Sets or clears one mark. The file is replaced atomically through a flushed temporary file.
pub fn set(date: NaiveDate, key: &str, mark: Option<Mark>) -> io::Result<Marks> {
    let report = super::load_check(date)?;
    if report.schema_version == 2 {
        let finding = report
            .content
            .findings
            .iter()
            .enumerate()
            .find(|(i, f)| finding_key(*i, &f.title) == key)
            .map(|(_, f)| f)
            .ok_or_else(|| store_io::invalid("Unknown finding key"))?;
        TriageStore::default().set_finding(
            finding,
            match mark.map(|m| m.status) {
                Some(Status::Applied) => TriageStatus::Applied,
                Some(Status::Dismissed) => TriageStatus::Dismissed,
                None => TriageStatus::Open,
            },
            None,
            None,
        )?;
        return load(date);
    }
    let _guard = WRITES.lock().unwrap_or_else(|e| e.into_inner());
    let mut file: File =
        super::sync_state::update(&state_dir(), "checks/state.json", |file: &mut File| {
            let day = file.entry(date.to_string()).or_default();
            match mark {
                Some(mark) => {
                    day.insert(key.to_owned(), mark);
                }
                None => {
                    day.remove(key);
                }
            }
            if day.is_empty() {
                file.remove(&date.to_string());
            }
        })?;
    Ok(file.remove(&date.to_string()).unwrap_or_default())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriageStatus {
    Open,
    Dismissed,
    Snoozed,
    Applied,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TriageEntry {
    pub finding_id: String,
    pub status: TriageStatus,
    pub updated_at: String,
    pub snooze_until: Option<NaiveDate>,
    pub evidence_fingerprint: String,
    pub evidence_keys: Vec<String>,
    pub note: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TriageFile {
    pub schema_version: u32,
    pub findings: BTreeMap<String, TriageEntry>,
}
impl Default for TriageFile {
    fn default() -> Self {
        Self {
            schema_version: 1,
            findings: BTreeMap::new(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct TriageStore {
    pub root: PathBuf,
}
impl Default for TriageStore {
    fn default() -> Self {
        Self { root: state_dir() }
    }
}
impl TriageStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
    pub fn load(&self) -> io::Result<TriageFile> {
        let file: TriageFile = super::sync_state::load(&self.root, "checks/triage.json")?;
        if file.schema_version != 1 {
            return Err(store_io::invalid("Unsupported triage schema"));
        }
        Ok(file)
    }
    /// Raw entries include handled and expired items for filter/history views.
    pub fn list(&self) -> io::Result<Vec<TriageEntry>> {
        Ok(self.load()?.findings.into_values().collect())
    }
    pub fn set_finding(
        &self,
        finding: &Finding,
        status: TriageStatus,
        snooze_until: Option<NaiveDate>,
        note: Option<String>,
    ) -> io::Result<TriageEntry> {
        if finding.id.is_empty() || finding.evidence_keys.is_empty() {
            return Err(store_io::invalid(
                "Durable triage requires a prepared v2 finding",
            ));
        }
        if status == TriageStatus::Snoozed
            && snooze_until.is_none_or(|d| d <= Local::now().date_naive())
        {
            return Err(store_io::invalid("Snooze needs a future date"));
        }
        let entry = TriageEntry {
            finding_id: finding.id.clone(),
            status,
            updated_at: Local::now().to_rfc3339(),
            snooze_until: if status == TriageStatus::Snoozed {
                snooze_until
            } else {
                None
            },
            evidence_fingerprint: evidence_fingerprint(finding),
            evidence_keys: finding.evidence_keys.clone(),
            note,
        };
        super::sync_state::update(&self.root, "checks/triage.json", |file: &mut TriageFile| {
            file.findings.insert(finding.id.clone(), entry.clone());
        })?;
        Ok(entry)
    }
    pub fn status_for(&self, finding: &Finding, today: NaiveDate) -> io::Result<TriageStatus> {
        Ok(effective_status(
            self.load()?.findings.get(&finding.id),
            finding,
            today,
        ))
    }
    /// Compatibility adapter for the current page. New screens can use status_for directly.
    pub fn marks_for_report(&self, report: &DailyCheck, today: NaiveDate) -> io::Result<Marks> {
        let file = self.load()?;
        let mut marks = Marks::new();
        for (index, finding) in report.content.findings.iter().enumerate() {
            let entry = file.findings.get(&finding.id);
            let status = match effective_status(entry, finding, today) {
                TriageStatus::Open => continue,
                TriageStatus::Applied => Status::Applied,
                TriageStatus::Dismissed | TriageStatus::Snoozed => Status::Dismissed,
            };
            marks.insert(
                finding_key(index, &finding.title),
                Mark {
                    status,
                    at: entry.unwrap().updated_at.clone(),
                },
            );
        }
        Ok(marks)
    }
}
fn effective_status(
    entry: Option<&TriageEntry>,
    finding: &Finding,
    today: NaiveDate,
) -> TriageStatus {
    let Some(entry) = entry else {
        return TriageStatus::Open;
    };
    if finding
        .evidence_keys
        .iter()
        .any(|key| !entry.evidence_keys.contains(key))
        || (entry.status == TriageStatus::Snoozed && entry.snooze_until.is_none_or(|d| d <= today))
    {
        TriageStatus::Open
    } else {
        entry.status
    }
}
