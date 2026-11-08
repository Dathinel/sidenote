# ADR 0001: Domain model and storage seam

Status: accepted, 2026-11-08.

## Context

Shared state lives in about ten logical JSON stores (`<meeting>.json`, `all.json`, `projects.json`,
`task-overrides.json`, `memory/records.json`, `checks/<date>.json` and others). `sync_state.rs`
merges them from device journals. Eight modules call it directly through `load`, `update`,
`save_changes`, `new_id`, `stores` and `reconcile`: `persistence.rs`, `tasks.rs`, `memory.rs`,
`check_state.rs`, `daily_check.rs`, `config.rs`, `meetings.rs` and `agent_tools.rs`. Meeting text is Hex folder files, read twice:
by `meetings::scan` for the UI and by `agent_tools::Catalog` for MCP.

Today's ids do not survive what comes next. Meeting ids are Hex folder names (`<digits>-<digits>`)
that collide across devices. Chat messages are arrays under a meeting key. Tasks are keyed by
text and index. Chat threads, people, workspaces, a server and possibly a second sync engine
all need ids that every device and the server agree on.

## Decision

1. **Ids.** Every entity has a UUID. New entities get UUIDv7 (time-ordered). Entities derived
   from an outside key get UUIDv5 under a fixed namespace per kind, so every device computes the
   same id with no coordination. Meeting id = `v5(NS_MEETING, "hex/" + device_id + "/" + folder)`.
   The old key stays as `legacy_key`; MCP accepts both forms for one major version.
2. **Entities.** Workspace (personal or team), Meeting, Transcript, Note (generated summary,
   user notes, scratchpad), Task, Project, MemoryRecord, ChatThread, ChatMessage, Person,
   AttendeeSuggestion, Device, DailyReport and Proposal. Person and AttendeeSuggestion have no
   audio or embedding fields, by design (C1). A suggestion carries its evidence: calendar,
   transcript self-introduction or manual.
3. **Envelope.** Every entity has `id`, `workspace_id`, `schema` (u16), `created_at`,
   `created_by` (device or user), `updated_at`, `deleted` and optional `provenance`
   (harness, model, source).
4. **Storage classes.** Records (small, field-mergeable) go through the sync engine. Content
   (transcripts, long note bodies) is a content-addressed blob that the record points to.
   Device-local data (`config.json`, `device.json`, keys, indexes, jobs) never syncs.
5. **Agents write proposals only.** `propose_memory` becomes the first Proposal kind. Chat
   summaries, task completion and new projects follow the same path: pending until a person accepts.
6. **The seam.** A `store` module with API version 1:

   ```rust
   pub trait Store: Send + Sync {
       fn get<E: Entity>(&self, id: Id<E>) -> Result<Option<Versioned<E>>>;
       fn query<E: Entity>(&self, q: &Query<E>) -> Result<Page<Versioned<E>>>;
       fn apply(&self, batch: Vec<Op>) -> Result<Applied>; // Put fields, Delete, Append
       fn changes(&self, since: Cursor) -> Result<Changes>; // UI refresh, indexer, sync
       fn blob(&self, hash: &BlobId) -> Result<Option<Vec<u8>>>;
       fn put_blob(&self, bytes: &[u8]) -> Result<BlobId>;
   }
   ```

   `Op::Put` carries field patches and an optional expected version. Engines behind it:
   `JournalEngine` now, `AutomergeEngine` only if ADR 0002's gate passes, and `RemoteEngine`
   (server API) for team workspaces. `sync_state` becomes private to `store::journal`; a CI check
   fails if any file outside `src/core/store/` names it.
7. **Versioning.** Additive changes keep API 1. Each entity has upgrade functions that run on
   read; writers always write the newest schema; a reader that meets a newer major schema stops
   with "Update Meetings on this device" instead of guessing.
8. **Shared crate.** `crates/meetings-domain` (MIT OR Apache-2.0, no IO) holds ids, entities,
   envelope, Proposal and Provenance. The desktop app and the server both use it.

**What moves first:** (1) ticks and chat messages in `persistence.rs`, because chat threads come
next; (2) the meeting catalog, so UI and MCP share one `MeetingSource`; (3) tasks and projects;
(4) memory and proposals; (5) daily reports and triage.

## Options rejected

- SQLite as the main store: a database file does not merge over file sync; keep it as a rebuildable index only.
- One trait per store (today's shape): every new engine would need an adapter for each store.
- An ORM (Diesel, SeaORM) in the client: SQL-shaped, does not fit journals or Automerge.
- Random v4 ids for imported data: two devices importing the same meeting would disagree.
- ULID: same ordering as UUIDv7, but Postgres and the `uuid` crate support UUIDv7 directly.

## Consequences

- Swapping the sync engine touches one module, not the UI, MCP or pipeline.
- Server and client share one set of types, so team workspaces need no second model.
- A one-time refactor of eight modules, and a period where MCP accepts two id forms.
- Schema upgrades need discipline: every change ships with an upgrade function and a test.

## Revisit if

- The seam needs more than two engine-specific escape hatches.
- Queries outgrow in-memory filtering (then add a local SQLite index behind `query`).

## First step

Roadmap lane F1: add `crates/meetings-domain` with `Id<T>`, v7 and v5 constructors, the meeting id
function with golden test vectors, the envelope and Provenance. No behavior change.
