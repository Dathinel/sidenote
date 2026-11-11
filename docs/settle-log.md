# Automatic settle history

The requested base is `release/dev12` (`d1ec063`). This slice records existing app-owned task and project decisions. It does not introduce a new automatic approval policy for Memory or People.

## Automatic change inventory

The audit reviewed production Rust writers, their callers, the notes scripts entry point, and agent tools. The references below are to this branch. Source review is evidence rung 2; the logged writes and safety rules have executable tests (rung 4), with native UI Undo and Redo checked separately (rung 5).

| Existing change | Writer or source | Settle coverage |
| --- | --- | --- |
| Initial task owner, date, guessed date, team, waiting status/dependency and subtasks inferred from a meeting | `src/core/notes/queue.rs:454`; normalization in `src/core/notes/schema.rs:206` | Logged as `task_properties`, one entry per changed task. Undo restores these properties while retaining the task and its source text. |
| Missing or invalid task date becomes three weekdays after the meeting | `src/core/notes/schema.rs:148` and `src/core/notes/schema.rs:258` | Included in the same task entry, including `due_default`. |
| Suggested meeting project, per-task project and whether each task is ours | `src/core/notes/queue.rs:489`; `src/core/notes/schema.rs:304` | Logged as `project_assignment`, one entry for the meeting's complete sorting result. |
| Task regeneration and sorting again | `src/core/notes/queue.rs:785` and `src/core/notes/queue.rs:795` | Uses the same logged entry points. These requests require a click, but their model-selected results use the same safety path. |
| Existing person names and aliases combined into the displayed roster | `src/core/people.rs:34`, `src/core/people.rs:71`; `src/core/tasks_v2.rs:168` | Display-time resolution. No stored people merge is performed. Team and alias editing is an owner action (`src/core/tasks_v2.rs:244`). |
| Waiting task becomes available when its blocker is done | `src/core/tasks_v2.rs:112` | Display-time calculation only; the stored waiting status is unchanged. Not logged as a stored settlement. |
| Daily findings and suggested Memory additions, replacements or retirement | `src/core/daily_check.rs:1013`, `src/core/daily_check.rs:1037`; `src/core/memory.rs:398` | Report and pending proposal writes only. No Memory record is accepted or retired automatically. Existing owner decisions retain the previous Memory event history and Undo. |
| Ask agent proposes an addition to Memory | `src/core/agent_tools.rs:594`; `src/core/memory.rs:402` | Queues a proposal requiring owner review; it cannot accept it. Not presented as a settled item. |
| Memory proposal acceptance, confirmation, retirement and task completion from Daily Check | `src/core/memory.rs:459`, `src/core/memory.rs:697`, `src/core/memory.rs:703`; `src/core/daily_check.rs:257`, called by `src/ui/check.rs:667` | Owner actions. Not automatic settlements. |
| Cross-meeting automatic completion of an old task, automatic resolution between Memory versions/dates, or persistent automatic people/alias merging | Inspected the notes schema/queue, task stores, People roster, Memory store, Daily Check and agent tools | No such autonomous writer was identified on this base. These kinds have no fabricated history or Undo in this slice. The notes schema accepts only `todo` and `waiting` (`src/core/notes/schema.rs:212`). |
| External notes script writes | `src/core/notes/mod.rs:103`; external publication preserved at `src/core/notes/queue.rs:446` | Not intercepted. An arbitrary script owns its files and does not call the app's write-ahead API. Existing script results are preserved. To log them safely, the script must use an app-owned commit protocol in a later slice. |

Generated source artifacts and infrastructure also write automatically, but do not settle task/people/Memory properties: quick meeting titles and narrative summaries (`src/core/notes/queue.rs:393`, `:454`), transcript text (`src/core/notes/transcript.rs:75`), transcription/normalization jobs (`src/core/meeting_router/jobs.rs:332`, `:417`), annotation model output (`src/core/annotations/generation.rs:453`), user-note autosave/recovery (`src/core/live.rs:172`, `:183`), pending-report reconciliation (`src/core/memory.rs:429`), and sync import/journals/backups (`src/core/sync_state.rs:330`). Their generated text, job receipts, caches, UI preferences and migration bookkeeping are outside settle history. The summary's task properties are logged even though its narrative is not.

## Storage and recovery

`memory/settle-log/` is a single device-local append-only log represented by immutable JSON intent and outcome records. This uses the existing flushed, atomic publication primitive instead of introducing a JSONL append that can tear. Existing stored keys are retained.

Each intent includes a checksum, device identity, causal sequence and the complete source-file before/after values needed for recovery. Its entries include the requested ID, time, kind, item ID/label, source meeting/offset, plain reason, confidence and logical before/after JSON. An intent is flushed and published before the data write. The outcome is published after it. Startup and the notes worker roll pending intents forward, recognize already-landed writes, and skip divergent or deleted sources without overwriting them.

Existing extractors expose no calibrated confidence value, so `confidence` is `null`. The project sorter supplies no transcript timestamp, so project entries have `source_offset: null`; task entries retain their cited offset when available. Neither value is invented.

Undo checks the current logical after value, source task identity/content, and versions of relevant owner edits, including deleted edit tombstones. Project Undo also checks the source task list and newer task/meeting project assignments. A deleted meeting or task is not recreated. Each inverse operation is another intent. Undoing it is Redo. Concurrent edits are serialized through the existing shared-store writer locks, with the settle log's own local lock protecting operation order.

Task regeneration retains existing override/tick remapping (`src/core/tasks_v2.rs:543`). Those owner-selected values are not overwritten by Undo.

The log is deliberately outside `state/` and is not synced. See `docs/sync.md`. History is forward-only from this slice; existing meetings are not backfilled with invented decisions.

## UI and meeting integration

Memory shows the last seven days, newest first, at most five rows, plus See all. Each row has its label, reason, source meeting, available transcript offset, wall time and Undo. An undone row offers Redo, including after restart. The toast offers the same inverse action. An empty automatic section stays hidden.

The requested dev.12 base predates the approved Reading rail. The existing meeting summary view uses `src/ui/memory/settled.rs`'s `meeting_footer`, and `core::settle::Store::meeting_count` is available for the dev.13 integration. It counts active original automatic entries, excluding currently undone entries and inverse operations. See in Memory opens this meeting's settled history.

## Validation

`src/core/settle/tests.rs` covers both logged kinds: immutable logging, Undo, Redo, later owner edits, tombstones, deleted sources, pending-before-data recovery, data-before-outcome recovery, inverse-operation recovery, project reassignment/deletion, and independent multi-task Undo/counts. The notes-worker integration test checks that real extraction writes both kinds.

`tools/render_settle_review.py` produces the light/dark native render matrix with isolated invented meetings, calling the same write-ahead APIs as production. `tools/verify_settle_review.py` checks real source data after Undo/Redo, toast Redo, persistence after restart, source navigation, meeting counts/history and four pixel-identical Settings General references built from the requested dev.12 base. Gate and render receipts are in `artifacts/memory-settle/`.
