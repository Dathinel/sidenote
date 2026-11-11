# Relay preview, slice 2

Decisions needed: none. This slice is based on dev.13 (d85f637), with
slice 1 (4993b20) merged first. Stop before remaining shared-data adapters.

## Scope and success criteria

Manual tasks only. Keep the preview off by default. Preserve dev.13 Settings,
Ask, People, Memory, annotations and meeting UI. No deployment, login, version
bump or push. Build and render in this worktree, with synthetic data only.

Success means two isolated app processes pair through the local Worker, require
both confirmations, migrate existing task identities once, retain independent
offline edits, expose concurrent same-field alternatives, replicate a recorded
resolution and return to unchanged folder bytes when disabled.

## Pairing and removal

Settings > This PC > Sync contains the Labs switch, pairing controls and paired
devices. The relay origin stays under Advanced and is device-local. There is no
hosted default. Use a WSS origin or numeric-loopback WS for local tests.

The existing PAKE invitation expires after 600 seconds and is single use. The
encrypted identity proof now signs each device's name with the transcript.
Both devices show the names and the same eight-word safety phrase, retaining
the core's 32-bit comparison value. Each side must confirm within the bounded
pairing attempt before a membership grant and task session are accepted.
Cancellation, wrong codes and expiry fail closed. Do not log invitation codes,
phrases, protocol payloads, keys or document bodies. Keys remain DPAPI protected
in the local replica. Codes are held in zeroizing buffers outside the text widget.

Remove stops this device's session and creates a fresh local group and key.
All remaining peers must pair again. The confirmation explains that limitation.
Removal cannot erase a replica already held by another device. It prevents new
edits on this device from reaching the removed group. An address-only settings
save cannot restore an older room or peer list.

## Migration and rollback

Enabling takes a verified byte backup of immutable journals containing manual
task changes, their tombstones and applicable legacy snapshots. Backups and a
hashed manifest live under local/relay/backups, outside folder synchronization.
Corrupt or incomplete journal views refuse migration. The local mode lock keeps
the app's task writers out during cutover.

Import keeps the existing stable task IDs. A content receipt in the Automerge
task document makes retries idempotent. Already-present IDs are not overwritten;
independent imports of identical IDs and values collapse to one visible task.
Different values remain alternatives. Deletions beat independent stale imports.
The first backup is kept when adopting another paired group.

While enabled, manual task mutations write only to the relay document. Folder
manual tasks are read-only. Meeting task overrides and other domains continue
through existing journals. A legacy snapshot is not published back into manual
journals while preview owns the type. A save spanning both authorities is refused.
There is deliberately no bidirectional journal bridge in this slice: replaying
whole-item snapshots would reintroduce the losing-writer problem.

Disabling compares the current manual journal set and raw bytes to the verified
backup before restoring the folder view. It does not rewrite the originals. The
relay database, memberships, receipts and history remain for a later enable.
If an old client or folder agent changed those journals, disabling refuses with
an actionable restore-backup message. It never deletes external changes to force
a match. Stop other manual-task writers during preview. Relay-era edits stay in
the relay collection; rollback does not export them into older journals.

## Conflicts and offline behavior

Each top-level field remains an Automerge register, with device, UTC time and
source metadata. Existing slice 1 registers still decode. The UI marks tasks
with multiple distinct values as "2 versions" and offers each device's value
and time in Sync and the task panel. A choice writes the selected value and a
replicated history record containing every alternative, the choosing device
and time. The record is data, never a console log. Concurrent later resolutions
can produce another chooser. Timestamps are descriptive, never winner clocks.

Different-field edits merge silently. Nested subtask collections remain whole
fields. Keep a full durable replica on every paired device. The relay still has
no content backlog: devices must overlap online to exchange changes. A lone
device can edit locally but cannot claim delivery.

## Validation contract and limits

The headless test runs two real app executables with separate synthetic data,
meeting and peer directories and a numeric-loopback Wrangler Worker. It uses
the product host/join service, with protected file handoff for test invitations
and confirmations, never provisioned memberships or stdout codes. It checks:

- Expired-code and wrong-code refusal, matching names/phrase, no tasks before both confirmations.
- Single-use replay refusal and unchanged membership.
- Shared-ID migration without duplicate tasks or identical-value conflicts.
- Durable offline owner/due merge and same-owner alternatives with times.
- Resolution convergence, replicated choice history and peer removal.
- Original folder byte equality through edits and rollback, then relay resume.

The local expiry-only harness moves invitation metadata past its deadline, then
the second real app attempts to join and verifies refusal without task delivery.
Only the local test config exposes that route; production handlers process all
normal traffic without logging. The core expiry test advances the host deadline
to an expired state and checks that no grant is emitted. The Worker test checks expired invitations refuse the
guest before upgrading its socket or sending a bundle. These tests avoid a
ten-minute wall-clock wait. Backup drift, legacy IDs, stale authority saves and
stale resurrection have separate regression tests.

Offscreen renders cover preview Sync, invitation, comparison, paired devices and
conflict chooser in both themes at 1280x800. The first four are synthetic UI
states. The conflict scene uses a real local Automerge conflict; a Rune script
clicks a version and checks that the chooser becomes a saved-choice record.
Run `python tools/check-relay-renders.py <frames> <chooser-proof>` after rendering
those scenes with `tools/render_ui.ps1`; only conflict proof scenes need Trees.
Never dump pairing invitation or confirmation accessibility trees into logs.

Run fmt, strict workspace clippy and locked workspace tests with and without
asr-runtime, locked build, MCP smoke, relay Bun tests and TypeScript checking.
Evidence is local execution (rung 4), not two-device or release qualification.
Physical-device tests, crash/disk-full recovery, adversarial protocol review,
mixed-version writable clients, selective multi-peer rekey, deployed self-host
operations and independent security review remain release gates. Follow the
original integration plan for later shared domains and Worker self-hosting.

## Local execution receipt

The merge commit is bf6ada3. Its full gate set passed before slice 2 edits.
Final fmt, both strict workspace clippy variants, both locked workspace test
variants, locked build, MCP smoke, 21 relay Bun tests and TypeScript checking
passed. The final Settings search-highlight correction also passed fmt and both
clippy variants. The MCP handshake still reports version 0.0.1-dev.13.

Local command receipts and full logs are in artifacts/relay2-merge-gates,
artifacts/relay2-verified-gates and artifacts/relay2-style-gates. Requested PNGs
are in artifacts/relay2-renders. The real offscreen conflict click and saved
choice are in artifacts/relay2-chooser-proof. These artifacts are ignored by Git.

One earlier ASR-feature workspace run timed out waiting for a pairing phase.
The isolated rerun, five additional real-pairing repetitions and both final
workspace variants passed. The timeout's cause was not reproduced. Secret-free
phase diagnostics are retained; no timeout, quota or security gate was weakened.

No physical device, Cloudflare deployment, login, push or version change was
performed. Slice 3 has not started.

## Round 1 UI review

The Labs description now contains one muted line. Shared card surfaces own their
borders; resting rows inherit the surface rather than painting over the rounded
edge. Relay and device rows use the pinned Lucide refresh-cw and laptop icons at
18 px, with the shared 2.25 stroke.

Pairing has one primary action and a link that reveals code entry. The editor
groups eight digits, preserves digit selection and disables Pair until complete;
Enter uses the same guard. Conflict choices use local human times, name the field
in the header and place Keep this beside each compact value row. Remove names the
device, explains re-pairing and offers Remove and Cancel.

The existing full gates remain required. Round 1 command receipts are in
artifacts/relay2-r1-gates. The 12 requested light/dark frames, including removal
confirmation, are in artifacts/relay2-r1-renders. Scripted reveal, removal/Cancel
and real conflict resolution are in artifacts/relay2-r1-proof. The render checker
also checks card alignment, transparent rows, the primary action and disabled
entry. Pairing codes and confirmation phrases are excluded from tree dumps.
