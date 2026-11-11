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

Round 1 review: preserve the accepted storage, migration and four tabs. Group normalized duplicate decision text with expandable citations; simplify person rows to a company-cell picker and ellipsis editor; hide the company affiliation on company-scoped People; correct the recording fixture; remove the status placeholder; align Ask, suggestions, title and breadcrumb; and move All companies into the External section heading. Keep every original gate and regenerate the same 40 synthetic PNGs. Do not push, bump or pack.

Use only this worktree and its target folder. Put the GNU tools directory first on PATH. Keep version 0.0.1-dev.14. Do not push, pack, install or interact with the owner's screen. Reuse atomic writes and immutable journals. Preserve existing local data on corrupt input or failed publication. Never send email. Optional addresses are typed by the user and copying uses only the clipboard. Use building-2, Lucide stroke 2.25, list icons 18 px and inline icons 16 px. Titles have at most two lines. Product wording is Meeting summary.

## Edge cases

Blank names are invalid. Names and aliases compare after whitespace folding and lowercasing. Migration retries must be idempotent after interruption. Existing company records win duplicate matches. Merging the same record or an unknown id is invalid. Undo must refuse if later changes would be overwritten. Unknown company links remain visible as unassigned rather than inventing a record. Hidden old-only people remain hidden; source meetings remain searchable. Multiple people and aliases in one meeting count once. Decisions use explicit decided points, with their meeting citation. Empty meetings must never cause Ask to fall back to all meetings.

## Out of scope

No inferred companies, historical meeting rewriting, identity backfill, project lens redesign, archival, multiple domains, sending mail or relay implementation.

## Branch integration notes

The assigned base contains the People side controls and team aliases, but no company picker or optional email editor. This slice adds the missing company assignment UI and preserves side controls. TODO: integrate companies into the relay shared-data allowlist on the sync branch; this branch has local journal support but no relay shared-data allowlist.

## Storage and migration

`companies.json` is a logical store in the existing immutable device journal. Its catalog holds company records keyed by stable id and normalized person-name links containing `company_id` and optional typed email. A guarded publication changes the catalog and its Undo receipt together. Undo compares the current catalog with the receipt's result before restoring the prior catalog. Merge retains the source name as an alias, moves every explicit person link, and preserves both notes and domains, retaining a second domain in the notes.

Migration reads only explicit text from `local/ui.json` (`people_companies` and `people_emails`) and `people.json` (`people` rows with `name`, `company` and optional `email`). It never scans meeting names to create records. It keeps original bytes under `local/company-migration-backup/`, leaves the source files in place, and journals a migration marker only after backup succeeds. Migration runs before People can save preferences. Corrupt input stops migration without replacing it. Case and whitespace variants reuse the same company record; aliases participate in matching and duplicate-name validation.

The existing journal reader recognizes company stores and safely hashed company Ask thread keys. No relay or deployed sync configuration was changed. Company Ask builds context from the tab query's matching meetings and their tasks. An empty company yields empty meeting context. It has its own persisted thread and never falls back to the global thread.

## Verification

Evidence rung 4: all requested Cargo gates pass. Both workspace test configurations pass 262 tests. Seven company tests cover migration backup and retry, malformed input, normalized deduplication, rename, merge, Undo guards, restart persistence, private Ask threads, aliases, hidden old-only people, tab queries, empty Ask context and normalized decision grouping with ordered, distinct source meetings. Protocol smoke passes against this worktree's debug executable. The existing Settings reachability unit test also passes.

Evidence rung 5 for the offscreen native app: all 40 company cases render at 1280 by 800 and 390 by 800 in light and dark, using isolated synthetic fixtures. Their element trees have no render warnings, and the person controls, company tabs and rows pass horizontal-bound checks. Native scripts exercise External-header navigation, tab switching, decision-source navigation, rename and Undo across app reloads, merge and Undo with restored person links, global search, typed email, the company picker and narrow Settings navigation. The unmodified legacy-settings validator finds all 49 entries, all nine Settings pages and the Advanced search reveal in 23 native render trees, including the actual open Teams Advanced editor.

Reproduce from this worktree, with the GNU tools directory first on PATH and `CARGO_TARGET_DIR` set to this worktree's `target`:

```text
cargo fmt --all --check
cargo clippy --workspace --locked --all-targets -- -D warnings
cargo clippy --workspace --locked --all-targets --features asr-runtime -- -D warnings
cargo test --workspace --locked
cargo test --workspace --locked --features asr-runtime
cargo build --locked
python tools/mcp-smoke.py target/x86_64-pc-windows-gnu/debug/sidenote.exe
python tools/render_companies.py target/x86_64-pc-windows-gnu/debug/sidenote.exe
python tools/verify_companies.py target/x86_64-pc-windows-gnu/debug/sidenote.exe
python tools/render_companies.py target/x86_64-pc-windows-gnu/debug/sidenote.exe --settings
python tools/settings-reachability.py artifacts/companies
```

The render index records the executable SHA-256. The final 40 company images match the current debug executable. The ordered PNG list is `artifacts/companies/png-paths.txt`, the render index is `artifacts/companies/render-index.json`, interaction results are `artifacts/companies/interaction-results.json`, and individual gate logs and exit codes are under `artifacts/companies/gates/`.

No external Ask agent was invoked, no mail was sent, and the test did not write to the Windows clipboard. Actual clipboard delivery is not runtime-tested. Email copying uses the existing `Tx::copy` path.

## Differences from the proposed mockups

- The company breadcrumb returns to People / External, its implemented entry point. The existing application sidebar, including Annotations, is retained. The mockup's Projects / Companies breadcrumb and selected Projects navigation belong to the separate project-lens work. There is no Companies sidebar entry.
- Four shared native segmented tabs replace the mockup's underline tabs. The names are exactly Overview, People, Meetings and Decisions. Counts remain in the quiet header line instead of being repeated on the tabs.
- Ask is the real shared composer above the tabs, with runner, thinking and send/stop controls. The static mockup shows a search icon and arrow without these controls. Its left edge and width match the content column, and its suggestion controls remain aligned below it at both widths.
- Overview displays explicit company notes and a last-meeting source card. It does not fabricate the mockup's generated stand-of-play sentences. Until a status exists, no status content or placeholder is shown. The two commitment groups remain beside each other on desktop and stack on a narrow window.
- The native synthetic fixture has two linked people and three matching meetings, with two external open tasks and four internal open tasks. The static mockup has four meetings, two external follow-ups and one internal task. Native lists retain all matching rows and scroll rather than being trimmed to the mockup's counts.
- Shared person rows use 34-pixel initials, as required by the settled rule, rather than the mockup's placeholder user glyph. The Company cell in External opens the picker. Add email and editing live in a 16-pixel Lucide ellipsis menu, with Copy email available only for a supplied address. Company-scoped People omits the redundant Company column. The company group header itself opens the company, includes a 16-pixel chevron, and All companies sits at the right of the section heading.
- More opens the shared company editor; merge is an inline confirmation dialog using one shared bordered panel. These editing states are not drawn in the supplied company overview mockup.
- At 390 pixels, Settings navigation collapses behind the Settings pages control in the breadcrumb. This keeps External rows and their side controls inside the viewport. Opening navigation retains every Settings destination and search. Desktop Settings geometry remains the existing shared layout.
- Dark renders use the application's current dark tokens. The proposed company and External PNGs provide only light desktop references, so there is no proposed dark or narrow image for a pixel comparison.


## Round 1 review result

All eight UI corrections are implemented without changing the accepted stored data or migration. Decision grouping is a display projection: case, whitespace and a terminal period are normalized, the first meeting in the query order remains the main citation, and expansion exposes every distinct source meeting. Blank decision text yields the usual empty state. A regression test keeps genuinely different decisions separate and deduplicates repeated points within one meeting.

Native checks assert one fixture decision row with its two-earlier-meetings meta line, expansion and source navigation, absence of the repeated person links and company-scoped column, the External company-cell picker, ellipsis email/editor actions, All companies navigation, full-width Ask and shortcut alignment, and an identical title/breadcrumb origin. The same 40 PNG names, sizes, themes and empty-state cases are regenerated.

The grey New meeting control was the existing guarded recording action, not a company-specific style. The company render scene had not initialized its synthetic Hex recording capabilities. It now uses the same Hex fixture as the other page scenes, producing the shared primary blue action. Runtime availability checks remain intact.
