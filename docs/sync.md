# Sidenote on multiple Windows devices

This page covers folder sync. For the opt-in relay preview, see the
[self-host guide](self-host-relay.md). Relay preview keeps folder transport for
meeting source files; its shared app data uses the relay collection instead.

Syncthing moves files. Sidenote reads peer text and merges immutable state journals in
memory. The app does not install Syncthing or configure shares or scheduled tasks.

New installs use `%APPDATA%\sidenote` for state and `%APPDATA%\sidenote-peers`
for peers. Each falls back to its old `hex-meetings` or `hex-meetings-peers` folder
when the new folder does not exist. Existing shares stay in place. The examples
below show legacy installs; replace those roots with the selected Sidenote roots
only on new installs. See [rename-migration.md](internal/rename-migration.md).

## Folders and device ownership

Use one outgoing meeting share for each recording device and a separate state share:

| Source | Destination on another device | Direction |
| --- | --- | --- |
| desktop `%APPDATA%\hex\meetings` | `%APPDATA%\hex-meetings-peers\desktop` | Send only to receive only |
| laptop `%APPDATA%\hex\meetings` | `%APPDATA%\hex-meetings-peers\laptop` | Send only to receive only |
| each `%APPDATA%\hex-meetings` | each `%APPDATA%\hex-meetings` | Send and receive, filtered below |

Never receive another device's meetings into the local Hex meetings folder. Do not share
all of `%APPDATA%\hex`, which includes API discovery credentials. Peer meetings remain
read-only inputs. Each device runs Hex and the notes pipeline/listener for its own local
recordings. Peer arrivals never trigger the pipeline. Local meeting ids take precedence;
peers show `from <device>` in the list and detail. The current Rune UI has no audio controls.

The non-synced `device.json` contains `name`, `daily_check` and a random, stable installation
`id`. The app fills a missing id once; changing the display name does not change ownership.
Do not copy this file to another installation. Keep it when reinstalling onto the same data.

## Single writer state

Only `state/` is shared inside the state share. A change is published at:

```text
%APPDATA%\hex-meetings\state\<device-id>\<logical-store-hash>\<random-transaction-id>.json
```

Each installation writes only its own device directory. Files are immutable: the app writes
a temporary file, flushes it, then publishes with a rename that refuses an existing target.
It never replaces another device's file and never rewrites a merged snapshot. Local writer
locks live under `local/locks` and are excluded from sync; readers never lock a peer's files.

Each journal identifies its device and logical store, carries a BLAKE3 checksum, and holds
item changes and migration receipts. Reads combine all available device journals in memory:

- Higher item timestamp wins; equal times are resolved by device id. Transaction id is the
  final deterministic tie-break for equal versions from the same device.
- New edits advance past the observed clock and use at least the local wall clock. There is no
  rewrite in response to a remote edit, so receiving state does not generate echo changes.
- Ticks, messages, manual tasks, project names/aliases, task overrides, memory records,
  proposals and triage entries have separate item keys. Daily reports use one item per date.
- Removing an item appends a tombstone. Tombstones are retained permanently; older snapshots
  and delayed journals cannot restore an item they contain. Clearing history removes the
  messages observed by that operation; an independently arriving new question remains.
- Edits compare against the actual snapshot that was edited. They do not delete unrelated
  items that arrived while the operation was in progress.

The editor keeps its edit-start value and merges it with changes already visible when Save
runs. Different fields survive; if both changed one field, the current save wins. Publication
runs on a worker and tries at most three times if more state arrives. A busy-store error
keeps every input so Save can be retried. A meeting-task edit is rejected if its original
text no longer occupies that index after summary regeneration.

**Offline limit:** edits made on two devices while both are offline still resolve per whole
item when their journals arrive. An offline owner change and an offline due-date change to
the same task can therefore replace one another. Save-time merging cannot see a journal
that has not arrived. Field-level offline convergence is deferred to the live sync engine.
This version writes the existing `checked` representation, readable by the 16cc27a reader.

The app refreshes on focus and every 10 seconds while the window is visible. No refresh
runs while it is hidden or minimized. One worker compares file names, sizes and modified
times before parsing changes. Peer reads skip reparse points and non-regular files and use
size limits (4 MB journals, 16 MB summaries and the existing transcript limit). Errors skip
the affected entry and are counted. Each folder has an 8,192-entry limit and each logical store has a 256 MB total limit.
An incomplete read keeps that store's last good data. Each file is read at most once in a refresh. Results from before a local mutation are ignored.
The footer's timer shows when a refresh was last applied; it does not confirm peer delivery.

No compaction or journal deletion is performed. Keep every journal, including tombstones,
on every device. Removing old files manually can restore old state or lose history.
Tombstones hide removed content from the merged view; they do not securely erase old journal
bytes, existing backups or other devices' copies.

A malformed/incomplete peer journal is left in place, reported on refresh, and skipped until
it becomes readable. Healthy own state remains available. An unreadable own journal blocks
local mutations rather than treating it as empty. Checksums detect incomplete/corrupt files;
they are not authentication against a malicious device allowed to rewrite and re-sign data.

## Migration and backup

Migration runs at startup and on Save. Refresh reads stores without writing; incoming
changes are read on focus or the 10-second visible refresh. Migration reads
legacy per-meeting JSON, `all.json`, task overrides, projects, task settings, memory records,
legacy company memory, triage and daily reports. It never overwrites, deletes or renames a
legacy input. Existing supported `*.sync-conflict-*` copies are also read as legacy inputs.
This replaces the old in-place conflict merger and conflict-copy archival.

Before importing each distinct source revision, the app publishes its exact original bytes
to this non-synced backup path:

```text
backups\sync-migration\<device-id>\<source-fingerprint>\<original-relative-path>
```

The backup must succeed before the import is published. The same journal records the import
and its source fingerprint. A crash before publication leaves only a backup; a retry is safe.
A crash after publication finds the receipt and does not repeat the import. An unchanged
source is imported once per installation; a new source revision gets another backup/receipt.
Malformed legacy input is retained and blocks that store's import/edit with a diagnostic.

Legacy items seed the journal using their existing per-item timestamps when present, or zero.
A reserved higher clock epoch puts every post-migration edit after every seed, including seeds
with an incorrectly future-dated source timestamp.
Absent legacy items never imply deletion. New edits use later versions; later-arriving older
imports cannot overwrite their tombstones. Plain legacy company-memory documents are retained
by content identity and combined until the owner accepts the existing record migration.
The original `memory/company.md` remains untouched. Generated company text, migration previews,
accepted legacy-memory copies, KB snapshots and report display copies live under `local/`.

New manual task ids combine installation id and random identity. Numeric `n` fields are only
legacy metadata. Legacy tasks without ids get content-based identities plus an occurrence
number: different tasks with the same number are retained, as are repeated identical tasks
within one snapshot. Missing legacy identities are also namespaced by installation and input path, so independent
identical tasks/questions on different devices are retained. This can duplicate shared legacy
copies; the owner may resolve duplicates. Existing explicit identities still deduplicate.

All devices must use this layout and the state ignore template before resuming two-way sync.
Older app versions must not keep editing shared legacy files. The notes pipeline writes project suggestions only in each meeting's `tasks.json`.
Only user actions register project names; loads still import existing legacy names without rewriting that file.
Local inputs and backups remain available for recovery. Recovery means restoring a damaged
immutable journal from an intact replica/backup, or appending a deliberate new edit; do not
replace another device's journal with a merged snapshot.

## Meeting share `.stignore`

Copy [meetings.stignore](sync/meetings.stignore) to the root of every outgoing and receiving
meeting share. The exact file is:

```text
// Only known meeting text travels. New files and all audio stay on the recording device.
/*/*/*
!/*/manifest.json
!/*/english-summary.json
!/*/english-summary.md
!/*/english-transcript.md
!/*/transcript.md
!/*/transcript.ndjson
!/*/tasks.json
!/*/quick-title.json
!/*/notes.md
!/*/attendees.txt
!/*/granola-transcript.txt
*
```

The allow list includes only these exact names directly inside a meeting folder.
`transcript.ndjson` is text used by Hex; audio evidence, originals, exports, backups,
locks and every unknown filename stay local, including new audio formats.
`notes.md` holds the user's typed notes; `quick-title.json` supplies the early title.
The meeting directories remain traversable so Syncthing can find the allowed files.
Patterns follow the [Syncthing ignore syntax](https://docs.syncthing.net/users/ignoring.html).
Apply this template on both ends; this repository change does not deploy live copies.

## State share `.stignore`

Copy [state.stignore](sync/state.stignore) to `%APPDATA%\hex-meetings\.stignore` on every
device. The exact file is:

```text
// Local inputs, caches, settings and backups never travel between devices.
/backups
/local
/task-verify
/kb-snapshot
(?i)probe-*
(?i)*.lock
(?i)*.tmp
(?i)runner-state.json*
(?i)runner-state.sync-conflict-*
(?i)runners.json*
(?i)runners.sync-conflict-*
(?i)device.json*
(?i)device.sync-conflict-*
(?i).env
(?i).env.*
(?i)*.env
(?i)*.env.*
(?i)*secret*
(?i)*credential*
(?i)*token*
(?i)*password*
(?i)*.pem
(?i)*.key
(?i)*.pfx
(?i)*.p12
(?i)id_rsa*
(?i)id_ed25519*
// Device journals and format markers sync. Earlier exclusions still take precedence.
!/state
!/state/**
*
```

The final allow rules include immutable journals and device-owned `area-format.json` markers.
Legacy root files, `memory`, `checks`,
configuration, local projections and all backups stay local. Never ignore
`*.sync-conflict-*` globally: valid leftover journal copies are read idempotently without
being rewritten or moved. Supported legacy conflict copies are backed up and imported.
Syncthing does not sync `.stignore`, so apply the templates on every share endpoint.

## Ask safety and daily ownership

Every single-meeting and Ask All invocation uses the restricted read-only runner policy,
regardless of prompt membership, context size or whether peer folders have arrived yet.
This also covers a peer root that appears while an Ask is already running. Claude is
restricted to Read/Grep/Glob with MCP disabled; answer-only Claude stays answer-only. Codex
arguments are rebuilt with the read-only sandbox instead of trusting configured argument
order. OpenCode is refused because this app lacks verified read-only enforcement for it.
Runners start in local app state, never a peer folder. This rule also applies on devices
that currently have only local meetings.

Each running Ask retains its submitted message id. A failure removes only that question and
writes its tombstone; a question appended by another device is not selected for cleanup.

Choose one daily-check owner. Keep `daily_check: true` there and set it false on secondary
installations. Missing settings default to the hostname and true; the owner must opt out the
second device. For example, before the first daily invocation on the laptop:

```json
{"name":"laptop","daily_check":false}
```

The app adds the stable id. Disabled daily checks return quietly before a runner, KB refresh
or report is started, including scheduled `--daily-check`, `--force` and `--dry-run` calls.
The owner reads local and peer meetings and publishes reports/proposals to its journal.
Other devices can read them and edit their own ticks, tasks, memory and Ask history. Two
accidental daily owners cannot replace each other's files; the later report item wins in the
view. To transfer scheduling ownership, disable the old owner and enable the new one.

## Automatic settle history

`memory/settle-log/` is one append-only, device-local log. Each immutable intent is
flushed and published before its source-data write; a separate immutable outcome
records whether it landed. Startup reconciles pending intents. Undo and Redo append
new intents and never rewrite earlier history. The directory is outside the shared
`state/` tree and must stay excluded from sync in this slice.

Settle history needs to join sync in a later slice. That work must preserve operation
identity, causality, source deletions and the optimistic guards before allowing an
Undo from another device. Do not copy one device's history into another device's log.
