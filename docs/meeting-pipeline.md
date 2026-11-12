# Meeting pipeline scripts

Scope: the entry scripts and their called helpers in `scripts/meeting-pipeline` are versioned from the
installed source snapshot after a private-data scan. No pipeline was executed.
Import commits keep file boundaries where practical. Private defaults must be
supplied through the environment before deployment; processing logic is retained.

`hex-meeting-english.ps1` accepts `auto`, `other`, or the project key supplied in
`HEX_MEETING_PROJECT`. Set that environment variable to the existing private helper's
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

Private context now comes from the environment instead of source literals. `hex-meeting-check.ps1` is mentioned by the notes helper as an optional manual
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
keywords), `HEX_MEETING_KB_REPO` (private repository path),
`HEX_MEETING_OWNER_NAME`, `HEX_MEETING_COMPANY_NAME`, `HEX_MEETING_PRODUCT_NAME`,
and `HEX_TE_ASR_MODEL` (the existing Telugu model identifier).
`HEX_MEETING_OWNER_ALIASES` optionally supplies semicolon-separated owner aliases.
Do not put their private values in the repository or diagnostic logs.

The updater refuses replacement when required configuration is missing, preserving
the previous installation. With the original values supplied, project routing,
roster/KB reads and model selection follow the prior behavior. Example people and
company names in instruction templates use generic placeholders; their record
schema and instruction rules are retained. Runtime equivalence is unverified.
