# Security follow-up contract

Decisions needed: none. Source: the independent read-only review at 030a513.

## Threat model

1. A removed or stolen device must lose access to every change after removal.
2. A device must never rejoin a retired group, in either pairing direction.
3. Preview transitions must preserve other devices' folder and relay edits.

## Purpose and success criteria

Address the review in order. Persist protected retired-room markers and refuse
those rooms at membership merge, pairing adoption and session startup. Test
restart persistence and wrong-direction pairing. Complete distributed retirement
only if survivor notification, offline catch-up and old-history isolation can
all be proved. Otherwise hide Remove and refuse it in the backend, with the
explicit message that removal is unavailable. No partial revocation claim.

Disabling preview may safely refuse changed folder sources but must never advise
restoring older bytes over another device's work. Re-enable imports only folder
field deltas since its prior snapshot, or safely refuses an ambiguous transition.
Tests cover peer journals arriving while enabled, existing-task folder edits on
re-enable, relay restores and unchanged original bytes.

Worker changes: optional allowlist, bounded pending pairing claims per principal
and IP, and a lower connection byte budget. Test open default, restricted access,
caps and refusal without room allocation where possible. Bound unreasonable
capability requirements; exclude headless qualification hooks from normal release
builds. Resolve the Low worker containment finding if its implementation can be
verified here, otherwise record it as a release-blocking deferral.

Keep existing shared UI styles. Remove the sticky Sync-title overlap, show
unavailable revocation honestly, and render the affected states offscreen in
both themes. Fold in obvious hardening and give each remaining item a reason.

## Constraints and edge cases

Existing group rotations have no retirement receipt. Preserve old user content;
do not infer that an old copy is safe to import. Refuse retired host and guest
paths, membership unions, restart and stale settings. Concurrent folder arrivals,
deleted/restored tasks, idempotent retries and corrupted backups must fail safely.
Allowlist changes must not leak identifiers into logs. Tests use synthetic roots.

No deploy, login, push, version bump or owner-screen windows. Wait for existing
Cargo locks without killing processes. Run every requested gate from this
worktree, with GNU first on PATH and the worktree's own target. Stop near 75
minutes at a clean commit and report incomplete checks explicitly.

Out of scope: hosted qualification, physical-device results, an encrypted relay
backlog, installer signing and a substitute for independent re-review.

## Findings and evidence

All execution results here are local, rung 4. No finding is dismissed as not
reproducible. The default-off preview does not provide device revocation.

| Review item | Result | Commit and regression evidence |
| --- | --- | --- |
| H1, old-group re-pairing undoes removal | Fixed local guard | 6465d81, 5183f56. `retired_group_pairing_is_refused_whichever_device_hosts`; `retired_groups_refuse_old_membership_and_loading_after_restart`. Protected retirement records block membership merge and group loading. Startup retires inactive records left by older previews. |
| H2, survivors keep sharing with removed peer | Deferred; safe fallback applied | 6465d81. `three_peers_refuse_unavailable_removal_without_claiming_revocation` proves the backend refuses removal and leaves authority unchanged. The UI has no Remove action. Signed retirement, acknowledgments, offline delivery and old-history isolation are still required. |
| M1, self-hosted relay abuse | Fixed bounded controls | 330dc83. Worker tests `allowlist refuses unlisted genesis without persisting a sync room`, `pair-owner allowlist and three pending slots are enforced across rooms`, `IP pairing cap refuses a ninth pending room before room allocation`, and `connection byte budget refuses more than four MiB per second`. Distributed abuse and edge rules still need hosted qualification. |
| M2, unsafe backup-restore advice when disabling | Fixed | 6465d81. `peer_folder_edits_refuse_disable_without_overwriting_or_restore_advice` copies a second owner's journals into the first root, checks safe refusal and proves newer bytes unchanged. Shared-folder checks use the same safe advice. |
| M3, re-enable misses edits or replays deletions | Fixed for snapshots made by this build | 6465d81. `reenable_imports_existing_task_field_deltas_and_preserves_relay_fields`, `unchanged_folder_deletion_does_not_delete_a_relay_restore`, `a_new_folder_deletion_is_imported_once_on_reenable`. Older receipts without a baseline safely refuse changed imports. |
| L1, unbounded capabilities poison durable policy | Fixed for incoming requirements | 6465d81. `unbounded_remote_requirements_do_not_poison_the_durable_policy` checks both version dimensions and reopening. Valid next-version requirements still retain downgrade protection. Removal remains unavailable even when compatible. |
| L2, parser keeps the user's full token | Deferred, partial hardening | 6465d81 adds job-object UI restrictions and validates them inside parser workers. Existing memory, process-count and handle-inheritance limits stay. Restricted-token/Low-integrity or AppContainer execution needs Windows token, pipe, loader and filesystem-access tests before claiming containment. |
| L3, qualification driver ships in production | Fixed for normal optimized builds | 6465d81. Headless driver, crash pauses and capability impersonation compile only in development builds or with the explicit `relay-qualification` feature. `normal_release_rejects_headless_driver` checks an ordinary optimized binary exits before opening a replica and emits no output. The build/test result is recorded in artifacts. |
| Sync title overlaps scrolled content | Fixed | 5183f56. Sync's title scrolls with its content; notices stay outside the viewport. `check-relay5-renders.py` checks the title's scroll ancestry. Light/dark images cover the affected states. |

`5183f56` also fixes a race exposed by the unchanged mixed-version test. Device
list reads now share the mode lock with the authority/retirement handoff. Startup
finishes retirement if a crash occurs after publishing the new authority.

## Hardening suggestions

| Suggestion | Result and reason |
| --- | --- |
| 1. Shoulder-surfed code and spoofed device name | Added the requested warning about errors or absent words on the other screen. A public-key fingerprint UI is deferred because it needs comparison copy and accessibility review alongside the existing phrase. |
| 2. Replica location in folder share | Fixed. `preview_excludes_local_replica_before_existing_ignore_rules` proves `/local` is inserted before existing rules and remains idempotent. Existing rules are retained; keys and retained replicas stay excluded after disabling. |
| 3. Relay-triggered reconnects | Duplicate pending probes no longer replace a fresh challenge, covered by `c2_c10_replayed_initiation_keeps_live_session_and_bad_peer_is_isolated`. Replay/decrypt failures still reconnect. Changing that recovery policy needs separate nonce/reassembly tests; the relay can already deny availability. |
| 4. Enrollment before bundle receipt | Deferred. Moving the write alone breaks the tested unknown-receipt recovery path. Grant delivery, durable guest receipt and interrupted recovery need a protocol change together. |
| 5. DPAPI scope | Fixed documentation. The self-host guide states that another Windows account cannot unwrap protected keys. |
| 6. Retired key material | Active protected group records are deleted after the durable retirement marker. Secure erasure of SQLite/WAL pages and backups is deferred. Logical deletion cannot establish that erasure. The marker, rather than deletion, enforces refusal. |
| 7. Confirmation timeout margin | Fixed. Overall attempts allow 150 seconds, while app confirmation remains 90 seconds. Pairing regression tests retain expiry and refusal coverage. |

## Limits and verification receipt

Previously removed peers may still share their old group with each other. This
build cannot revoke that access. Do not treat an earlier Remove action, an
address change or the abuse allowlist as revocation. Safe removal stays blocked
until distributed retirement and old-history isolation pass independent review.

The earlier slice 5 revocation claim is withdrawn in `sync-relay-release-gates.md`.
Two physical devices, an owner-account hosted relay and independent re-review
remain release gates. The restricted-token sandbox is also unfinished.

Authoritative commands and results are in `artifacts/relay2-security-final-gates/`.
Offscreen PNGs and element trees are in `artifacts/relay-security-ui/`.
The normal optimized build and driver-refusal proof are in
`artifacts/relay-security-release-*.json` and matching text logs. A combined
artifact index records the precise results after verification completes.
