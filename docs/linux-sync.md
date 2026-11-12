# Continuous server sync

Server and Hub own a relay session and a one-second ingestion loop without a window.
The service holds `local/service.lock`; an enabled replica also holds SQLite's
exclusive writer lock. Start the service with the step-2 unit and credential setup
in [linux-server.md](linux-server.md). Stop it before restoring a matched replica
and credential backup. A second owner exits instead of taking over its database.

## File transport and ingestion

The loop validates/reconciles transported journals and scans complete local/peer
meetings. `local/sync/index.json` records all source identities and an
opaque revision, not a second copy of meeting text. Incomplete or invalid inputs
stay in place for retry and raise an error count. Unchanged inputs do not advance
the ingestion generation. Index state survives restart; source meetings/journals
are never rewritten by the index. Remote meetings are discovered, not automatically
queued for model/notes processing in this step.

## Retained source revisions

Every source is retained as `(device, id, revision)`. Identical revisions coalesce
into one logical meeting count while inventory preserves every source. Divergent
revisions have `conflict=true` and increase the logical conflict count. Inspect all
source revisions and correct the authoritative source explicitly; ingestion never
chooses a winner or overwrites peer files. Task conflict resolution is unchanged.

A failed, missing or unreadable source has `pending=true` and keeps its last-good
revision across restart. Pending changes do not advance the content generation.
Recovery to the same content is not a second ingestion. This index has no meeting
deletion protocol: absence never silently deletes prior successful state.
