# Local slice 5 qualification

The independent review at 030a513 supersedes the revocation claim below. See
`sync-relay-security-fixes.md` for the fixes and remaining limits. Remove is now
unavailable in the UI and backend. This earlier receipt is historical evidence.

Decisions needed: none. The pre-test threat model and contract are at the top of
`sync-relay-integration.md`. Evidence here is rung 4: executed locally. It does
not establish physical-device, hosted-service or independent-review results.

## What ran

Two normal optimized release builds retain app version 0.0.1-dev.13. One declares
capability 1; the qualification feature `relay-capability-v2` declares capability
2. The real paired-process test checks the compiled declarations, confirms both
binaries have debug assertions disabled, refuses old writers, restarts after a
downgrade, then upgrades both replicas and resumes delivery without losing data.
This tests the current schema with different required capabilities. It does not
claim support for an unknown future data format or validate an installer.

Migration recovery kills a synthetic headless process after verified backup,
durable manual-task import and shared-import boundaries. Each restart produces
one stable manual task; disabling preview restores the exact original journal
bytes, and re-enabling does not duplicate it. This is process-crash evidence,
not power-loss, disk-full or physical storage fault evidence.

The exchange test pauses the local relay, waits until it has observed a large
encrypted sync frame, kills the sender, resumes forwarding and restarts it.
All acknowledged edits survive and reach the peer. The peer can then edit back.
Qualification controls exist only in the numeric-loopback test harness. A Worker
test verifies the production entry point refuses those routes.

Adversarial tests use the real encrypted-session handler and SQLite store.
Replayed, truncated, oversized, wrong-group and previous-session frames leave
accepted heads unchanged and enqueue no mutations. A fresh authenticated session
works after each refusal. Worker tests separately refuse oversized binary input.
This is protocol-level evidence, not an independent penetration test.

The earlier three-process test covered the remover's new group only. It did
not prove that other survivors stop sharing with the removed device or prevent
old-group edits from entering a new group. That revocation claim is withdrawn.
The crypto test still covers old-key refusal for a genuinely distinct group;
that primitive alone does not establish a safe removal workflow.

## UI and artifacts

The shared Sync notice stays above scrolling content, with a 16 px warning or
cloud-off icon. Advanced retains the detail and requested shorter help. Its
per-device badges and loose zero count are removed. No active global Sync status
slot was found in navigation, so no new dot was introduced.

Ten 1280x800 light/dark PNGs cover preview, operational, unreachable, wrong
address and update required. Automated tree checks verify banner position, icon
size, Advanced detail, help and badges. Artifacts live in this worktree:

- `artifacts/relay5-ui/`: PNGs, matching element trees and render index.
- `artifacts/relay5-release-builds/`: optimized binaries, commands and hashes.
- `artifacts/relay5-release-proof/`: compiled-binary pairing and upgrade proof.
- `artifacts/relay2-slice5-final-gates/`: authoritative full gate logs.
- `artifacts/relay5-evidence.json`: combined evidence and PNG hashes.

Artifacts contain synthetic data and are not committed. Codes and keys are not
included in logs or the render set. There was no deployment, login, push, version
change or visible app window.

## Release gates

Fmt, both strict clippy commands, the default workspace test suite, MCP smoke,
24 Bun tests and relay typecheck passed. Both optimized release builds and their
compiled-binary upgrade proof passed. The ASR-enabled workspace test command
compiled for 11 minutes and began running, then was stopped at the requested
time boundary. It is incomplete, not a pass. No test or gate was weakened.
The standalone `cargo build --locked` waited for that build lock and was stopped
while recompiling at the time limit. The earlier workspace bins/examples debug
build and both optimized release builds passed, but do not replace this gate.
An earlier Memory undo test encountered Windows file-sharing error 32; its
unmodified focused retry and the subsequent full default suite passed.

| Gate | Status | Evidence or remaining work |
| --- | --- | --- |
| Requested Sync UI fixes and ten renders | done | Shared notice, tree checks and offscreen PNGs. |
| Fmt, both strict clippy variants, default tests, MCP, relay tests/typecheck | done locally | Exit 0; 24 Bun tests; see gate logs. |
| ASR-enabled workspace tests | needs owner | Local command compiled and began tests; rerun to completion after time-limited stop. |
| Standalone locked debug build | needs owner | Rerun locally after time-limited stop; earlier debug and optimized builds passed. |
| Compiled release upgrade and downgrade | done locally | Real capability 1 and 2 optimized app processes, confirmed pairing. |
| Migration crash and rollback | done locally | Three durable boundaries, stable task identity, original bytes unchanged. |
| Interrupted exchange and restart | done locally | Relay-observed frame, killed sender, durable recovery and convergence. |
| Adversarial frames | done locally | Real protocol handler/store plus Worker boundary tests. |
| Three-peer removal and rotation | needs owner | Claim withdrawn after review; removal disabled pending distributed retirement and history isolation. |
| Two real devices | needs owner | Run the release matrix on distinct physical devices. |
| Owner-account hosted relay | needs owner | Deploy and qualify on the owner's account; include quotas and operations. |
| Independent security review | needs owner | Review received and blocking findings recorded; fixes need independent re-review. |

Release remains blocked on the three owner gates. Installer and signing evidence,
power-loss/storage faults, and any encrypted offline backlog are not established
by this local qualification. The relay still needs overlapping online replicas.
