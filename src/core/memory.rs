//! Owner-controlled company claims. records.json is authoritative; Markdown is a projection.
#[cfg(test)]
mod undo_tests;
use super::{
    state_dir,
    store_io::{self, fingerprint, invalid},
};
use chrono::Local;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fs, io, path::PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordType {
    Person,
    Project,
    Decision,
    Commitment,
    Question,
    #[default]
    Story,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordStatus {
    Proposed,
    Active,
    Superseded,
    Retired,
    Forgotten,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    OwnerConfirmed,
    SourceSupported,
    Inferred,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum MemorySource {
    Meeting {
        reference: String,
    },
    Kb {
        path: String,
        revision: Option<String>,
    },
}
impl MemorySource {
    pub fn reference(&self) -> &str {
        match self {
            Self::Meeting { reference } => reference,
            Self::Kb { path, .. } => path,
        }
    }
}
pub fn source_fingerprint(sources: &[MemorySource]) -> String {
    let canonical: BTreeSet<_> = sources
        .iter()
        .map(|s| serde_json::to_string(s).unwrap())
        .collect();
    fingerprint(canonical)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignment: Option<super::areas::Target>,
    pub id: String,
    #[serde(rename = "type")]
    pub record_type: RecordType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub project: Option<String>,
    #[serde(default)]
    pub people: Vec<String>,
    pub sources: Vec<MemorySource>,
    pub first_seen: Option<String>,
    pub last_confirmed: Option<String>,
    pub status: RecordStatus,
    pub basis: Basis,
    pub supersedes: Option<String>,
    pub updated_at: String,
    pub source_fingerprint: String,
    #[serde(default)]
    pub source_fingerprints: Vec<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalOp {
    Add,
    Confirm,
    Supersede,
    Retire,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryProposal {
    #[serde(default)]
    pub id: String,
    pub op: ProposalOp,
    pub target_id: Option<String>,
    pub text: String,
    #[serde(default)]
    pub record_type: RecordType,
    pub project: Option<String>,
    #[serde(default)]
    pub people: Vec<String>,
    pub sources: Vec<MemorySource>,
    pub rationale: String,
    /// Optimistic guard captured when the report is prepared, never trusted from the model.
    #[serde(default)]
    pub target_updated_at: Option<String>,
}
impl MemoryProposal {
    pub fn matches_target_version(&self, record: &MemoryRecord) -> bool {
        self.target_updated_at
            .as_deref()
            .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
            .zip(chrono::DateTime::parse_from_rfc3339(&record.updated_at).ok())
            .is_some_and(|(expected, current)| expected == current)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Pending,
    Accepted,
    Rejected,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingProposal {
    #[serde(flatten)]
    pub proposal: MemoryProposal,
    pub status: ProposalStatus,
    pub created_at: String,
    pub decided_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryUndo {
    pub before: Vec<MemoryRecord>,
    pub after: Vec<MemoryRecord>,
    pub proposal_before: Option<PendingProposal>,
    pub proposal_after: Option<PendingProposal>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryEvent {
    pub record_id: String,
    pub operation: String,
    pub at: String,
    pub proposal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo: Option<MemoryUndo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undone_at: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MigrationInfo {
    pub version: u32,
    pub source_path: String,
    pub fingerprint: String,
    pub accepted_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryFile {
    pub schema_version: u32,
    pub records: Vec<MemoryRecord>,
    pub proposals: Vec<PendingProposal>,
    pub events: Vec<MemoryEvent>,
    pub migration: Option<MigrationInfo>,
}
impl Default for MemoryFile {
    fn default() -> Self {
        Self {
            schema_version: 1,
            records: vec![],
            proposals: vec![],
            events: vec![],
            migration: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MigrationCandidate {
    pub record: MemoryRecord,
    pub line: usize,
    pub uncited: bool,
    pub ambiguous: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MigrationPreview {
    pub source_fingerprint: String,
    pub imported: usize,
    pub uncited: usize,
    pub ambiguous: usize,
    pub candidates: Vec<MigrationCandidate>,
    pub prepared: bool,
    pub runner: Option<String>,
    pub runtime_ms: u64,
    pub unplaced: Vec<MigrationIssue>,
    pub fallbacks: Vec<MigrationIssue>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MigrationIssue {
    pub line: usize,
    pub text: String,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct MemoryStore {
    pub root: PathBuf,
}
impl Default for MemoryStore {
    fn default() -> Self {
        Self::new(state_dir())
    }
}
impl MemoryStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.root.join("local/memory").join(name)
    }
    pub fn load(&self) -> io::Result<MemoryFile> {
        if super::paths::read_only() {
            return self.resolve_projects(self.load_locked()?);
        }
        let _lock = store_io::lock(&self.path("records.lock"))?;
        self.resolve_projects(self.load_locked()?)
    }
    fn resolve_projects(&self, mut file: MemoryFile) -> io::Result<MemoryFile> {
        let projects = super::TaskStore::new(self.root.clone()).load_projects()?;
        let catalog = projects.area_catalog()?;
        let project = |kind: &str,
                       id: &str,
                       raw: &Option<String>,
                       retained: Option<&super::areas::Target>| {
            let baseline = super::areas::assignment_baseline(serde_json::json!(raw), retained);
            match catalog.resolve_target(
                &projects,
                &serde_json::json!([kind, id]).to_string(),
                &baseline,
                retained.map(super::areas::Assignment::Target),
                raw.as_deref().unwrap_or(super::UNSORTED),
            ) {
                Some(target) => target
                    .project_id
                    .and_then(|id| catalog.projects.get(&id))
                    .map(|p| p.name.clone()),
                None => raw.as_deref().map(|p| projects.resolve(p)),
            }
        };
        for record in &mut file.records {
            record.project = project(
                "memory",
                &record.id,
                &record.project,
                record.assignment.as_ref(),
            );
        }
        for pending in &mut file.proposals {
            pending.proposal.project = project(
                "proposal",
                &pending.proposal.id,
                &pending.proposal.project,
                None,
            );
        }
        Ok(file)
    }
    fn load_locked(&self) -> io::Result<MemoryFile> {
        let file: MemoryFile = super::sync_state::load(&self.root, "memory/records.json")?;
        if let Some(preview) = super::relay_preview::get(&self.root)?
            && preview.conflicts()?.iter().any(|c| {
                c.task_id == "memory/records.json"
                    && serde_json::from_str::<Vec<String>>(&c.field).is_ok_and(|p| {
                        p.first().is_some_and(|v| v == "o:proposals")
                            && p.last().is_some_and(|v| v == "o:status")
                    })
            })
        {
            return Err(invalid(
                "Resolve Memory decision versions in Settings > Sync before using these records",
            ));
        }
        if file.schema_version != 1 {
            return Err(invalid("Unsupported memory schema version"));
        }
        Ok(file)
    }
    pub fn list_records(&self) -> io::Result<Vec<MemoryRecord>> {
        Ok(self.load()?.records)
    }
    fn save(&self, before: &MemoryFile, file: &MemoryFile) -> io::Result<()> {
        super::sync_state::save_changes(&self.root, "memory/records.json", before, file)?;
        if file.migration.is_some() {
            let text = render(&self.resolve_projects(file.clone())?);
            self.export(&text)?;
        }
        Ok(())
    }
    fn export(&self, text: &str) -> io::Result<()> {
        match fs::read(self.path("company.md")) {
            Ok(bytes) if bytes == text.as_bytes() => return Ok(()),
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        store_io::atomic_write(&self.path("company.md"), text.as_bytes())
    }
    /// Repairs an interrupted export. Legacy memory remains authoritative until acceptance.
    pub fn company_memory(&self) -> io::Result<String> {
        self.snapshot().map(|(_, text)| text)
    }
    /// One consistent prompt snapshot, protected against simultaneous owner edits.
    pub fn snapshot(&self) -> io::Result<(MemoryFile, String)> {
        if super::paths::read_only() {
            let file = self.resolve_projects(self.load_locked()?)?;
            let text = if file.migration.is_some() {
                render(&file)
            } else {
                self.legacy_text()?
            };
            return Ok((file, text));
        }
        let _lock = store_io::lock(&self.path("records.lock"))?;
        let file = self.resolve_projects(self.load_locked()?)?;
        if file.migration.is_some() {
            let text = render(&file);
            self.export(&text)?;
            Ok((file, text))
        } else {
            self.legacy_text().map(|text| (file, text))
        }
    }
    pub(super) fn cache_root(&self) -> PathBuf {
        self.root.join("local")
    }
    pub(super) fn legacy_text(&self) -> io::Result<String> {
        let value: serde_json::Value = super::sync_state::load(&self.root, "memory/legacy.json")?;
        Ok(value
            .get("documents")
            .and_then(serde_json::Value::as_object)
            .map(|docs| {
                docs.values()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n\n")
            })
            .unwrap_or_default())
    }
    /// Validates cached model output against the current legacy source.
    pub fn preview_migration(&self) -> io::Result<MigrationPreview> {
        let _lock = store_io::lock(&self.path("records.lock"))?;
        self.preview_migration_locked()
    }
    pub(super) fn preview_migration_locked(&self) -> io::Result<MigrationPreview> {
        if self.load_locked()?.migration.is_some() {
            return Err(invalid("Memory was already migrated"));
        }
        let text = self.legacy_text()?;
        super::memory_migration::cached_preview(&self.cache_root(), &text)
    }
    /// Potentially slow model call. Run on a worker, never on the render thread.
    pub fn prepare_migration(&self, force: bool) -> io::Result<MigrationPreview> {
        super::memory_migration::prepare(self, force)
    }
    pub fn accept_migration(&self, expected_fingerprint: &str) -> io::Result<MigrationPreview> {
        let _lock = store_io::lock(&self.path("records.lock"))?;
        let preview = self.preview_migration_locked()?;
        if preview.source_fingerprint != expected_fingerprint {
            return Err(invalid("Legacy memory changed; preview it again"));
        }
        let original = self.legacy_text()?.into_bytes();
        if fingerprint([
            std::str::from_utf8(&original).map_err(|_| invalid("Legacy memory is not UTF-8"))?
        ]) != expected_fingerprint
        {
            return Err(invalid(
                "Legacy memory changed during acceptance; preview it again",
            ));
        }
        let backup = self.path("company.legacy.md");
        if backup.try_exists()? {
            if fs::read(&backup)? != original {
                return Err(invalid("Legacy backup differs; refusing to overwrite it"));
            }
        } else {
            store_io::atomic_write(&backup, &original)?;
        }
        let mut file = self.load_locked()?;
        let before = file.clone();
        file.records.extend(preview.candidates.iter().map(|c| {
            let mut record = c.record.clone();
            record.status = RecordStatus::Active;
            record.updated_at = Local::now().to_rfc3339();
            record
        }));
        file.migration = Some(MigrationInfo {
            version: 1,
            source_path: "local/memory/company.legacy.md".into(),
            fingerprint: preview.source_fingerprint.clone(),
            accepted_at: Local::now().to_rfc3339(),
        });
        event(&mut file, "migration", "accept_migration", None);
        self.save(&before, &file)?;
        Ok(preview)
    }
    /// Report ingestion is idempotent. No record is modified here.
    pub fn stage_proposals(&self, proposals: &[MemoryProposal]) -> io::Result<()> {
        self.stage_review(proposals, true)
    }
    /// An agent may submit an uncited addition for explicit owner review, never acceptance.
    pub fn stage_agent_proposal(&self, proposal: &MemoryProposal) -> io::Result<()> {
        if proposal.op != ProposalOp::Add || proposal.target_id.is_some() {
            return Err(invalid("Agents may only propose additions"));
        }
        self.stage_review(std::slice::from_ref(proposal), false)
    }
    fn stage_review(&self, proposals: &[MemoryProposal], require_sources: bool) -> io::Result<()> {
        let _lock = store_io::lock(&self.path("records.lock"))?;
        let mut file = self.load_locked()?;
        let before = file.clone();
        merge_proposals(
            &mut file,
            proposals,
            require_sources,
            &Local::now().to_rfc3339(),
        )?;
        self.save(&before, &file)
    }
    /// Reconcile reports after a crash between report commit and pending-queue commit.
    pub fn list_pending_proposals(&self) -> io::Result<Vec<PendingProposal>> {
        Ok(self
            .list_proposals()?
            .into_iter()
            .filter(|p| p.status == ProposalStatus::Pending)
            .collect())
    }
    /// Includes report proposals not yet queued by a window refresh.
    pub fn list_proposals(&self) -> io::Result<Vec<PendingProposal>> {
        let mut file = self.load()?;
        for store in super::sync_state::stores(&self.root)? {
            let Some(date) = store
                .strip_prefix("checks/")
                .and_then(|s| s.strip_suffix(".json"))
                .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
            else {
                continue;
            };
            let value: serde_json::Value =
                super::sync_state::load(&self.root, &format!("checks/{date}.json"))?;
            if let Some(proposals) = value.get("memory_proposals") {
                let proposals = serde_json::from_value::<Vec<MemoryProposal>>(proposals.clone())?;
                if super::paths::read_only() {
                    merge_proposals(&mut file, &proposals, true, "")?;
                } else {
                    self.stage_proposals(&proposals)?;
                }
            }
        }
        if super::paths::read_only() {
            Ok(self.resolve_projects(file)?.proposals)
        } else {
            Ok(self.load()?.proposals)
        }
    }
    pub fn reject_proposal(&self, id: &str) -> io::Result<()> {
        self.decide_proposal(id, false)
    }
    pub fn accept_proposal(&self, id: &str) -> io::Result<()> {
        self.decide_proposal(id, true)
    }
    fn decide_proposal(&self, id: &str, accept: bool) -> io::Result<()> {
        let _lock = store_io::lock(&self.path("records.lock"))?;
        let mut file = self.load_locked()?;
        let before = file.clone();
        if accept && file.migration.is_none() {
            return Err(invalid("Accept the memory migration first"));
        }
        let index = file
            .proposals
            .iter()
            .position(|p| p.proposal.id == id)
            .ok_or_else(|| invalid("Unknown proposal"))?;
        if file.proposals[index].status != ProposalStatus::Pending {
            return Err(invalid("Proposal was already decided"));
        }
        let proposal = file.proposals[index].proposal.clone();
        let projects = super::TaskStore::new(self.root.clone()).load_projects()?;
        let catalog = projects.area_catalog()?;
        let assignment = catalog.resolve_target(
            &projects,
            &serde_json::json!(["proposal", proposal.id]).to_string(),
            &serde_json::json!(proposal.project),
            None,
            proposal.project.as_deref().unwrap_or(super::UNSORTED),
        );
        if accept {
            // The owner is confirming the claim here; automated report ingestion still requires evidence.
            validate_reviewable_proposal(&proposal)?;
            if blocked(&file, &proposal.sources) {
                return Err(invalid("This source set was forgotten"));
            }
            if let Some(target) = &proposal.target_id {
                let record = active_record(&mut file, target)?;
                if !proposal.matches_target_version(record) {
                    return Err(invalid(
                        "Record changed since this proposal; review a new proposal",
                    ));
                }
                match proposal.op {
                    ProposalOp::Confirm => {
                        if !proposal.sources.iter().any(|s| !record.sources.contains(s)) {
                            return Err(invalid(
                                "Confirmation needs a new source or explicit owner confirmation",
                            ));
                        }
                        for source in &proposal.sources {
                            if !record.sources.contains(source) {
                                record.sources.push(source.clone());
                            }
                        }
                        record.last_confirmed = Some(Local::now().to_rfc3339());
                        record.basis = Basis::OwnerConfirmed;
                    }
                    ProposalOp::Supersede => record.status = RecordStatus::Superseded,
                    ProposalOp::Retire => record.status = RecordStatus::Retired,
                    ProposalOp::Add => unreachable!(),
                }
                record.updated_at = Local::now().to_rfc3339();
                record.source_fingerprint = source_fingerprint(&record.sources);
                record.source_fingerprints = record
                    .sources
                    .iter()
                    .map(|s| source_fingerprint(std::slice::from_ref(s)))
                    .collect();
            }
            let record_id = if matches!(proposal.op, ProposalOp::Add | ProposalOp::Supersede) {
                let record_id = format!("memory-{}", proposal.id);
                file.records.push(MemoryRecord {
                    assignment: assignment.clone(),
                    id: record_id.clone(),
                    record_type: proposal.record_type,
                    text: Some(proposal.text.clone()),
                    project: match &assignment {
                        Some(target) => target
                            .project_id
                            .as_ref()
                            .and_then(|id| catalog.projects.get(id))
                            .map(|p| p.name.clone()),
                        None => proposal.project.clone(),
                    },
                    people: proposal.people.clone(),
                    sources: proposal.sources.clone(),
                    first_seen: Some(Local::now().to_rfc3339()),
                    last_confirmed: Some(Local::now().to_rfc3339()),
                    status: RecordStatus::Active,
                    basis: Basis::OwnerConfirmed,
                    supersedes: proposal.target_id.clone(),
                    updated_at: Local::now().to_rfc3339(),
                    source_fingerprint: source_fingerprint(&proposal.sources),
                    source_fingerprints: proposal
                        .sources
                        .iter()
                        .map(|s| source_fingerprint(std::slice::from_ref(s)))
                        .collect(),
                });
                record_id
            } else {
                proposal.target_id.clone().unwrap()
            };
            event(&mut file, &record_id, "accept_proposal", Some(id));
        } else {
            event(
                &mut file,
                proposal.target_id.as_deref().unwrap_or("proposal"),
                "reject_proposal",
                Some(id),
            );
        }
        file.proposals[index].status = if accept {
            ProposalStatus::Accepted
        } else {
            ProposalStatus::Rejected
        };
        file.proposals[index].decided_at = Some(Local::now().to_rfc3339());
        capture_undo(&before, &mut file);
        self.save(&before, &file)
    }
    pub fn comment_proposal(&self, id: &str, comment: &str) -> io::Result<()> {
        if comment.trim().is_empty() || comment.chars().count() > 4000 {
            return Err(invalid("Write a comment of up to 4000 characters"));
        }
        let _lock = store_io::lock(&self.path("records.lock"))?;
        let mut file = self.load_locked()?;
        let before = file.clone();
        let proposal = file
            .proposals
            .iter_mut()
            .find(|p| p.proposal.id == id && p.status == ProposalStatus::Pending)
            .ok_or_else(|| invalid("This proposal is no longer waiting"))?;
        proposal.comment = Some(comment.trim().into());
        self.save(&before, &file)
    }
    pub fn undo_event(&self, record_id: &str, at: &str) -> io::Result<()> {
        let _lock = store_io::lock(&self.path("records.lock"))?;
        let mut file = self.load_locked()?;
        let before = file.clone();
        let index = file
            .events
            .iter()
            .position(|e| e.record_id == record_id && e.at == at)
            .ok_or_else(|| invalid("This change is no longer available"))?;
        if file.events[index].undone_at.is_some() {
            return Err(invalid("This change was already undone"));
        }
        let undo = file.events[index]
            .undo
            .clone()
            .ok_or_else(|| invalid("This change has no saved previous state"))?;
        if undo.before.iter().any(|r| blocked(&file, &r.sources))
            || undo
                .proposal_before
                .as_ref()
                .is_some_and(|p| blocked(&file, &p.proposal.sources))
        {
            return Err(invalid("Undo cannot restore a forgotten source"));
        }
        for expected in &undo.after {
            let current = file.records.iter().find(|r| r.id == expected.id);
            if serde_json::to_value(current)? != serde_json::to_value(Some(expected))? {
                return Err(invalid(
                    "Memory changed after this decision. Undo was not applied",
                ));
            }
        }
        if let Some(expected) = &undo.proposal_after {
            let current = file
                .proposals
                .iter()
                .find(|p| p.proposal.id == expected.proposal.id);
            if serde_json::to_value(current)? != serde_json::to_value(Some(expected))? {
                return Err(invalid(
                    "The proposal changed after this decision. Undo was not applied",
                ));
            }
        }
        file.records
            .retain(|r| !undo.after.iter().any(|a| a.id == r.id));
        file.records.extend(undo.before);
        if let Some(proposal) = undo.proposal_before {
            file.proposals
                .retain(|p| p.proposal.id != proposal.proposal.id);
            file.proposals.push(proposal);
        }
        file.events[index].undone_at = Some(Local::now().to_rfc3339());
        event(&mut file, record_id, "undo_decision", None);
        self.save(&before, &file)
    }
    pub fn edit_record(&self, id: &str, text: &str) -> io::Result<()> {
        if text.trim().is_empty() {
            return Err(invalid("Record text must not be empty"));
        }
        self.owner_change(id, "edit", |r| {
            r.text = Some(text.trim().into());
            r.basis = Basis::OwnerConfirmed;
        })
    }
    pub fn merge_edit(
        &self,
        base: &MemoryRecord,
        text: &str,
    ) -> io::Result<super::EditView<MemoryRecord>> {
        let _lock = store_io::lock(&self.path("records.lock"))?;
        if text.trim().is_empty() {
            return Err(invalid("Record text must not be empty"));
        }
        let mut mine = base.clone();
        mine.text = Some(text.trim().into());
        let file = super::sync_state::update_merged(
            &self.root,
            "memory/records.json",
            |file: &mut MemoryFile| {
                let before = file.clone();
                let current = active_record(file, &base.id)?;
                *current = super::live_edit::merge(base, &mine, current)?;
                current.basis = Basis::OwnerConfirmed;
                current.updated_at = Local::now().to_rfc3339();
                event(file, &base.id, "edit", None);
                capture_undo(&before, file);
                Ok(())
            },
        )?;
        if file.value.migration.is_some() {
            let text = render(&self.resolve_projects(file.value.clone())?);
            self.export(&text)?;
        }
        Ok(super::EditView {
            value: file
                .value
                .records
                .into_iter()
                .find(|r| r.id == base.id)
                .unwrap(),
            generation: file.generation,
        })
    }
    pub fn confirm_record(&self, id: &str) -> io::Result<()> {
        self.owner_change(id, "confirm", |r| {
            r.basis = Basis::OwnerConfirmed;
            r.last_confirmed = Some(Local::now().to_rfc3339());
        })
    }
    pub fn retire_record(&self, id: &str) -> io::Result<()> {
        self.owner_change(id, "retire", |r| r.status = RecordStatus::Retired)
    }
    pub fn forget_record(&self, id: &str) -> io::Result<()> {
        let _lock = store_io::lock(&self.path("records.lock"))?;
        let mut file = self.load_locked()?;
        let before = file.clone();
        let record = file
            .records
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or_else(|| invalid("Unknown record"))?;
        let forgotten = record.source_fingerprint.clone();
        let forgotten_sources = record.source_fingerprints.clone();
        record.text = None;
        record.project = None;
        record.assignment = None;
        record.people.clear();
        record.sources.clear();
        record.status = RecordStatus::Forgotten;
        record.updated_at = Local::now().to_rfc3339();
        // Forgetting also removes receipts containing the claim or its proposal text.
        for event in &mut file.events {
            if event.record_id == id
                || event.undo.as_ref().is_some_and(|u| {
                    u.before
                        .iter()
                        .chain(&u.after)
                        .any(|r| r.id == id || r.source_fingerprint == forgotten)
                        || u.proposal_before.iter().chain(&u.proposal_after).any(|p| {
                            p.proposal.target_id.as_deref() == Some(id)
                                || source_fingerprint(&p.proposal.sources) == forgotten
                        })
                })
            {
                event.undo = None;
            }
        }
        // Proposals can contain the same claim and rationale. Remove those copies in this store.
        file.proposals.retain(|p| {
            p.proposal.target_id.as_deref() != Some(id)
                && source_fingerprint(&p.proposal.sources) != forgotten
                && !p.proposal.sources.iter().all(|s| {
                    forgotten_sources.contains(&source_fingerprint(std::slice::from_ref(s)))
                })
        });
        event(&mut file, id, "forget", None);
        self.save(&before, &file)
    }
    fn owner_change(
        &self,
        id: &str,
        operation: &str,
        change: impl FnOnce(&mut MemoryRecord),
    ) -> io::Result<()> {
        let _lock = store_io::lock(&self.path("records.lock"))?;
        let mut file = self.load_locked()?;
        let before = file.clone();
        let record = active_record(&mut file, id)?;
        change(record);
        record.updated_at = Local::now().to_rfc3339();
        event(&mut file, id, operation, None);
        capture_undo(&before, &mut file);
        self.save(&before, &file)
    }
}
fn merge_proposals(
    file: &mut MemoryFile,
    proposals: &[MemoryProposal],
    require_sources: bool,
    created_at: &str,
) -> io::Result<()> {
    for proposal in proposals {
        if require_sources {
            validate_proposal(proposal)?;
        } else {
            validate_reviewable_proposal(proposal)?;
        }
        if proposal.id != proposal_id(proposal) {
            return Err(invalid("Proposal id does not match its content"));
        }
        if blocked(file, &proposal.sources)
            || file.records.iter().any(|r| {
                r.status == RecordStatus::Forgotten && proposal.target_id.as_deref() == Some(&r.id)
            })
        {
            continue;
        }
        if !file.proposals.iter().any(|p| p.proposal.id == proposal.id) {
            file.proposals.push(PendingProposal {
                proposal: proposal.clone(),
                status: ProposalStatus::Pending,
                created_at: created_at.into(),
                decided_at: None,
                comment: None,
            });
        }
    }
    Ok(())
}
fn active_record<'a>(file: &'a mut MemoryFile, id: &str) -> io::Result<&'a mut MemoryRecord> {
    file.records
        .iter_mut()
        .find(|r| r.id == id && r.status == RecordStatus::Active)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Target is not an active record"))
}
fn event(file: &mut MemoryFile, id: &str, operation: &str, proposal: Option<&str>) {
    file.events.push(MemoryEvent {
        record_id: id.into(),
        operation: operation.into(),
        at: Local::now().to_rfc3339(),
        proposal_id: proposal.map(str::to_owned),
        undo: None,
        undone_at: None,
    });
}
fn capture_undo(before: &MemoryFile, file: &mut MemoryFile) {
    let changed: Vec<_> = file
        .records
        .iter()
        .filter(|after| {
            let old = before.records.iter().find(|r| r.id == after.id);
            serde_json::to_value(old).unwrap() != serde_json::to_value(Some(*after)).unwrap()
        })
        .cloned()
        .collect();
    let old = before
        .records
        .iter()
        .filter(|r| changed.iter().any(|a| a.id == r.id))
        .cloned()
        .collect();
    if let Some(event) = file.events.last_mut() {
        let id = event.proposal_id.as_deref();
        event.undo = Some(MemoryUndo {
            before: old,
            after: changed,
            proposal_before: before
                .proposals
                .iter()
                .find(|p| Some(p.proposal.id.as_str()) == id)
                .cloned(),
            proposal_after: file
                .proposals
                .iter()
                .find(|p| Some(p.proposal.id.as_str()) == id)
                .cloned(),
        });
    }
}
fn blocked(file: &MemoryFile, sources: &[MemorySource]) -> bool {
    let fingerprint = source_fingerprint(sources);
    file.records.iter().any(|r| {
        r.status == RecordStatus::Forgotten
            && (r.source_fingerprint == fingerprint
                || (!sources.is_empty()
                    && sources.iter().all(|s| {
                        r.source_fingerprints
                            .contains(&source_fingerprint(std::slice::from_ref(s)))
                    })))
    })
}
pub(super) fn validate_proposal(p: &MemoryProposal) -> io::Result<()> {
    if p.sources.is_empty() {
        return Err(invalid("Automated report proposals require sources"));
    }
    validate_reviewable_proposal(p)
}
fn validate_reviewable_proposal(p: &MemoryProposal) -> io::Result<()> {
    if p.text.trim().is_empty()
        || p.rationale.trim().is_empty()
        || (p.op == ProposalOp::Add) != p.target_id.is_none()
    {
        return Err(invalid(
            "Proposal needs text, rationale and a target except for add",
        ));
    }
    let meeting = Regex::new(r"^\[\[[0-9]+-[0-9]+(?:@[0-9]+:[0-5][0-9])?\]\]$").unwrap();
    for source in &p.sources {
        let valid = match source {
            MemorySource::Meeting { reference } => meeting.is_match(reference),
            MemorySource::Kb { path, revision } => {
                super::daily_check::safe_kb_path(path)
                    && revision
                        .as_ref()
                        .is_some_and(|r| r.len() == 40 && r.bytes().all(|b| b.is_ascii_hexdigit()))
            }
        };
        if !valid {
            return Err(invalid(
                "Proposal source needs an exact meeting reference or KB path and captured revision",
            ));
        }
    }
    Ok(())
}
pub fn proposal_id(p: &MemoryProposal) -> String {
    fingerprint([
        serde_json::to_string(&p.op).unwrap(),
        p.target_id.clone().unwrap_or_default(),
        p.text.split_whitespace().collect::<Vec<_>>().join(" "),
        source_fingerprint(&p.sources),
        serde_json::to_string(&p.record_type).unwrap(),
        p.project.clone().unwrap_or_default(),
        serde_json::to_string(&p.people).unwrap(),
        p.target_updated_at.clone().unwrap_or_default(),
    ])
}
fn render(file: &MemoryFile) -> String {
    let mut text = String::from("# Company memory\n");
    for record in file
        .records
        .iter()
        .filter(|r| r.status == RecordStatus::Active)
    {
        text.push_str(&format!(
            "\n## {} ({})\n\n{}\n",
            record.project.as_deref().unwrap_or("Company"),
            record.id,
            record.text.as_deref().unwrap_or_default()
        ));
        if !record.sources.is_empty() {
            let sources: Vec<_> = record
                .sources
                .iter()
                .map(|s| match s {
                    MemorySource::Meeting { reference } => reference.clone(),
                    MemorySource::Kb { path, revision } => format!(
                        "`{path}` (revision: {})",
                        revision.as_deref().unwrap_or("unknown")
                    ),
                })
                .collect();
            text.push_str(&format!("\nSources: {}\n", sources.join(", ")));
        }
    }
    text
}

pub(super) fn migration_sources(paragraph: &str) -> Vec<MemorySource> {
    static CITATIONS: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let citations = CITATIONS.get_or_init(|| Regex::new(r"\[\[[^\]\n]+\]\]|`([^`\n]+)`").unwrap());
    let mut sources = Vec::new();
    for source in citations.captures_iter(paragraph).filter_map(|c| {
        if c[0].starts_with("[[") {
            Some(MemorySource::Meeting {
                reference: c[0].into(),
            })
        } else {
            let path = c[1].to_owned();
            (path.contains('.') && !path.contains(' ')).then_some(MemorySource::Kb {
                path,
                revision: None,
            })
        }
    }) {
        if !sources.contains(&source) {
            sources.push(source);
        }
    }
    sources
}

pub(super) fn migration_preview(original: &str) -> MigrationPreview {
    let text = original
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n");
    let mut preview = MigrationPreview {
        source_fingerprint: fingerprint([original]),
        imported: 0,
        uncited: 0,
        ambiguous: 0,
        candidates: vec![],
        prepared: false,
        runner: None,
        runtime_ms: 0,
        unplaced: vec![],
        fallbacks: vec![],
    };
    let mut project = None;
    let mut lines = Vec::new();
    let mut start = 1;
    let flush = |lines: &mut Vec<&str>,
                 start: usize,
                 project: &Option<String>,
                 preview: &mut MigrationPreview| {
        if lines.is_empty() {
            return;
        }
        let paragraph = lines.join("\n");
        lines.clear();
        let sources = migration_sources(&paragraph);
        let lower = paragraph.to_lowercase();
        let record_type = if lower.starts_with("decided") {
            RecordType::Decision
        } else if lower.starts_with("open question") {
            RecordType::Question
        } else {
            RecordType::Story
        };
        let uncited = sources.is_empty();
        // Whole paragraphs preserve context. Multiple sentences or uncertain language need owner review.
        let claim = lower
            .split_once(" source:")
            .or_else(|| lower.split_once(" sources:"))
            .map_or(lower.as_str(), |(claim, _)| claim);
        let ambiguous = claim.matches(". ").count() > 0
            || [
                "unknown",
                "unconfirmed",
                "unsettled",
                "conflict",
                "not confirmed",
                "not stated",
                "?",
            ]
            .iter()
            .any(|s| claim.contains(s));
        preview.candidates.push(MigrationCandidate {
            record: MemoryRecord {
                assignment: None,
                id: format!(
                    "legacy-{}",
                    fingerprint([start.to_string(), paragraph.clone()])
                ),
                record_type,
                text: Some(paragraph),
                project: project.clone(),
                people: vec![],
                source_fingerprint: source_fingerprint(&sources),
                source_fingerprints: sources
                    .iter()
                    .map(|s| source_fingerprint(std::slice::from_ref(s)))
                    .collect(),
                sources,
                first_seen: None,
                last_confirmed: None,
                status: RecordStatus::Proposed,
                basis: Basis::Unknown,
                supersedes: None,
                updated_at: String::new(),
            },
            line: start,
            uncited,
            ambiguous,
        });
    };
    let bullet_pattern = Regex::new(r"^(?:[-*+] |[0-9]+[.)] )").unwrap();
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        let bullet = bullet_pattern.is_match(line);
        if trimmed.is_empty() || trimmed.starts_with('#') || bullet {
            flush(&mut lines, start, &project, &mut preview);
        }
        if trimmed.starts_with('#') {
            if trimmed.starts_with("## ") {
                project = Some(trimmed.trim_start_matches('#').trim().into());
            }
        } else if !trimmed.is_empty() {
            if lines.is_empty() {
                start = index + 1;
            }
            lines.push(line);
        }
    }
    flush(&mut lines, start, &project, &mut preview);
    preview.imported = preview.candidates.len();
    preview.uncited = preview.candidates.iter().filter(|c| c.uncited).count();
    preview.ambiguous = preview.candidates.iter().filter(|c| c.ambiguous).count();
    preview
}
