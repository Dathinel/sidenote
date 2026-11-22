# Company memory and daily check v2

> Storage update: [multi-device sync](../sync.md) now keeps authoritative items in immutable
> device journals. The `memory/records.json` name below is a logical store and a legacy import
> source. Generated company text, accepted legacy copies and migration preview caches now live
> under `local/memory/`. Examples below describe the storage contract, not a live-data verification.


Decisions needed: none. This is the Rust data contract for the next UI lane.

All paths below are relative to `core::state_dir()`. `SIDENOTE_STATE_DIR` redirects that root. Meeting input still comes from `core::meetings_dir()` under APPDATA. Constructors taking a root are useful for previews and isolated copies; the UI normally uses `Default`.

## Files and write boundaries

| Path | Shape and owner |
| --- | --- |
| `memory/records.json` | Schema 1: `{schema_version, records: [], proposals: [], events: [], migration: null or object}`. Authoritative memory store, including proposal decisions and tombstones. |
| `memory/company.md` | Until migration acceptance, the original legacy memory. Afterwards, a generated projection of active records, with sources and IDs. |
| `memory/migration-preview.json` | Versioned cache of the runner response, source fingerprint, runner name, error and runtime. Validated against the current source on every read. Failed attempts are cached too. |
| `memory/company.legacy.md` | Exact bytes of the original memory, created on migration acceptance, never overwritten with different bytes. Existing `memory/history/` files are untouched. |
| `checks/YYYY-MM-DD.json` | New reports use schema 2. Reports without a version load as schema 1. Ordinary runs remain idempotent; `--force` can replace today's report. |
| `checks/triage.json` | Schema 1: `{schema_version, findings: {finding_id: entry}}`. Durable across report dates and reruns. |
| `checks/state.json` | Legacy date to `index\|title` marks. Read and written only for schema 1 reports. No guessed migration to cross-day identities. |
| `memory/records.lock`, `checks/triage.lock` | OS-released exclusive writer locks. A competing writer returns an I/O error; retry after refreshing. |

Stores reject unknown schema versions and propagate malformed JSON and I/O errors. Writes use a flushed temporary file and the existing Windows atomic replacement primitive. The records file and Markdown projection are separate replacements: if projection fails after the JSON commit, the operation returns an error, but the JSON change may already be stored. Refresh records before retrying. `MemoryStore::company_memory()` repairs the export from authoritative records. Identical exports are not rewritten.

Daily checks never apply task actions or accepted memory changes. They validate and write a report, then stage its proposals. Queue reads recover proposals from committed reports if a process stopped between those two writes. Proposal IDs deduplicate replay, including accepted and rejected proposals. A failed report validation writes neither a report nor proposals. A proposal-queue I/O failure after report commit is recoverable through `list_pending_proposals()`.

## Memory shapes

```json
{
  "id": "legacy-... or memory-...",
  "type": "person|project|decision|commitment|question|story",
  "text": "One claim or preserved legacy paragraph",
  "project": "Project name or null",
  "people": ["Person name or ID"],
  "sources": [
    {"type": "meeting", "reference": "[[1800000000000-1@21:10]]"},
    {"type": "kb", "path": "projects/projects.yaml", "revision": null}
  ],
  "first_seen": null,
  "last_confirmed": null,
  "status": "proposed|active|superseded|retired|forgotten",
  "basis": "owner_confirmed|source_supported|inferred|unknown",
  "supersedes": null,
  "updated_at": "RFC3339 timestamp",
  "source_fingerprint": "stable hash of the source set",
  "source_fingerprints": ["stable hash of each source"]
}
```

KB revision is null only when legacy provenance does not establish it. The engine stamps the actual captured revision onto model proposals. `first_seen` and `last_confirmed` are nullable strings, not dates invented from ingestion. Imported candidates keep last_confirmed null and basis unknown. first_seen stays null unless an exact ISO date is stated as the first observation in the source; other dates remain in the claim text without inventing a year or treating a due date as confirmation. Migration acceptance approves import, without asserting every imported claim is freshly confirmed. Accepted model changes and explicit owner confirmations use the current RFC3339 timestamp and owner-confirmed basis. Repeated ingestion changes neither record date.

The `project` and `people` fields are grouping references; v1 has no separate entity graph or canonical identity service. Migration uses the answer-only runner to split clauses into person, project, decision, commitment, question and story records. Project and people names must occur in the source paragraph or its heading. Person records retain only stated roles or involvement. Separate section inputs support a project summary with its purpose and current stated state; generic grouping headings are not treated as projects.

Forgetting removes `text` from JSON, clears project, people and sources, and leaves the record ID, forgotten status and source fingerprints. Events contain IDs and operations, not copies of claim text. Related proposals inside this store are removed too; report recovery skips their forgotten source sets. Exact source sets and subsets consisting entirely of one tombstone's sources are blocked, regardless of new wording. A genuinely new cited source can still support a new proposal for owner review. This conservative rule can also suppress a different claim using the same old sources. Existing reports, the legacy backup, history and source meetings retain their contents; historical-copy cleanup is outside v1.

A queued proposal is:

```json
{
  "id": "deterministic-proposal-hash",
  "op": "add|confirm|supersede|retire",
  "target_id": null,
  "text": "Complete proposed claim; current text for confirm/retire",
  "record_type": "story",
  "project": null,
  "people": [],
  "sources": [{"type": "meeting", "reference": "[[1800000000000-1]]"}],
  "rationale": "Why this needs owner approval",
  "target_updated_at": null,
  "status": "pending|accepted|rejected",
  "created_at": "RFC3339",
  "decided_at": null
}
```

Report `memory_proposals` contain the proposal portion, without queue status or decision timestamps. `target_id` is required for every operation except add. The engine captures `target_updated_at` from the records included in its input, and assigns the ID. Acceptance refuses a stale or inactive target. Supersede marks the old record superseded and inserts an active record with `supersedes` pointing to it. Retire keeps the text. Confirm requires a source absent from the target's source list; an explicit owner confirmation uses `confirm_record` instead. Rejection records the decision and does not change any record.

## Core API for the UI

Import `sidenote::core::memory::*`. All operations return `std::io::Result` and should run off the render path.

| Signature on `MemoryStore` | Behaviour |
| --- | --- |
| `default() -> MemoryStore` / `new(root: PathBuf) -> MemoryStore` | Use application state or an explicit root. |
| `load(&self) -> io::Result<MemoryFile>` | Full authoritative state, proposal decisions, events and migration receipt. |
| `snapshot(&self) -> io::Result<(MemoryFile, String)>` | Consistent records and Markdown under one lock; the daily prompt uses this to guard concurrent owner edits. |
| `list_records(&self) -> io::Result<Vec<MemoryRecord>>` | All statuses, including superseded/retired records and tombstones. UI filters by type, project, people and status. |
| `company_memory(&self) -> io::Result<String>` | Legacy memory before migration; afterwards repairs and returns active-record Markdown. Also exported as existing `core::company_memory()`. |
| `preview_migration(&self) -> io::Result<MigrationPreview>` | Read-only. Revalidates a matching cached response, or returns the deterministic preview with prepared=false. Includes counts, candidates, source line numbers, fingerprint, runner/runtime, unplaced text and fallback reasons. Refuses an already completed migration. |
| `prepare_migration(&self, force: bool) -> io::Result<MigrationPreview>` | Slow worker-only operation. Uses the matching cache unless forced. Calls the configured answer-only runner and caches its response or failure. Rechecks source fingerprint and migration status before saving. |
| `accept_migration(&self, expected_fingerprint: &str) -> io::Result<MigrationPreview>` | Revalidates cached output without calling the model, refuses changed source or conflicting backup, writes exact backup and activates all candidates atomically in records.json. Regenerates company.md. |
| `list_pending_proposals(&self) -> io::Result<Vec<PendingProposal>>` | Recovers committed-report proposals, then returns pending items. Does not apply them. Propagates malformed report errors. |
| `accept_proposal(&self, id: &str) -> io::Result<()>` | Owner action only. Requires accepted migration, a pending proposal and unchanged target; applies and logs the decision. |
| `reject_proposal(&self, id: &str) -> io::Result<()>` | Owner decision, leaves memory unchanged and records rejection. |
| `edit_record(&self, id: &str, text: &str) -> io::Result<()>` | Immediate active-record text edit, rejects empty text, records owner-confirmed basis and edit event. Does not imply fresh source confirmation. |
| `confirm_record(&self, id: &str) -> io::Result<()>` | Immediate owner confirmation and confirmation timestamp/event. |
| `retire_record(&self, id: &str) -> io::Result<()>` | Immediate active to retired transition and event; text remains available in history. |
| `forget_record(&self, id: &str) -> io::Result<()>` | Any existing record to tombstone, scrubs in-store proposals and regenerates active export. |

Engine-facing helpers are `stage_proposals(&self, proposals: &[MemoryProposal]) -> io::Result<()>`, `proposal_id(p: &MemoryProposal) -> String`, and `source_fingerprint(sources: &[MemorySource]) -> String`. They are not UI approval shortcuts.

Migration keeps blank-line paragraphs and top-level Markdown bullets intact, including nested/continuation lines. It preserves citation order and deduplicates repeated citations within a record. Headings become project group labels. Multiple sentences or uncertainty language mark a candidate ambiguous; no semantic splitting, merging or inferred chronology is attempted. Counts overlap: an uncited candidate may also be ambiguous. The owner accepts the preview as a whole in v1, then uses record edits/retire/forget controls. Rejecting or closing the preview simply leaves legacy memory in use. An empty legacy store can be accepted to initialize a new memory.

## Report schema 2

`core::daily_check::{DailyCheck, CheckContent, Finding}` retain the old public fields with deserialization defaults. New reports omit empty `story`, `summary` and `memory_update`; they do not contain a full company-memory copy. The current page reads `brief` and crudely renders `story_alignment`, retaining the existing Apply actions and v1 rendering.

The outer report adds `schema_version: 2`; date, runner, timing, counts, KB snapshot and warnings remain. The flattened content contains:

- `brief: Vec<String>`: exactly three nonempty single-line strings, ordered new/changed, biggest open issue, next due.
- `coverage: {meeting_count, task_count, kb_revision, kb_captured_at, warnings}`: assigned by the engine, never trusted from model output.
- `findings: Vec<Finding>`: old kind/severity/title/explanation/evidence/action fields, plus `id`, `project`, `subject_refs`, `subject_keys`, `evidence_keys`, `first_seen`, `last_seen`, `change` (`new`, `changed`, `still_open`) and `material_state`.
- `story_alignment: [{project, status, kb_claim: {text, sources}, meeting_claim: {text, sources}, question}]`: status is aligned, tension, unknown or needs_decision. Both sides need valid citations except an explicitly unknown side. No score.
- `memory_proposals: Vec<MemoryProposal>`: pending suggestions only.
- `comparison: {baseline_date, complete, new, changed, carried, resolved, unmatched}`: arrays of IDs. Unmatched legacy findings use `legacy:YYYY-MM-DD:index` as a locator, without inventing durable IDs for old reports.
- `resolutions: [{finding_id, explanation, evidence}]`: positive-evidence proposals for absence from the current findings. Resolution requires a complete scan and at least one newly cited source. Omission alone yields unmatched. Citation validation checks existence and syntax; it cannot establish that the text proves the interpretation.

Finding identity requires matching kind, exact canonical subjects, original task text where applicable, and overlapping source references. Titles and explanations play no part. Subject forms are `task:meeting-id:index`, `memory:record-id`, or `project:project-name|specific-stable-subject`. The last form is a model-supplied scope key; ambiguous changes to it produce a new finding. One-to-many or many-to-one matches do not inherit IDs. `subject_keys` include an engine hash of the original task text so regenerated task indices do not silently match another task. Duplicate generated IDs reject the run.

Comparisons use the previous saved report before today, including on `--force`. Matched IDs retain first_seen; last_seen is today. A changed source fingerprint, severity, action or factual `material_state` gives changed; paraphrased titles alone do not. Without a unique match the engine assigns a deterministic new ID. An issue returning after a gap can reuse its historical ID and first_seen only after the same conservative one-to-one match against the latest observation of each historical identity. It counts as new against the immediately previous report. No semantic or title-similarity matching is attempted. Comparison is incomplete if either current or baseline coverage has warnings or a stale snapshot.

Source existence, action targets/values and proposal source syntax are validated before commit. KB proposal revisions come from the captured snapshot. The prompt uses plain words and contains no em dash. Full memory copies are removed from old reports before passing their content back to the model; old files remain untouched. Original meetings and historical story text remain source context.

## Durable triage and Apply

Import `core::check_state::{TriageStore, TriageStatus, TriageEntry}`. Entry shape:

```json
{"finding_id":"finding-...","status":"open|dismissed|snoozed|applied","updated_at":"RFC3339","snooze_until":null,"evidence_fingerprint":"...","evidence_keys":[],"note":null}
```

| Signature on `TriageStore` | Behaviour |
| --- | --- |
| `default() -> TriageStore` / `new(root: PathBuf) -> TriageStore` | Application or isolated state root. |
| `load(&self) -> io::Result<TriageFile>` / `list(&self) -> io::Result<Vec<TriageEntry>>` | Raw durable entries, including handled/expired items for filters and history. |
| `set_finding(&self, finding: &Finding, status: TriageStatus, snooze_until: Option<NaiveDate>, note: Option<String>) -> io::Result<TriageEntry>` | Explicit owner mark. Requires a prepared v2 finding; snoozed requires a future date. Open is undo. Does not undo a task edit. |
| `status_for(&self, finding: &Finding, today: NaiveDate) -> io::Result<TriageStatus>` | Effective status: returns open on new evidence or expired snooze. Does not erase the stored mark. |
| `marks_for_report(&self, report: &DailyCheck, today: NaiveDate) -> io::Result<Marks>` | Adapter for the old page's `index\|title` rendering; snoozed appears handled there. |

`check_state::load(date)` and `set(date, key, mark)` retain their signatures. They route v2 reports to triage and v1 reports to the untouched legacy format. Legacy marks never transfer to another date. New screens should use TriageStore directly.

Material evidence keys are exact meeting citations and content hashes of cited KB files, sorted and deduplicated. A wording change or removal/reordering of evidence does not reopen a handled item. A newly cited moment/source or changed cited KB file does. KB hashing is file-level in v1: a changed cited file can reopen an item even when the changed section is unrelated; there is no validated excerpt model yet. A KB revision change alone does not reopen it if the cited file bytes are unchanged.

Existing `core::apply_action(action: &CheckAction) -> io::Result<()>` remains the action entry point for set_owner, set_due, set_project, mark_done and note. It rescans meetings and checks the exact expected task text, index, meeting and value. On success, call `set_finding(..., Applied, None, None)`. On action failure leave triage unchanged. Task writes and triage are separate commits: if the action succeeds but marking fails, refresh before offering Apply again, especially for append-only notes. The old page retains this behaviour.

## Migration preview and validation

Opening Memory starts `prepare_migration(false)` on a worker. The UI remains usable and acceptance is paused while it runs. The banner shows unplaced source text and validation fallback reasons, with line numbers. Run preview again forces a fresh attempt. The dedicated `Memory migration (answer only)` runner is installed from defaults when missing and excluded from Ask, like the Daily check runners. Its default is the same tool-free Sonnet configuration as the existing answer-only runner. Runner configuration is read from the store root, including an isolated state directory.

The input includes every deterministic paragraph with a stable input id, heading, exact text and extracted sources. Additional project-summary inputs contain each section's text and sources, so summaries cannot borrow citations from another section. The model must return a strict JSON envelope and one group per input. A group contains typed records or an explicit unplaced reason. Rust validates each paragraph independently:

- Known record type and schema, nonempty claim text, and a nonempty verbatim evidence excerpt from that input.
- Every source reference must exactly match that input's source list, including meeting timestamps. Every cited paragraph retains all its input citations across the output records, and each claim in a cited paragraph has sources. Citation markup in claim text must also be declared in its sources.
- Project names occur in that input or its heading; people occur in that input. Project and person records need their named subject. Project-summary inputs produce project records only.
- first_seen is unknown unless it is a valid YYYY-MM-DD date occurring verbatim in that input. last_confirmed remains unknown.
- Missing, duplicate or invalid paragraph groups fall back to that paragraph's unchanged deterministic candidate, with a visible reason. Malformed JSON or unknown input ids fall back for the entire response. Invalid project summaries and deliberately unplaced groups retain their full source text in the unplaced list.

The cache stores raw output rather than trusting serialized records. Reading and accepting rebuild the candidates with current validation. The cache is keyed by exact source fingerprint and cache version. Runner failure is also a completed cached attempt, with deterministic fallback, so reopening does not repeatedly call the model. Invalid/stale cache content is ignored. A separate preview lock prevents competing model runs; the records lock is held only while committing the cache or accepting. A model call never writes records, the legacy original, its backup or the active projection. Acceptance preserves the existing optimistic fingerprint guard and exact company.legacy.md backup. Unplaced text remains in that backup and in the visible preview.

Validation establishes source membership, structural coverage and literal names/dates. It cannot prove semantic entailment, that a citation supports the model's interpretation, or that every clause was split correctly. The owner still reviews and accepts the preview. No knowledge graph, automatic expiry/application, numeric alignment score, or historical-copy cleanup is included.

## Validation and boundaries

Required gates: `cargo clippy --locked --all-targets -- -D warnings` and `cargo build`, from this worktree with the supplied GNU tools first on PATH. No tests or GUI launches. Model validation is authorized only on an isolated temporary copy, never real memory or the KB repository.

`examples/memory_migration.rs` refuses any root outside a `hex-memory2-...` directory under the OS temp folder and requires `SIDENOTE_STATE_DIR`. Copy company.md and runners.json there, build the example, then execute it directly. `--model` explicitly prepares a fresh model preview; without it the example makes no model call and reuses the cached or deterministic preview. `--preview-only` writes the inspection artifact and stops before acceptance. It reports type counts, fallback/unplaced details and model runtime, and verifies cached reproduction. Acceptance operates only on the copy, re-fetches every imported record, compares every candidate field except the expected acceptance status/timestamp changes, and the exact backup bytes, and reports copied old-report load counts. Its inspection artifact is migration-preview.json at the temporary root; the internal response cache is memory/migration-preview.json.
