# ADR 0007: Chat history

Status: accepted, 2026-11-08. Builds on ADR 0001 (entities, Store) and ADR 0006 (provenance).

## Context

`persistence.rs` stores Ask messages as an array per meeting key, plus one array under `all`
for Ask across meetings. A message has `role`, `text`, `id`, `at` and `legacy_order`, but no
thread, no title and no record of which harness or model answered (finding C3). The owner wants
history that can be found, searched, exported and traced, and wants external agents to be able
to save what they learned.

## Decision

**Entities.**

```text
ChatThread  { id (v7), workspace_id, title, title_source: user|auto,
              scope: Meeting(id) | AllMeetings | Project(id) | None,
              created_at, updated_at, pinned, archived, deleted }
ChatMessage { id (v7), thread_id, role: user|assistant|external_summary|note,
              text (inline up to 32 KiB, else a content blob), created_at, device_id,
              status: complete|failed|stopped,
              provenance: { harness, harness_version, model, model_source: reported|requested|unknown,
                            agent_name, task_kind, duration_ms, tools_used,
                            context: { meeting_ids, capped } },
              citations: [{ meeting_id, at_ms }] }
```

Journal keys are `[thread, id, field]` and `[message, id, field]`. A completed message is
immutable except for `status`. Nothing here depends on the engine.

**Provenance rules.**
- Record the model the harness reports: Claude's stream-json, Codex's JSON events, or ACP session info.
- If the harness reports nothing, record the requested model with `model_source = requested`.
- Never guess. Legacy messages get `unknown`, shown as "Model not recorded".

**Migration.**
- Each legacy `<meeting>.json` message list becomes one thread with scope Meeting, titled "Questions about <meeting title>". `all.json` becomes one thread with scope AllMeetings, titled "Across meetings".
- The thread id is `v5(NS_THREAD, legacy store key)`, so every device migrates to the same thread.
- Message ids are kept (`sync_state` already assigned them). Order is `(at, legacy_order)`.
- After migration, writes go to the new keys and old keys are only read. "Clear history" becomes "Delete thread".
- Ask context stays at the last 12 messages, 4,000 characters each, for now.

**Search.**
- **Index:** a local full-text index in `local/index.sqlite` (rusqlite with bundled SQLite FTS5) covering meetings, notes, tasks and chat.
- **Rebuildable, never synced:** the index is built from the `Store` and rebuilt when its schema version changes.
- **Filters:** scope, harness, model and date.
- **Until the index exists,** chat search uses today's in-memory literal search.

**Export.**
- Per thread or all threads, as JSON (schema `meetings.chat.v1`, provenance included) or Markdown (front matter with provenance).
- Available from the UI and from `hex-meetings.exe --export-chats <folder>`.
- The app can import its own JSON to restore.

**MCP tool `propose_chat_summary`** (local and remote, ADR 0005):

```text
title, summary (up to 16 KiB), key_points[], decisions[], follow_ups[],
related_meeting_ids[], source: { client_name, model?, conversation_ref? }
```

- **The tool creates a pending Proposal** of kind `chat_summary`. A repeat with the same content hash within 7 days returns the existing proposal id.
- **The user reviews it in Meetings:**
  - Edit, then Accept: creates a ChatThread with one `external_summary` message carrying the client's provenance.
  - Reject: drops it.
- **Remote proposals** land in that workspace's queue, visible per the ACL.
- **The tool is off unless the new `chat_summaries` feature is on in `config.json`,** like the other optional features.

## Options rejected

- Importing full chat logs from other harnesses: each format differs, and it copies far more private text than a summary.
- A separate SQLite database as the source of truth for chats: bypasses the seam and sync.
- Tantivy for local search: larger binary and its own index format; FTS5 is enough at this size.
- Writing summaries straight into history from MCP: agents write proposals only (ADR 0001).

## Consequences

- Every answer can be traced to a harness and model, or is marked unknown.
- Chats sync, export and copy to a team workspace like any other entity.
- One extra local file (`index.sqlite`) that can always be deleted and rebuilt.

## Revisit if

- Users want to resume an agent session across harnesses: store ACP session ids on the thread.
- Search across all entities becomes slow on the heavy-year corpus: tune FTS5 or move to Tantivy.

## First step

Roadmap lane C1: thread and message types in `meetings-domain` and the migration function,
tested on synthetic legacy stores from two devices (same thread ids on both).
