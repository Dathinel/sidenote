# Task spec: Automerge pilot (ADR 0002 gate)

Lane S3 in [ROADMAP.md](../ROADMAP.md). Budget: 3 days. Depends on F2 (Store API) and S1
(conformance suite). Run no tests on a laptop; hosted CI is the only test gate.

## Goal

Decide with numbers whether Automerge replaces journals v3 as the engine behind `Store`.
The default answer is no. The pilot can change it.

## Branch and files

- Branch `pilot-automerge`, worktree `.worktrees/pilot-automerge`.
- `src/core/store/automerge.rs`: `AutomergeEngine` implementing the `Store` engine trait, behind Cargo feature `automerge-pilot` (off by default).
- `tests/sync_conformance.rs`: already exists from S1; add the Automerge engine to the generic `suite::<E>()` list.
- `tests/support/corpus.rs`: already exists from S1; reuse.
- `.github/workflows/sync-pilot.yml`: runs on `windows-latest`, on push to `pilot-automerge` and on manual dispatch.
- `docs/architecture/spikes/pilot-thresholds.json`: the numbers below, read by the CI check.
- `docs/architecture/spikes/automerge-pilot-results.md`: the result, written at the end.

Pin `automerge` to an exact version in `Cargo.toml` and record it in the results.

## Expected semantics (both engines must match)

The adapter may model data however it likes (for example, delete as an `deleted_at` field
rather than an Automerge map delete), as long as results match this table.

| Case | Expected result |
| --- | --- |
| Two devices edit different fields of one entity | Both edits kept |
| Two devices edit the same field | Higher `(at, device, change id)` wins, on every device |
| Delete and a concurrent edit | Higher version wins; an older edit never revives a deleted item |
| Two devices append messages | All messages kept, ordered by `(created_at, id)` |
| Clear thread | Removes only messages the clearing device had seen |

## Scenarios (all must pass)

1. Concurrent offline edits to different fields of one task converge with both edits.
2. Concurrent edits to the same field: 3 devices, all 6 delivery orders, identical result.
3. A task toggled 1,000 times across 2 devices ends at the highest version.
4. 2 devices append 50 messages each offline; all 100 are present and ordered.
5. Delete versus concurrent edit follows the table, in both orders.
6. Clear history keeps a message that arrived concurrently.
7. Compaction: the view is identical after compaction, and a late pre-compaction batch changes nothing.
8. Device loss: wipe one device and rebuild from peers; the view is identical.
9. Corrupt input (truncated, flipped bit, wrong device, bad signature) is rejected and reported; other data stays readable; a corrupt own batch blocks writes.
10. Hostile input (`at = u64::MAX`, 1,000,000 changes, a 100 MiB batch) is rejected under caps with no out-of-memory.
11. Legacy import: synthetic v2 journals and legacy JSON import once, stay idempotent over 3 restarts, and write backups first.
12. Schema upgrade: v1 data reads as v2; v2 data read by v1 code returns the "newer schema" error.

**Property test:** `proptest` with 3 replicas, random ops (put field, delete, append, clear),
random delivery order and duplicate delivery. 2,000 cases: a fixed seed plus one random seed
printed in the log. Invariant: all replicas reach identical canonical JSON.

## Corpus ("heavy year", deterministic from seed 20261108)

500 meetings, 6,000 tasks, 25,000 tick toggles, 300 chat threads with 3,000 messages averaging
1.5 KB, 1,500 memory records, and 365 daily reports of 8 KB. They are written as about 40,000
batches from 2 devices. All text is generated; no real meeting data.

## Metrics (both engines, 20 runs each, written to `pilot-metrics.json` as a CI artifact)

| Id | Metric | Pass for Automerge |
| --- | --- | --- |
| M1 | Bytes on disk after compaction | at most 1.5 times journals |
| M2 | Cold open, materialize every store, p95 | at most 400 ms and at most 1.5 times journals |
| M3 | Apply one 100-change batch, p95 | at most 25 ms |
| M4 | Peak extra RSS during M2 | at most 100 MB |
| M5 | Release binary size increase | at most 4 MB |
| M6 | Bytes sent to bring a new device up to date | reported only |
| M7 | Migration round trip: canonical JSON equal for every store | exact |

Journals must also meet the absolute limits in M2 and M4. If they do not, compaction moves
earlier in the roadmap.

## Pass and fail

- **Pass:** every scenario and the property test are green for Automerge, and M1 to M5 and M7 meet the table. Then ADR 0002 flips to Automerge, and a migration lane is added.
- **Fail:** any scenario still fails after the adapter work in the 3 days, or any metric misses. Then journals v3 stays.
- **Either way:** write the numbers, versions and verdict to `automerge-pilot-results.md`, update ADR 0002's status line, and do not merge the engine code to main. The suite and corpus stay.

The CI job reads `pilot-thresholds.json` and fails when a threshold is missed. Never relax a
threshold to make the pilot pass.
