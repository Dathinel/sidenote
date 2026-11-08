# ADR 0002: Sync engine

Status: accepted with a gate, 2026-11-08. Journals v3 is the default; Automerge replaces it
only if it passes the pilot in [spikes/automerge-pilot-spec.md](spikes/automerge-pilot-spec.md).

## Context

Today's journals (`src/core/sync_state.rs`, about 840 lines) are already a CRDT: a map of
last-writer-wins registers. Each device writes immutable files under `state/<device>/`; readers
merge by `(at, device, transaction id)`; tombstones are permanent; legacy JSON imports once,
after a backup. Syncthing moves the files.

Gaps: no authorship (security finding S02); a peer can send `at = u64::MAX`; no compaction, so
history grows forever; a whole field is one register, so two edits to one text field keep only
one; the only transport is Syncthing, which strangers will not set up.

Our data is mostly records: ticks, task fields, memory status, project names, append-only chat.
Last-writer-wins per field fits all of them. Character-level text merge (Automerge's main
strength) matters only when two people edit the same note at once, and in A1 that happens only
in team workspaces, where the server can order edits.

## Decision

1. **Journals v3 is the engine** behind the ADR 0001 seam:
   - Keys become `[kind, id, field]` for new entities, so concurrent edits to different fields both survive.
   - Each journal is signed with the device's ed25519 key and checked against the device roster (ADR 0003).
   - Reject `at` more than 24 hours past the local clock epoch; cap 10,000 changes and 8 MiB per journal.
   - **Own-journal compaction:** a device may replace its own journals for a store with one
     snapshot holding its newest change per key, tombstones included. This is safe because the
     merge only ever uses each device's newest change per key. A late copy of an old file changes nothing.
2. **Engines emit opaque batches.** A `Transport` trait moves batches and never parses them:
   `FolderTransport` (Syncthing, free, no account, today's behavior) and `RelayTransport`
   (encrypted, paid, ADR 0003 and 0004).
3. **Run the Automerge pilot** (3 days, hosted CI only, synthetic data). Automerge replaces
   journals only if every gate holds:
   - All 12 conformance scenarios and the 2,000-case property test pass for the semantics table in the spec.
   - Disk size after compaction is at most 1.5 times journals.
   - Cold open p95 is at most 400 ms on `windows-latest` and at most 1.5 times journals.
   - Applying a 100-change batch has a p95 of at most 25 ms.
   - Peak extra memory is at most 100 MB, and the release binary grows by at most 4 MB.
   - Migration from journals round-trips every store exactly.

   Any miss means journals v3 stays, with the numbers recorded.

## Migration of today's journals

- **v2 to v3.** v2 stays readable forever. The first write after upgrade is v3. Each device records
  the hashes of v2 files it has already seen. Once a device enrolls a signing key, new unseen v2
  files claiming that device are rejected, so nobody can forge old unsigned history. When every
  roster device reports v3, each device compacts its own v2 files into one signed v3 snapshot.
  Only the owner deletes its own files. Backups go to `backups/sync-migration/` as today.
- **Journals to Automerge** (only if the pilot passes). Per store and device: materialize the
  merged view, write one Automerge document with actor = device id, and keep the journal reader
  for 90 days to import late arrivals as new changes. Original journals stay in `backups/`.

## Options rejected

- Automerge now, without a pilot: delete-versus-edit and history size are unmeasured for our data.
- Yrs or Loro: the same migration cost, and their strengths are also shared text.
- cr-sqlite: last release 2024-01.
- Electric or PowerSync: server-readable database sync; they do not fit E2E personal sync.
- Syncthing only: strangers will not configure shares, and there is no phone path.
- A hand-written text CRDT: if text merge becomes a need, use Automerge for that field type.

## Consequences

- Little code changes: the engine work is incremental hardening of tested code.
- Concurrent edits to one text field still keep one side. The UI shows "Edited on another device" when a newer remote version replaces a local edit.
- Signing and compaction close S02 and unbounded growth.
- The pilot costs 3 days, but either answer is settled with numbers.

## Revisit if

- Live co-editing of notes becomes a product feature. Then use Automerge for note bodies only, as a content type behind the seam.
- Journals miss the absolute thresholds (400 ms cold open, 100 MB memory) on the heavy-year corpus.

## First step

Roadmap lane S1: the engine-agnostic conformance suite and synthetic corpus, run against
`JournalEngine` in hosted CI. The pilot (S3) reuses it unchanged.
