# Meeting pipeline scripts

Scope: the entry scripts and their called helpers in `scripts/meeting-pipeline` are versioned from the
installed source snapshot after a private-data scan. No pipeline was executed.
Import commits keep file boundaries where practical. Private defaults must be
supplied through the environment before deployment; processing logic is retained.

`hex-meeting-english.ps1` accepts `auto`, `other`, or the project key supplied in
`HEX_MEETING_PROJECT`. Set that environment variable to the prior private
project key when explicitly selecting that project. The default remains `auto`.
Private people, company context, credentials and runtime configuration are not packaged.

## Included helpers

These helpers and instruction templates are called directly or through another
helper, scanned as source, and included beside the four entry scripts:

- `hex-meeting-quick-title.ps1`
- `meeting-audio.py`
- `meeting-translate-prompt.md`
- `meeting-notes-prompt.md`
- `hex-meeting-notes.ps1`
- `hex-meeting-render.py`
- `hex-meeting-publish.py`
- `hex-meeting-tasks.py`
- `hex-meeting-compare-delayed.ps1`
- `hex-meeting-compare-granola.py`

Private context now comes from the environment instead of source literals.
`hex-meeting-check.ps1` is mentioned by the notes helper as an optional manual
cross-check, not called by this pipeline. `hex-meeting-listen.py` is a caller of the
watcher, not a called helper. Neither is part of the payload.

Other prerequisites remain Python, PowerShell, uv, the configured model/CLI tools,
the existing Hex app/API and diarization models. Reading script source does not
qualify any of these runtime dependencies. Runtime configuration, meeting data,
transcripts, notes and audio were not opened by this lane.

The source scan report is stored in the owner review directory as
`pipeline-scripts-scan.md`; it contains finding types and source locations only.

## Private configuration migration

Before deploying the app update, configure the environment inherited by the app and
its watcher. This lane does not read or write private configuration. Use the prior
installation's values for `HEX_MEETING_PROJECT` (project key),
`HEX_MEETING_PROJECT_PATTERN` (case-insensitive regex for the prior project/company
keywords), `HEX_MEETING_PROJECT_CONTEXT_PATTERN` (the prior context/title regex),
`HEX_MEETING_KB_REPO` (private repository path),
`HEX_MEETING_OWNER_NAME`, `HEX_MEETING_COMPANY_NAME`, `HEX_MEETING_PRODUCT_NAME`,
and `HEX_TE_ASR_MODEL` (the existing Telugu model identifier).
`HEX_MEETING_OWNER_ALIASES` optionally supplies semicolon-separated owner aliases.
Do not put their private values in the repository or diagnostic logs.

The updater refuses replacement when required configuration is missing, preserving
the previous installation. With the original values supplied, project routing,
roster/KB reads and model selection follow the prior behavior. Example people and
company names in instruction templates use generic placeholders; their record
schema and instruction rules are retained. Runtime equivalence is unverified.

## App update installation

The app update callback embeds all 14 files, their SHA-256 hashes, and a small
stable watcher entrypoint. A version-bound pending marker retries through the
updater on app reopen. Developer builds and PCs without an existing pipeline are
not installed by this path. This lane never executes installation.

A first migration copies all 14 owner scripts byte-for-byte into a new
`.pipeline-backups/<id>` backup and an immutable, hash-verified legacy generation.
The legacy comparison helper is relocated to read prompts beside itself; the
separate owner backup remains byte-exact. It atomically publishes that legacy
selection before replacing the sole root
watcher with the pointer-only entrypoint. That entrypoint replacement is atomic
and also backed up. The other original root scripts and all existing backups stay
untouched. Before the bootstrap pointer exists, an interrupted migration still
leaves the original legacy entrypoint unchanged.

Updates write a fresh `.pipeline-generations/generation-<id>` folder. Each file is
flushed and SHA-256 checked; a manifest seal is written last and verified. No
selected generation is ever edited. Only after all bytes are verified does a
flushed temporary `.pipeline-current.json` atomically replace the pointer. Its
previous selection includes the old generation and manifest hash for rollback;
old and partially staged generations are retained. No per-file recovery journal
or in-memory rollback list is needed.

The stable entrypoint reads the pointer once, verifies the selected manifest and
all 14 hashes, then launches only that generation's watcher. Helpers resolve beside
their own script, including the comparison prompt reader, so a pointer switch
cannot move an in-flight job into another generation. There is no legacy-path
fallback in this managed runner. Existing scheduled callers keep their entrypoint
path; this lane does not modify their tasks or processes.

Source walkthrough: a stop during staging leaves the old pointer; a stop at the
atomic replacement exposes either the old sealed generation or the new sealed
generation. The original files are not progressively overwritten. Runtime kill,
power-loss/filesystem durability, bootstrap, rollback and concurrent-job behavior
remain unverified and require an authorized host. The reverse path is the saved
previous selection, atomically republished through the app updater. Private
configuration and model/CLI provisioning remain external.

## Transcription deadline

Only transcription uses the adaptive deadline. Preparation keeps its separate
1,800-second limit, and LLM calls keep their existing limits. The runner still
rejects deadlines outside 1..3,600 seconds.

The default is `min(3600, max(1800, ceil(300 + L * P * R * H)))` seconds:
`L` is the maximum normalized track duration in seconds, reported during existing
audio preparation without an extra read. Both tracks share the meeting timeline.
`P` is the pass count from the helper's actual transcription plan (currently two:
Telugu and Hindi), `R = 0.20` is ASR seconds per audio second per pass, and `H = 1.5`
supplies headroom. The 300 seconds allow for startup/reconnect overhead. A 64-minute
meeting with two passes receives 2,604 s. Missing/invalid duration or pass metadata
fails safely rather than silently using a mismatched helper's budget.

The measured two-pass time was 620.7 + 902.4 = 1,523.1 s, about 0.1983 s per audio
second per pass. This is one observation, not an upper-bound guarantee. Assumptions:
comparable speech density, segment count, hardware and quiet-laptop load. Silence
can overestimate work; overlapping tracks and many short segments can increase it.
The 3,600-second cap can still time out long or contended runs. Language routing
and checkpoint/resume remain separate P2 work.

`-TranscriptionTimeoutSeconds` (or `HEX_TRANSCRIPTION_TIMEOUT_SECONDS`) overrides
only transcription; 0 selects the formula, otherwise 1..3,600 is accepted.
`-AsrSecondsPerAudioSecondPerPass` and `-AsrDeadlineHeadroom` tune the formula.
No meeting data or model was used to validate these settings in this lane.
