# Project matching and merge

New names match registered projects and aliases after case, whitespace, punctuation
and simple plural normalization. Multiple matches need an explicit existing-name
choice. Unknown names remain suggestions until Create is pressed. Daily Check can
only apply an existing project; it cannot silently register a new one.

Merge into is available on a project page, its menu and task project groups.
Choose a survivor, review every affected meeting, task, memory record and proposal,
then confirm. Rename to an existing project opens this same preview.

## Storage and failure boundary

The merge changes only `projects.json` device journal registers: the source alias
and its merge receipt publish together in one flushed immutable journal file.
Source meetings, notes, pipeline task tags, task overrides, ticks and memory
records stay unchanged. All views resolve aliases, including memory search and
proposals. A meeting assignment in the same project store works without tasks.

The existing machine-local project store lock serializes local writers. Confirmation
rescans local and peer meetings and compares the full project, task and memory
inventory with the preview. It checks again after backup. Incoming project journal
changes invalidate the operation before publication. Independent task and memory
edits keep their own registers; a late incoming item resolves through the alias.

Before publication, `local/project-merges/<UTC date and random id>/backup.json`
holds the prior project view and byte copies of every project input journal and
legacy project file. It is flushed, read back, parsed and checked. Its checksum
is stored in the merge receipt. A crash before journal publication leaves the old
view, possibly with an unused backup; a crash after publication leaves the new
view and its durable receipt. No multi-store rollback is required.

Undo restores the source's prior alias and merge register in one journal, preserving
unrelated later edits. It works after restart on the originating device. It refuses
a missing, changed or foreign-device backup and a subsequently changed source alias.
Concurrent reverse peer merges resolve cycles to one deterministic project without
dropping any items. They are not automatic merge suggestions.

## Isolated verification

No tests are run locally. `examples/project_merge_proof.rs` is a separate-process
synthetic proof driver, invoked by `tools/project_merge_proof.ps1`. It requires
explicit data, meetings and peer folders under
`E:/Personal-Projects/reviews/project-merge-proof`. It checks merge inventory,
source byte preservation, alias search and proposals, completed manual tasks,
meetings without tasks, peer writes, stale previews, backup checksum refusal,
crash after backup, retry and exact project-view restoration after restart.

The crash switch is debug-only and restricted to that proof folder. Backups are
retained locally without pruning. Whole project histories can make backups large.
Preview and backup run synchronously, so large libraries can pause the UI briefly.
