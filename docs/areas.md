# Areas

An Area is an ongoing domain. A Project is an outcome that can finish. Projects
have one parent area, or stay unassigned until the owner chooses. There is one
project level. Unknown classifier labels remain suggestions.

The sidebar lists six active areas. Show all opens the area directory and its
inactive work. An area page starts with active projects and open area-only tasks.
Show all includes inactive projects and completed tasks. Search can find inactive
areas, their aliases and projects. Project pages and meeting chips show the area
and project together.

## Owner controls

Use Show all under Areas, then Create area or Review migration. The migration
lists every current project. Enter an area for each row. Equal area names reuse
one area. Keep as project preserves the project ID. Turn into area makes an
ongoing domain from a broad label. No owner mapping or private names ship here.

From a project page, Move into area keeps its ID and changes its parent. Turn
into area previews its tasks, meetings, manual tasks, memory records and memory
proposals. References default to area-only work. Pick an existing child project
for any reference that needs one. For migration, child choices use the other
rows assigned to the same area. A child cannot also be converted.

Every operation has a count preview and separate Confirm change button. Editing
a row or a conversion choice clears its preview. Area-only assignments are also
available from meeting and task project pickers. They use the same preview and
guarded publication. Area pages offer Rename area. Undo change is in the area
directory, including after restart.

## Storage and recovery

The existing `projects.json` device journal holds separate top-level registers
with the prefixes `area:`, `area-project:`, `area-link:`, `area-alias:` and
`area-operation:`. Each object gets its own register, so unrelated peer objects
do not share a whole-catalog replacement. Older Projects readers retain these
keys through their existing unknown-field map. Legacy project names, aliases,
meeting assignments, source files, overrides and memory files remain intact.
Older UI versions still show the original project labels.

Legacy project IDs are deterministic on first read. Separate `area-identity:`
seeds retain identity across renames. Seeds never publish default mutable project
metadata. Ordinary add and rename leave parent and conversion registers alone.
Move never changes identity. Conversion retains the
old identity with `converted_to`; its legacy aliases have explicit `kind: area`.
Per-reference links bind to original assignment values and task text. Later raw
assignment edits take precedence. Child links follow the child's stable ID if
it moves. Memory and proposals resolve links in their read projection; no source
memory record is edited by an area operation.

Before publication, `local/area-operations/<operation ID>/backup.json` records
the prior project view, versioned mapping, full preview and byte copies of the
project inputs. The backup is flushed, read back, parsed and checksummed. The
receipt pins the checksum, affected assignments, touched registers and child
project baselines. Local writers share the existing project lock. Full inventory
and journal versions are checked before publication, including after dependency
validation. A migration publishes one immutable journal and one receipt.

A crash before publication leaves the old view and possibly an unused backup.
A crash after publication leaves the new view and durable receipt. There is no
cross-store write to recover. Undo checks the originating device's backup and
current assignments, then publishes one journal restoring only touched registers.
It refuses changed backups, later moves, peer conflicts and dangling references.
It preserves unrelated later edits and records that the operation was undone.

Undo retires a newly created area instead of deleting it. Its name and stable ID
remain readable when a late peer reference arrives. Such work appears with a
conflict notice and a Restore area action. Known new dependencies block Undo.
Missing references get a `Missing area` entry without blocking the rest of the
catalog. Unknown fields survive writes. A newer or unreadable schema makes area
operations read-only while the available catalog stays readable.

Task overrides and manual tasks have one `assignment` field holding a project
choice and a revision. Choosing the same project again still records new intent
and supersedes an older area-only link. The raw project field stays for older
readers. Accepted memory proposals store the effective stable target in the new
record's own `assignment` field. These are whole fields for the parallel
field-level three-way merge; that stack is not included here.

The migration editor starts with one default for each project's references.
Area-only work turns the old project into an area alias. Exceptions are optional,
searchable and virtualized, as are the project rows. Review and Confirm stay
outside the scroll area. Changing a parent or conversion default clears invalid
child choices and requires a fresh review. Final-plan validation rejects a child
that is itself converted, in either row order.

The refresh worker builds tasks, chip lookups, labels, activity and reference
indexes. Meeting chips do no catalog rebuild or full task scan in the UI. Manual
task baselines are indexed once. The large render fixture has 14 projects and
800 tasks and writes a sibling proof receipt.

Activity, inventory, backup, apply and Undo run on workers. An open effective
task, a linked meeting in the past 30 days or a running linked Ask/router job
activates a project. An area is active when a project is active or it has open
area-only work. Activity does not infer that an outcome finished. The stored
status is retained independently from this computed navigation view.

## Proof and limits

No tests run locally. Build the `areas_proof` example offline, then invoke it with
`run --data-dir <fresh folder> --meetings-dir <folder>/meetings --peers-dir
<folder>/peers`. All three folders must be under
`E:/Personal-Projects/reviews/areas-proof`. It starts separate processes for move,
conversion, batch migration, restart Undo, stale previews, later moves, peer
edits, publication races, backup tampering and crashes before the commit point.
It checks source bytes after each scenario and writes `results.json`.

The `areas-worker-proof` render holds real workers before I/O and checks that
the UI returns, repeated operation clicks are ignored, worker thread IDs differ
and Undo restores the area view. It writes a sibling `.proof.json`.

Backups are local and retained without pruning. Preview scans all references,
so large histories can take time. Undo is deliberately conservative and may
refuse after unrelated alias edits or linked-child moves. Syncthing cannot make
two offline devices agree before they exchange journals. Simultaneous unseen
edits to the same register use the existing deterministic last-writer rule;
both immutable journals remain available. Classifier prompts, Group by, teams
and series are unchanged in this merge.
