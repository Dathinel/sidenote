//! Model output is an untrusted suggestion. Cache the response, then validate it on every read.
use super::{
    memory::{
        self, Basis, MemoryRecord, MemorySource, MemoryStore, MigrationCandidate, MigrationIssue,
        MigrationPreview, RecordStatus, RecordType,
    },
    run_runner, runners,
    store_io::{self, fingerprint, invalid},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fs, io, path::Path, time::Instant};

const CACHE_VERSION: u32 = 1;
const RUNNER: &str = "Memory migration (answer only)";

#[derive(Serialize)]
struct Input {
    id: usize,
    line: usize,
    heading: Option<String>,
    kind: &'static str,
    text: String,
    sources: Vec<String>,
    #[serde(skip)]
    fallback: Option<MigrationCandidate>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Answer {
    paragraphs: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Group {
    id: usize,
    records: Vec<Claim>,
    unplaced: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Claim {
    #[serde(rename = "type")]
    kind: RecordType,
    text: String,
    project: Option<String>,
    people: Vec<String>,
    sources: Vec<String>,
    evidence: String,
    first_seen: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Cache {
    version: u32,
    source_fingerprint: String,
    response: Option<String>,
    error: Option<String>,
    runner: Option<String>,
    runtime_ms: u64,
}

fn inputs(original: &str) -> Vec<Input> {
    let baseline = memory::migration_preview(original);
    let mut inputs: Vec<_> = baseline
        .candidates
        .into_iter()
        .enumerate()
        .map(|(id, c)| Input {
            id,
            line: c.line,
            heading: c.record.project.clone(),
            kind: "paragraph",
            text: c.record.text.clone().unwrap_or_default(),
            sources: c
                .record
                .sources
                .iter()
                .map(|s| s.reference().into())
                .collect(),
            fallback: Some(c),
        })
        .collect();
    let normalized = original
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n");
    let lines: Vec<_> = normalized.lines().collect();
    // Summary inputs include the whole section, so every summary source is still local to its input.
    for (index, line) in lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with("## "))
    {
        let end = lines
            .iter()
            .enumerate()
            .skip(index + 1)
            .find(|(_, l)| l.starts_with("## "))
            .map_or(lines.len(), |(i, _)| i);
        let mut sources = BTreeSet::new();
        for input in inputs
            .iter()
            .filter(|p| p.kind == "paragraph" && p.line > index + 1 && p.line <= end)
        {
            sources.extend(input.sources.iter().cloned());
        }
        inputs.push(Input {
            id: inputs.len(),
            line: index + 1,
            heading: Some(line[3..].trim().into()),
            kind: "project_summary",
            text: lines[index..end].join("\n"),
            sources: sources.into_iter().collect(),
            fallback: None,
        });
    }
    inputs
}

fn issue(input: &Input, reason: &str) -> MigrationIssue {
    MigrationIssue {
        line: input.line,
        text: input.text.clone(),
        reason: reason.into(),
    }
}

fn fallback(preview: &mut MigrationPreview, input: &Input, reason: &str) {
    if let Some(candidate) = &input.fallback {
        preview.candidates.push(candidate.clone());
        preview.fallbacks.push(issue(input, reason));
    } else {
        preview.unplaced.push(issue(input, reason));
    }
}

fn validate(input: &Input, group: Group) -> Result<Vec<MigrationCandidate>, String> {
    if group.records.is_empty() || group.unplaced.is_some() {
        return Err("Return records or an unplaced reason, never both".into());
    }
    let mut used = BTreeSet::new();
    let mut records = Vec::new();
    let context =
        format!("{}\n{}", input.heading.as_deref().unwrap_or(""), input.text).to_lowercase();
    for (index, claim) in group.records.into_iter().enumerate() {
        if claim.text.trim().is_empty()
            || claim.evidence.trim().is_empty()
            || !input.text.contains(&claim.evidence)
        {
            return Err("Claim needs text and a verbatim evidence excerpt from its input".into());
        }
        if input.kind == "project_summary" && claim.kind != RecordType::Project {
            return Err("A project summary must have type project".into());
        }
        if claim
            .project
            .as_ref()
            .is_some_and(|p| p.trim().is_empty() || !context.contains(&p.to_lowercase()))
            || claim.people.iter().any(|p| {
                p.trim().is_empty() || !input.text.to_lowercase().contains(&p.to_lowercase())
            })
        {
            return Err("Project or person name does not occur in this input".into());
        }
        if claim.kind == RecordType::Person && claim.people.is_empty()
            || claim.kind == RecordType::Project && claim.project.is_none()
        {
            return Err("Person and project records need their named subject".into());
        }
        if let Some(date) = &claim.first_seen
            && (chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err()
                || !input.text.contains(date))
        {
            return Err(
                "first_seen must be an ISO date stated verbatim in this input, or null".into(),
            );
        }
        if claim.sources.iter().any(|s| !input.sources.contains(s))
            || (!input.sources.is_empty() && claim.sources.is_empty())
        {
            return Err("Claim has an invented citation or drops all available citations".into());
        }
        // Citation-looking markup in prose cannot bypass the structured source check.
        for source in memory::migration_sources(&claim.text) {
            if !claim.sources.iter().any(|s| s == source.reference()) {
                return Err("Claim text contains a citation absent from its sources".into());
            }
        }
        used.extend(claim.sources.iter().cloned());
        let sources: Vec<_> = claim
            .sources
            .iter()
            .map(|s| {
                if s.starts_with("[[") {
                    MemorySource::Meeting {
                        reference: s.clone(),
                    }
                } else {
                    MemorySource::Kb {
                        path: s.clone(),
                        revision: None,
                    }
                }
            })
            .collect();
        let id = format!(
            "legacy-{}",
            fingerprint([
                input.id.to_string(),
                index.to_string(),
                serde_json::to_string(&claim).unwrap()
            ])
        );
        let source_fingerprint = memory::source_fingerprint(&sources);
        let source_fingerprints = sources
            .iter()
            .map(|s| memory::source_fingerprint(std::slice::from_ref(s)))
            .collect();
        let record = MemoryRecord {
            assignment: None,
            id,
            text: Some(claim.text),
            record_type: claim.kind,
            project: claim.project,
            people: claim.people,
            first_seen: claim.first_seen,
            last_confirmed: None,
            source_fingerprint,
            source_fingerprints,
            sources,
            status: RecordStatus::Proposed,
            basis: Basis::Unknown,
            supersedes: None,
            updated_at: String::new(),
        };
        records.push(MigrationCandidate {
            uncited: record.sources.is_empty(),
            record,
            line: input.line,
            ambiguous: false,
        });
    }
    if input.kind == "paragraph" && input.sources.iter().any(|s| !used.contains(s)) {
        return Err("Paragraph output loses an input citation".into());
    }
    Ok(records)
}

fn apply(original: &str, cache: &Cache) -> MigrationPreview {
    let mut preview = memory::migration_preview(original);
    preview.candidates.clear();
    preview.prepared = true;
    preview.runner = cache.runner.clone();
    preview.runtime_ms = cache.runtime_ms;
    let parsed = cache
        .response
        .as_deref()
        .ok_or_else(|| {
            cache
                .error
                .clone()
                .unwrap_or("Runner returned no output".into())
        })
        .and_then(|s| {
            serde_json::from_str::<Answer>(s)
                .map_err(|_| "Runner did not return the strict JSON envelope".into())
        });
    let inputs = inputs(original);
    let answer = match parsed {
        Ok(answer)
            if answer.paragraphs.iter().all(|g| {
                g.get("id")
                    .and_then(|v| v.as_u64())
                    .is_some_and(|id| id < inputs.len() as u64)
            }) =>
        {
            Some(answer)
        }
        _ => None,
    };
    for input in &inputs {
        let Some(answer) = &answer else {
            fallback(
                &mut preview,
                input,
                cache
                    .error
                    .as_deref()
                    .unwrap_or("Invalid JSON envelope or unknown paragraph id"),
            );
            continue;
        };
        let groups: Vec<_> = answer
            .paragraphs
            .iter()
            .filter(|g| g.get("id").and_then(|v| v.as_u64()) == Some(input.id as u64))
            .collect();
        if groups.len() != 1 {
            fallback(&mut preview, input, "Missing or duplicate paragraph output");
            continue;
        }
        let group = serde_json::from_value::<Group>(groups[0].clone());
        match group {
            Ok(group)
                if group.id == input.id
                    && group.records.is_empty()
                    && group
                        .unplaced
                        .as_ref()
                        .is_some_and(|s| !s.trim().is_empty()) =>
            {
                preview
                    .unplaced
                    .push(issue(input, group.unplaced.as_deref().unwrap()));
            }
            Ok(group) => match validate(input, group) {
                Ok(records) => preview.candidates.extend(records),
                Err(reason) => fallback(&mut preview, input, &reason),
            },
            Err(_) => fallback(
                &mut preview,
                input,
                "Paragraph does not match the strict JSON schema",
            ),
        }
    }
    preview.imported = preview.candidates.len();
    preview.uncited = preview.candidates.iter().filter(|c| c.uncited).count();
    preview.ambiguous = preview.candidates.iter().filter(|c| c.ambiguous).count();
    preview
}

pub(super) fn cached_preview(root: &Path, original: &str) -> io::Result<MigrationPreview> {
    match fs::read(root.join("memory/migration-preview.json")) {
        Ok(bytes) => match serde_json::from_slice::<Cache>(&bytes) {
            Ok(cache)
                if cache.version == CACHE_VERSION
                    && cache.source_fingerprint == fingerprint([original]) =>
            {
                Ok(apply(original, &cache))
            }
            _ => Ok(memory::migration_preview(original)),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(memory::migration_preview(original)),
        Err(e) => Err(e),
    }
}

pub(super) fn prepare(store: &MemoryStore, force: bool) -> io::Result<MigrationPreview> {
    let _lock = store_io::lock(&store.cache_root().join("memory/migration-preview.lock"))?;
    let before = store.preview_migration()?;
    if before.prepared && !force {
        return Ok(before);
    }
    let original = store.legacy_text()?;
    let started = Instant::now();
    let selected = runners::list_runners_at(&store.root).and_then(|runners| {
        runners
            .into_iter()
            .find(|r| r.name == RUNNER && !r.tools)
            .ok_or_else(|| format!("Configure an answer-only runner named {RUNNER}"))
    });
    let mut cache = Cache {
        version: CACHE_VERSION,
        source_fingerprint: fingerprint([&original]),
        response: None,
        error: None,
        runner: None,
        runtime_ms: 0,
    };
    let result = selected.and_then(|runner| {
        cache.runner = Some(runner.name.clone());
        let prompt = format!(
            "{}\nINPUT JSON:\n{}",
            include_str!("memory_migration_prompt.txt"),
            serde_json::to_string(&inputs(&original)).unwrap()
        );
        run_runner(&runner, prompt, &store.root, |_| {})
    });
    cache.runtime_ms = started.elapsed().as_millis() as u64;
    match result {
        Ok(response) => cache.response = Some(response),
        Err(error) => cache.error = Some(error),
    }
    // The model never holds the records lock. Acceptance or source edits during the call invalidate it.
    let _records = store_io::lock(&store.cache_root().join("memory/records.lock"))?;
    if store.preview_migration_locked()?.source_fingerprint != cache.source_fingerprint {
        return Err(invalid(
            "Legacy memory changed while preparing the preview; run it again",
        ));
    }
    store_io::write(
        &store.cache_root().join("memory/migration-preview.json"),
        &cache,
    )?;
    Ok(apply(&original, &cache))
}
