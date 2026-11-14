# Meeting job contract v1

`src/core/meeting_job.rs` is the shared Windows/Linux contract. The bundled
`scripts/meeting-pipeline/job-contract.json` is its script wire vocabulary, compiled into
Rust and installed beside the legacy adapters. PowerShell and Python load that same file;
they reject unknown versions. Existing helper failure variants and queue job states remain
compatible. Old jobs without a stage start at `pipeline`.

Stages describe the current operation, not mandatory checkpoints: pipeline, prepare,
transcribe, translate, notes, render, publish, tasks, compare and title. Engines skip stages
they do not implement. The built-in queue records stage in its existing job journal; legacy
diagnostics retain stage/category/kind/exit/deadline/elapsed. Failure categories remain
HELPER_LIMIT, HELPER_AUTH, timeout and other. UI copy comes from fixed allowlisted causes,
never from command output, paths, transcripts or configuration.

Both engines acquire `.notes-engine.lock` in the meeting folder before processing and hold
the OS handle through publication. Nested script notes reuse and validate the owning handle.
Only `store_io` and the legacy PowerShell adapter implement the platform lock operation.
The lock file is never deleted: its existence does not imply ownership. Contention defers
the job without consuming an attempt. A crashed process loses ownership immediately.

On restart, queued/running work may reacquire the lock; a stored stage never authorizes
skipping validation or executing two producers. Recheck completed output under that lock.
An existing `english-summary.json` completes automatic work and is preserved, even if a
failure marker or running journal was left behind. No new journal or completion marker is
introduced. Explicit script NotesOnly replay retains its existing transcript and output
backup rules; standalone notes invocation is an explicit replay. Built-in sorting/task
replay retains its existing summary-preservation behavior. Typed user `notes.md` is input.
This contract does not claim transactional render/export/task publication: those ports and
engine-specific retry spacing/discovery policy remain later retirement stages.

Release tests cover competing claims, released claims, interrupted jobs, existing-output
preservation, old journal decoding, stage attribution and allowlisted UI causes. They must
run in the release check; no unit tests or installed-app acceptance run on the laptop here.
