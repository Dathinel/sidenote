# Companies

Decisions needed: none.

## Purpose

Give explicitly named companies a stable local identity and one place to read their people, meetings, decisions and open work. The target is this companies worktree on feat/companies, based on release/dev14.

## Success criteria

- Company records contain a stable id, name, optional domain, optional notes and aliases.
- Explicit company text on people migrates once, with a retained backup and case/whitespace deduplication. No meeting inference or identity backfill.
- Person assignments use company ids. Rename and merge retain references and provide guarded Undo.
- People External company headers and global search open the company. There is no sidebar entry, as specified in the page plan.
- Exactly Overview, People, Meetings and Decisions. Use shared Settings cards, rows, palette and the existing Ask component with scoped context.
- Every tab has a plain empty state. Meeting and decision rows cite and open their source meeting.
- Migration, deduplication, merge, Undo and data queries have executable tests. Run all requested Cargo gates, protocol smoke and the 49-setting reachability test.
- Offscreen synthetic fixtures cover every tab, empty states, External headers and merge at 1280 and 390 in both themes. Record comparison with the supplied mockups.

## Constraints

Use only this worktree and its target folder. Put the GNU tools directory first on PATH. Keep version 0.0.1-dev.14. Do not push, pack, install or interact with the owner's screen. Reuse atomic writes and immutable journals. Preserve existing local data on corrupt input or failed publication. Never send email. Optional addresses are typed by the user and copying uses only the clipboard. Use building-2, Lucide stroke 2.25, list icons 18 px and inline icons 16 px. Titles have at most two lines. Product wording is Meeting summary.

## Edge cases

Blank names are invalid. Names and aliases compare after whitespace folding and lowercasing. Migration retries must be idempotent after interruption. Existing company records win duplicate matches. Merging the same record or an unknown id is invalid. Undo must refuse if later changes would be overwritten. Unknown company links remain visible as unassigned rather than inventing a record. Hidden old-only people remain hidden; source meetings remain searchable. Multiple people and aliases in one meeting count once. Decisions use explicit decided points, with their meeting citation. Empty meetings must never cause Ask to fall back to all meetings.

## Out of scope

No inferred companies, historical meeting rewriting, identity backfill, project lens redesign, archival, multiple domains, sending mail or relay implementation.

## Branch integration notes

The assigned base contains the People side controls and team aliases, but no company picker or optional email editor. This slice adds the missing company assignment UI and preserves side controls. TODO: integrate companies into the relay shared-data allowlist on the sync branch; this branch has local journal support but no relay shared-data allowlist.
