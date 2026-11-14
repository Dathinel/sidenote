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

## Built-in checkpoint contract (retirement stage 1)

`meeting_job::checkpoint::Job` is a dormant shared coordinator, not a change to active
engine selection or the notes queue. It uses the same Stage and Cause types and the same
meeting claim. Its fixed order is discover, prepare (audio), route, transcribe, translate,
notes, tasks, render, publish. Existing script wire vocabulary remains version 1.

Each input identity has a bounded versioned journal in `.sidenote-jobs/<digest>.json`.
The caller supplies a 32-byte digest covering meeting identity, inputs, configuration, adapter
versions and available capabilities. No paths, helper output or meeting content are stored.
Previous identities are retained so switching inputs A -> B -> A does not rerun A.
New capabilities require a new identity; NotAvailable is recorded without failing the job.
The journal stores a completed prefix and a content-free Cause for the active Stage.
IO errors use `failure.rs`'s redacted MEETING_JOB boundary; invalid journals fail closed.

The claim covers journal reads, flushed atomic replacement and all adapter calls. A
completed stage is never called again for that identity. Before calling a new stage the
coordinator durably records its start and supplies a stable version/input/stage key.
After interruption or typed failure the same stage receives Recover, never another Start.
An adapter must reconcile a durable receipt for that key, returning committed results or
finishing uncommitted work idempotently. It must not repeat committed effects. Arbitrary
external effects cannot be made exactly-once by a journal alone; later adapters must
provide atomic publication or destination-side key deduplication before being wired in.
An interrupted save reloads the journal before another call. Missing adapters return
NotAvailable. No real processing, helper, UI, discovery or external effects are added here.

Release tests cover competing claims, released claims, interrupted jobs, existing-output
preservation, old journal decoding, stage attribution and allowlisted UI causes. They must
run in the release check; no unit tests or installed-app acceptance run on the laptop here.
