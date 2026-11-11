# Relay integration

Decisions needed: none. Slice 1 contract and evidence are retained below.
Slice 2 is authorized on dev.13 at d85f637; see docs/sync-relay-2.md for its contract.
Base: settings at 74150f0, version 0.0.1-dev.10. Core source: synccore-r1
at 461afde. The owner requires folder sync, independent offline field edits,
two-instance and two-device qualification, self-hosting and independent review.

## Threat model (three lines)

1. A relay or network attacker may observe metadata, delay, drop or replay traffic; signed encrypted peer sessions protect accepted content, not availability.
2. A paired member can read and change shared data; worker resource limits constrain hostile parsing but do not sandbox a compromised OS account.
3. Pairing-code theft, same-user malware and unlocked-device theft remain risks; confirmation and DPAPI protect enrollment and stored keys within those limits.

## Integration choice and data contract

Copy the tracked core and relay into this worktree, then use a workspace path
dependency. Do not cherry-pick the divergent app history. Keep the core tests,
patched spake2 and worker containment. Record provenance in the import commit.
The root lockfile and root spake2 patch become authoritative. Build the worker
beside sidenote.exe. Do not transfer SQLite, WAL, identities or protected groups
through Syncthing. Their directory is under ignored local/.

Recommend a full durable replica on each paired device. Automerge domains are
logical identifiers, not file paths. Map stable IDs and individual fields, never
one JSON string for a complete item. Keep concurrent alternatives and tombstones.
The imported core allowed tasks, memory, projects, diary and meeting:<id>.
Slice 3 adds explicit people and settings domains; teams share the people domain.

| App data | Planned relay representation | Folder sync / device boundary |
| --- | --- | --- |
| Manual tasks | tasks domain, stable task ID plus separate field registers; retain deletions and same-field alternatives | Slice 1 uses a separate preview collection. Existing journal tasks remain the rollback collection. |
| Meeting tasks and edits | immutable generated task identity plus field-level overrides, ticks, subtask IDs and membership tokens | Source tasks.json stays in allowlisted meeting folders until migration of generated data is qualified. |
| People | stable person IDs, names and aliases as separate values | Existing attendees and summary people remain read-only folder inputs initially. |
| Teams | stable team IDs, name field, member/alias sets | teams.json is currently device-local. Do not imply it already syncs. |
| Projects and areas | stable IDs, field registers, alias edges, explicit merge and undo operations | Keep journal writer until project invariants and recovery tests pass. |
| Memory | memory domain, record/proposal IDs, separate content, status and source fields | Legacy text requires one canonical import and source receipts. No silent duplicate bootstrap. |
| Settings marked You | shared identity/profile fields, language and vocabulary choices individually allowlisted | A UI section heading alone is not consent to sync. Helper paths, credentials and recording hardware stay local. |
| Meeting files | later meeting:<id> for typed notes and immutable generated metadata with hashes | Slice 1 keeps manifest, summary, transcript, tasks, notes, attendees and quick title on existing folder sync. Audio, exports, backups and unknown files stay local. |
| Device settings | none | Device ID/name, data paths, relay endpoint, Labs switch, local UI, GPU/model binaries, runner configuration and secrets stay per device. |

Slice 1 flattens each preview manual task's top-level fields into distinct root
registers keyed by [task ID, field]. A dedicated presence register is a tombstone.
There is no shared root-object bootstrap race. Nested subtask collections remain
whole-field registers in this slice. Concurrent same-field alternatives remain in
Automerge history; a conflict editor is a later release requirement. No clock is
used to choose a different-field winner.

## Two writers and migration

Slice 1 does not migrate user data. While enabled, only manual tasks in the
preview collection are shown and edited through relay storage. Meeting overrides
continue through journals. Folder manual tasks are retained byte-for-byte and
become visible again when preview is disabled. Preview changes do not write back
into folder journals. Never advertise this isolation as completed migration.
Reject a mutation that mixes preview task edits and folder edits in one save.
Project merge/undo touching preview tasks is blocked pending its own adapter.

For later migration: stop writers on all devices, finish folder catch-up, verify
the journals, and take a timestamped offline backup of state, legacy files and
device-local configuration. Verify a manifest of lengths and hashes. Snapshot
SQLite only with writers stopped or the SQLite backup API, including keys under
appropriate OS protection. Keep backups outside every synchronized folder.

Choose one seed device and migration epoch. Import its merged view once, using
existing stable IDs and a durable source-hash receipt per store. Other devices
receive causal history, not independent snapshot bootstraps. Record a per-domain
authority marker before enabling edits. An upgraded journal bridge imports each
immutable journal transaction once, with origin receipts; it emits field deltas,
never re-imports its own materialized projection. An old app must be read-only
for migrated domains. Mixed-version writable clients are unsupported and must
block cutover. This prevents a journal/relay echo loop and whole-item overwrites.

Rollback: stop relay writers everywhere, preserve the relay database and backup,
disable the authority marker and preview, restore the verified folder snapshot,
then resume matching folder clients. Old whole-item winners cannot establish lost
user intent; retain losing journal values for review. Edits since the backup require an explicit
reviewed export of relay field deltas. Never merge a stale snapshot automatically.
For slice 1 simply disable preview; it has not modified the folder task collection.

## Offline operation

The Worker has no offline content backlog. A lone online device commits locally;
it cannot deliver to an offline peer and must not claim delivery. On reconnect,
two simultaneously online full replicas exchange inventory and missing changes.
If their online times never overlap, relay delivery cannot occur. Folder sync may
still deliver its own domains, but does not deliver the preview database.

A later small encrypted backlog is worth evaluating for intermittent laptops.
It needs authenticated quotas, expiry, replay protection, acknowledgments,
membership/rotation semantics, recovery and a cost bound. It remains ciphertext
and is not a backup. Do not add it in this slice or weaken the overlap requirement.

## Pairing, keys, endpoint and hosting

Use the existing explicit host/join PAKE and compare-and-confirm flow. Both users
must confirm the displayed short authentication string. Do not auto-accept in
product UI. Codes and keys never go to logs, command lines or config JSON.
DPAPI-protected identities/groups stay in local/relay/replica.sqlite. The public
room locator may be stored in local/relay/settings.json. Do not copy identities
between real devices. Revocation, rotation and lost-device recovery gate release.

Slice 1 deliberately has no pairing UI. Its automated synthetic fixture enrolls
two temporary device identities in memory and writes each protected membership
through the core API before launching the two headless app processes. That is a
test setup, not evidence of product pairing. An unprovisioned preview fails closed
with a setup-required message; enabling it never creates a new public relay.

Labs / Relay sync (preview) is off by default. Settings / Sync / Advanced shows
the relay-address input only while enabled. Settings stay per device. Accept
only WSS origins on port 443, or explicitly local ws://127.0.0.1:<port> for this
preview. Reject credentials, paths, query, fragments and plaintext hostnames.
No default hosted endpoint. Apply endpoint changes on restart; disabling preview
stops its session and restores the folder view. Fixed error messages contain no
document bodies, keys or raw protocol payloads.

Cloudflare Worker self-host guide outline for a later slice:
1. Explain metadata exposure, full replicas, overlap and account quota costs.
2. Install bun, Node 22+ for the pinned Wrangler, and Wrangler; use the owner's Cloudflare account.
3. Configure the Worker name, Durable Object binding and SQLite migration.
4. Run local bun tests and wrangler dev on numeric loopback first.
5. Authenticate and deploy only with the owner's explicit deployment request.
6. Set the returned WSS origin on both devices, pair with confirmation, and test.
7. Document upgrades, backups, quota monitoring, outage diagnosis and removal.
This task performs no login or deployment. The copied relay README is historical
core documentation, not completed app self-host qualification.

## Acceptance and tests

Slice 1: run two headless instances of the real app binary with separate synthetic
data/meeting/peer roots and a numeric-loopback local Worker. Use the app TaskStore
mutation/read APIs, not a core-only merge harness. Assert A's task reaches B,
stop both sessions, edit owner on A and due on B offline, restart and assert both
fields on both devices. Assert restart durability, deletion, endpoint validation,
the disabled default and no preview publication into folder journals. Retain and
run all existing folder sync tests and the core's security/store/worker tests.
Use bun for relay tests. Check the helper is packaged and no windows are opened.

Later: stale-editor field deltas, concurrent text/subtasks/check/delete/restore,
same-field conflict recovery, concurrent create and bootstrap, malformed peer
data, wrong key/code and declined confirmation, relay restart and no-overlap,
disk-full/crash/backup restore, ID collision, migration retry and rollback,
mixed-version rejection, project merge/undo, clock skew and authority echo loops.
Release also requires two physical devices, a working owner-hosted Worker and
an independent security review. These are not replaced by local automation.

Run the requested fmt, locked clippy and workspace tests with and without
asr-runtime, build and MCP smoke. Report exact results and limitations. Do not
bump the version, push or deploy. Stop at the authorized slice and 90-minute limit.

## Ordered slices

1. **M: opt-in manual-task path.** Import core/tests/helper; isolated preview
   task collection, Labs switch, Advanced endpoint, two-instance offline proof.
2. **L: pairing and migration.** Confirmed host/join UI, verified backup and
   receipts, per-domain authority, journal bridge, rollback and conflict UI.
3. **L: remaining shared data.** Meeting task edits, people/teams, projects,
   memory and explicit You settings with domain-specific merge semantics.
4. **M: Worker self-hosting.** Reproducible guide, operational checks and owner
   deployment evidence. Decide separately whether to fund an encrypted backlog.
5. **L: release qualification.** Two-device matrix, adversarial/recovery tests,
   independent security review, revocation/rotation and packaging evidence.

Slices 1 and 2 are accepted. Slice 3 is authorized on feat/sync-relay-2 after
Round 1 UI acceptance (28fc734). Slice 3 is accepted at 742acee; slice 4 is authorized.

## Earlier evidence and limits

Slices 1 and 2 passed the recorded local gates. The initial provisioned pairing
fixture was replaced in slice 2 with the real confirmed pairing flow. See
`docs/sync-relay-2.md` for the accepted receipts. Local automated evidence is
rung 4 and does not replace physical-device or independent security qualification.

## Accepted slice 3

Slice 3 (742acee) shares meeting task edits/checks, stable subtasks, teams and
aliases, People classifications, projects/areas, Memory and your name. Selected
folder fields are backed up and frozen; source meeting files keep folder sync.
Helper/model choices, credentials, UI layout and People corpus eligibility stay
local. External Hex configuration has no adapter. Stable IDs, field alternatives,
tombstones, source receipts and existing project/Memory guards are retained.

Two real headless app processes proved confirmed pairing, offline field/member
merges, Memory text and accept/reject choices, stale project merge refusal,
owner-only undo, duplicate-free migration, unchanged folder bytes and durable
re-enable. Both feature gate variants, MCP, 21 Bun tests and typecheck passed.
The 44 offscreen frames and commands are indexed in artifacts/relay3-evidence.json.
This is rung 4 evidence. The pre-build slice 3 contract is retained in f9b56e6.
Mixed-version negotiation was not implemented or qualified in that slice.

## Slice 4 implementation contract

Purpose: prevent incompatible relay writers first, then make a local Worker
installation and its status understandable to a first-time self-hosting user.
The preview remains opt-in and source meeting files keep their folder transport.

Success criteria, in order:
1. Negotiate authenticated schema/capability requirements before document exchange.
   Persist the highest group requirement so reconnect, restart and downgrade cannot
   regain write access. Unsupported clients keep relay reads safe, refuse every
   relay mutation and show: "Another device runs a newer Sidenote. Update this one
   to keep syncing." Folder-only writes must still work. Prove two isolated app
   processes with different supported versions against numeric-loopback Wrangler.
2. Write docs/self-host-relay.md with prerequisites, pinned local commands, exact
   config values, Settings > Sync > Advanced setup, pairing/removal/rotation,
   cost/limit explanations and account-only steps. Execute every shell command
   shown locally, capturing output under artifacts. Account actions are prose,
   clearly unexecuted. Do not log protocol payloads, invitations, keys or secrets.
3. Check /health, distinguish an invalid endpoint from unreachable service, and
   show the configured address and last successful contact under Advanced.
   Render normal, unreachable and update-required states in both themes.
4. Run all existing gates: fmt, strict locked workspace clippy/tests with and
   without asr-runtime, locked build, MCP smoke, Bun tests and relay typecheck.

Edge cases: old/missing negotiation, future schema versus future feature version,
forged or stale announcements, offline edits after incompatibility, downgrade
across restarts, malformed health replies, timeouts, invalid addresses and no
previous contact. Keep successful-contact time per device and never call a queued
local edit delivered. Group removal must not retain another group's authority.

Out of scope: login, deployment, push, version bump, hosted services, screen
windows, backlog, selective rekey or slice 5. Physical devices, an owner-hosted
Worker and independent security review remain release gates. Stop at a clean
commit around 60 minutes and report any remaining work.
