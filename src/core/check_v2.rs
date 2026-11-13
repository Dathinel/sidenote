//! Report identity and comparison. Titles and explanations never establish identity.
use super::{
    daily_check::{CheckContent, DailyCheck, Finding, KbSnapshot},
    store_io::fingerprint,
};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingChange {
    #[default]
    New,
    Changed,
    StillOpen,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Coverage {
    pub meeting_count: usize,
    pub task_count: usize,
    pub kb_revision: String,
    pub kb_captured_at: String,
    pub warnings: Vec<String>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlignmentStatus {
    Aligned,
    Tension,
    Unknown,
    NeedsDecision,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    pub text: String,
    pub sources: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryAlignment {
    pub project: String,
    pub status: AlignmentStatus,
    pub kb_claim: Claim,
    pub meeting_claim: Claim,
    pub question: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Comparison {
    pub baseline_date: Option<NaiveDate>,
    pub complete: bool,
    pub new: Vec<String>,
    pub changed: Vec<String>,
    pub carried: Vec<String>,
    pub resolved: Vec<String>,
    pub unmatched: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resolution {
    pub finding_id: String,
    pub explanation: String,
    pub evidence: Vec<String>,
}

pub fn evidence_fingerprint(finding: &Finding) -> String {
    fingerprint(&finding.evidence_keys)
}
fn same_subject(a: &Finding, b: &Finding) -> bool {
    !a.subject_refs.is_empty()
        && a.subject_refs == b.subject_refs
        && a.subject_keys == b.subject_keys
        && std::mem::discriminant(&a.kind) == std::mem::discriminant(&b.kind)
        && a.evidence.iter().any(|source| b.evidence.contains(source))
}
fn material_signature(f: &Finding) -> String {
    fingerprint([
        evidence_fingerprint(f),
        serde_json::to_string(&f.severity).unwrap(),
        serde_json::to_string(&f.action).unwrap(),
        f.material_state.clone(),
    ])
}
#[derive(Debug)]
pub enum PreparationError {
    InvalidContent,
    Io(io::Error),
}
impl From<io::Error> for PreparationError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
impl From<serde_json::Error> for PreparationError {
    fn from(error: serde_json::Error) -> Self {
        Self::Io(error.into())
    }
}

/// Called after source/subject validation. The engine assigns IDs, dates and comparisons.
pub fn prepare_content(
    content: &mut CheckContent,
    today: NaiveDate,
    history: &[DailyCheck],
    kb: &KbSnapshot,
    complete: bool,
) -> Result<(), PreparationError> {
    let previous = history.first();
    let mut known = BTreeMap::new();
    for report in history {
        for finding in &report.content.findings {
            if !finding.id.is_empty() {
                known.entry(&finding.id).or_insert(finding);
            }
        }
    }
    let mut comparison = Comparison {
        baseline_date: previous.map(|p| p.date),
        complete: complete && previous.is_none_or(|p| p.warnings.is_empty() && !p.kb.stale),
        ..Comparison::default()
    };
    let old = previous
        .map(|p| p.content.findings.as_slice())
        .unwrap_or_default();
    let mut matched = BTreeSet::new();
    let mut ids = BTreeSet::new();
    // Only one-to-one matches inherit identity. Splits/merges stay unmatched.
    let current = content.findings.clone();
    for finding in &mut content.findings {
        finding.subject_refs.sort();
        finding.subject_refs.dedup();
        let mut evidence_keys = Vec::new();
        for source in &finding.evidence {
            let key = if source.starts_with("[[") {
                source.clone()
            } else {
                format!(
                    "{source}@{}",
                    fingerprint([
                        String::from_utf8_lossy(&fs::read(kb.path.join(source))?).as_ref()
                    ])
                )
            };
            evidence_keys.push(key);
        }
        evidence_keys.sort();
        evidence_keys.dedup();
        finding.evidence_keys = evidence_keys;
        let candidates: Vec<_> = old
            .iter()
            .filter(|o| !o.id.is_empty() && same_subject(finding, o))
            .collect();
        let unique = candidates.len() == 1
            && current
                .iter()
                .filter(|f| {
                    let mut f = (*f).clone();
                    f.subject_refs.sort();
                    f.subject_refs.dedup();
                    same_subject(&f, candidates[0])
                })
                .count()
                == 1;
        if unique {
            let prior = candidates[0];
            finding.id = prior.id.clone();
            finding.first_seen = prior.first_seen.or(Some(previous.unwrap().date));
            matched.insert(prior.id.clone());
            if material_signature(finding) == material_signature(prior) {
                finding.change = FindingChange::StillOpen;
                comparison.carried.push(finding.id.clone());
            } else {
                finding.change = FindingChange::Changed;
                comparison.changed.push(finding.id.clone());
            }
        } else {
            finding.id = format!(
                "finding-{}",
                fingerprint([
                    serde_json::to_string(&finding.kind)?,
                    serde_json::to_string(&finding.subject_refs)?,
                    serde_json::to_string(&finding.subject_keys)?,
                    evidence_fingerprint(finding)
                ])
            );
            // A split must not accidentally regenerate an old identity without a unique match.
            if old.iter().any(|o| o.id == finding.id) {
                finding.id = format!(
                    "{}-{}",
                    finding.id,
                    fingerprint([today.to_string(), finding.material_state.clone()])
                );
            }
            finding.first_seen = Some(today);
            let historical: Vec<_> = known
                .values()
                .filter(|prior| same_subject(finding, prior))
                .collect();
            if historical.len() == 1
                && current
                    .iter()
                    .filter(|f| {
                        let mut f = (*f).clone();
                        f.subject_refs.sort();
                        f.subject_refs.dedup();
                        same_subject(&f, historical[0])
                    })
                    .count()
                    == 1
            {
                finding.id = historical[0].id.clone();
                finding.first_seen = historical[0].first_seen.or(Some(today));
            }
            finding.change = FindingChange::New;
            comparison.new.push(finding.id.clone());
        }
        finding.last_seen = Some(today);
        if !ids.insert(finding.id.clone()) {
            return Err(PreparationError::InvalidContent);
        }
    }
    for (index, prior) in old.iter().enumerate() {
        let key = if prior.id.is_empty() {
            format!("legacy:{}:{}", previous.unwrap().date, index)
        } else {
            prior.id.clone()
        };
        if matched.contains(&key) {
            continue;
        }
        // Absence alone never resolves an issue. New positive sources must be supplied explicitly.
        let resolved = comparison.complete
            && !prior.id.is_empty()
            && content.resolutions.iter().any(|r| {
                r.finding_id == prior.id
                    && !r.explanation.trim().is_empty()
                    && !r.evidence.is_empty()
                    && r.evidence.iter().any(|e| !prior.evidence.contains(e))
            });
        if resolved {
            comparison.resolved.push(key);
        } else {
            comparison.unmatched.push(key);
        }
    }
    content.comparison = comparison;
    Ok(())
}
