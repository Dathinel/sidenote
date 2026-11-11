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
