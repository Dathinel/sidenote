Decisions needed: none.

# Settings build contract

## Purpose
Implement the approved 119-row Settings audit and 49-key search mapping from the settings-ia proposal on the assigned settings-build branch. Preserve existing persistence and safety behavior while giving every control one clear home.

## Success criteria
- Group all eleven pages under You and meetings, This PC, and Data and maintenance.
- Identify Hex ownership once near each relevant page top and retain a small Hex mark and Hex prefix on each Hex group.
- Provide scroll-following section jump tabs for Hex and dictation and other pages with more than four groups, using the Meetings tab styling.
- Render readable setting names, values and per-app recording policies. Hide Windows-incompatible choices without overwriting saved values.
- Move privacy, retention, connections and diagnostics to their approved destinations. Keep all 49 legacy keys searchable and reachable.
- Use existing Quiet ink button variants without modifying the shared button component.
- Keep Updates in one card with installed version, successful check time, check action and inline update states.
- Pass formatting, both Clippy configurations, both workspace test configurations, build, MCP smoke and 49-key reachability.
- Capture every Settings page and required states offscreen at 1280 and 390 in both themes, and compare with proposal PNGs.

## Edge cases
Unknown Hex settings and choices remain visible with readable recovery text. Unknown app policies are preserved. Unsupported saved macOS values are explained and never rewritten automatically. Secret values remain masked. Keep audio continues writing both existing keys. Update installation remains blocked during recording, Ask, jobs, download and preparation. Failures retain the last successful check time. Search opens the correct disclosure and canonical destination, including editors and legacy aliases. Narrow layouts wrap without losing controls. Jump-tab state follows manual scrolling as well as clicks.

## Out of scope
No stored setting deletion, version bump, package, installation, push, real Hex setting mutation, new backend feature or shared button restyling. No visible windows or input on the user's screen. Work and Cargo output stay in this worktree.

## Evidence
The proposal is the approved design input. Implementation inspection is rung 2, executable gates are rung 4, and inspected offscreen application pixels are rung 5 for rendering only. No claim of live production configuration verification is made.

## Owner breadcrumb decision
Remove the Settings breadcrumb on every page. Keep page navigation available at narrow widths. Use one shared quiet-chevron component on Meetings, company details, notebook details, the person editor, Memory All and Tasks with a project context. It accepts exactly a parent and current title, uses 13 px muted parent text with hover underline, a 14 px Lucide chevron at 2.25 stroke, and an ink current title at weight 500. Both titles truncate. Remove the annotation capture breadcrumb. Top-level lists, Memory home and Diary have none.

## Implementation and verification

Source changes are committed as `a37e20e`, followed by the readable-value and offscreen-fixture correction in `a403ab1`. The contract was committed first as `c7c690b`. Persistence keys were retained. The dynamic per-app policy editor uses the installed Hex schema's `ask`, `always_record`, and `never_offer` transport values and preserves unrecognized entries and fields.

Final evidence is rung 4 for executable gates and rung 5 for offscreen application pixels and navigation. No live Hex setting was changed. Cargo used this worktree's target directory and configuration, with the requested GNU tools directory first on PATH.

| Gate | Result |
|---|---|
| `cargo fmt --all --check` | Pass |
| `cargo clippy --workspace --locked --all-targets -- -D warnings` | Pass |
| Clippy with `--features asr-runtime` | Pass |
| `cargo test --workspace --locked` | Pass |
| Workspace tests with `--features asr-runtime` | Pass |
| `cargo build --locked` | Pass |
| `python tools/mcp-smoke.py <worktree>/target/x86_64-pc-windows-gnu/debug/sidenote.exe` | Pass |
| Legacy settings reachability | All 49 keys, eleven pages and search reveal pass |
| Individual legacy-key searches | All 49 reach the approved page and a visible canonical destination |
| Render structure | Both themes and widths pass ownership, readable values, row bounds, jump selection and Updates checks |

The detailed command arguments, exit codes and durations are in `artifacts/settings-build/gates/results.json`. Render and reachability receipts are in `render-verification.txt` and `reachability.txt` in that folder.

## Render paths

1. `artifacts/settings-build/updates-1280-dark.png`
2. `artifacts/settings-build/hex-dictation-1280-dark.png`
3. `artifacts/settings-build/meetings-recording-1280-dark.png`

`artifacts/settings-build/render-manifest.json` lists 105 Settings captures. They cover all eleven pages, Meetings and Models tabs, expanded disclosures, Updates idle/available/checking, Hex scroll state, the team editor and search, at widths 1280 and 390 in light and dark themes. `search-render-manifest.json` lists 49 additional key-navigation captures. The 44 breadcrumb captures cover all six drill-downs, all six parent actions, long titles and all named top-level exclusions. Their manifest is in `artifacts/settings-build/breadcrumbs/`.

Every Settings capture was compared with its matching proposal PNG where one exists. The same comparison file enumerates all 49 search and 44 breadcrumb proofs and identifies their missing proposal baselines. See `artifacts/settings-build/proposal-comparison.md` for the per-image differences, `proposal-comparison.json` for dimensions and overlap metrics, and `index.html` for the gallery.

## Differences from the proposal

- The owner decisions remove Settings breadcrumbs, move the single Hex ownership sentence to the page top, and add scroll-following section tabs on Hex and dictation and Meetings Recording.
- Native Rune field, switch and text geometry is retained. The shared Quiet ink button implementation is unchanged. Native switches keep their existing on-state color.
- The Hex model group lists every model returned by Hex. The proposal shows a compact selected-model example. Compatibility and connection are named disclosures.
- The available-update fixture opens release notes inside the single card. The proposal's main available state has release notes closed. Native actions wrap below status copy; all states retain the installed version.
- Values, people, model availability and timestamps come from isolated fixtures. The final update fixtures do not schedule a Hex refresh.
- No 390px proposal PNGs were supplied. Narrow native captures collapse the sidebar, wrap tabs and stack row controls. No exact checking, scrolled, search, Prompt library or team-editor PNG was supplied; the comparison table identifies the closest page reference or the missing baseline.

## Breadcrumb call sites

- `src/ui/mod.rs`: meeting detail and Memory All now use the shared quiet-chevron component. The former All meetings alternative is consolidated into the same parent/current form.
- `src/ui/companies.rs`: company detail and the person editor use People as the parent.
- `src/ui/annotations/notebooks.rs`: notebook detail uses Annotations as the parent.
- `src/ui/tasks.rs`: Tasks with a selected project context uses Tasks as the parent and the project name as current.
- `src/ui/hex/settings.rs`: the breadcrumb row is removed from every Settings page. Narrow page navigation and save feedback remain available.
- `src/ui/annotations.rs`: capture detail keeps a Back to annotations action and has no breadcrumb. Top-level Meetings, Tasks, Memory, Diary and Annotations do not add one.
- `src/ui/widgets.rs`: one shared component accepts exactly parent and current. It uses muted 13px parent text with hover underline, the 14px Lucide chevron at the shared 2.25 stroke, and ink current text at weight 500. Both labels truncate.

## Scope and timing

No push, version bump, package or installation was performed. All app captures and input events used the offscreen runner. No windows or input were sent to the user's screen.

Elapsed time was about 100 minutes, including the initial audit and source reading. Work was split between orientation, implementation, executable verification, render review and rework, with compiler waits overlapping edits. The initial dependency build took about 14 minutes. Repeated compile cycles after accessor and fixture mistakes were the largest avoidable cost.

## Round 2 contract
Implement only the final six review corrections. Keep Updates unchanged. Meetings retains Recording, Summaries and Tasks as its only tab row, with Hex summaries in Summaries. Only Hex and dictation uses the all-Hex ownership sentence and unmarked section headings; other Hex-bearing pages use the marked-settings sentence. Keep the vocabulary Hex-owned and explain both readers. Add capability-gated Use and Install model actions through Hex's existing protocol, with Install in Hex when direct installation is unavailable. On-state switches use the primary-button fill and ink tokens. Preserve all legacy keys and the shared button implementation. Complete the application edits, run formatting and both Clippy and test configurations, make one explicit cargo build, then run MCP smoke, 49-key reachability, and offscreen light/dark renders at 1280 and 390. Do not push, package, install or interact with the user's screen.

Vocabulary ownership evidence: `HexEngine::vocabulary_add/remove` writes through Hex; `core::notes::helper::Cli::glossary` reads the same list into Sidenote summary prompts. Hex's [vocabulary source documentation](https://github.com/anomalyco/hex/blob/main/docs/features/vocabulary.md) describes its transcription and rewriting readers. Copy names both applications without promising every speech model uses vocabulary.

Model command evidence: installed hidden `model --help`, `model use --help` and `model install --help` expose the actions. The [protocol source](https://github.com/anomalyco/hex/blob/main/src/control.rs) defines `model-use`, `model-install` and `window-show` with the model id and optional models pane. Verification never invokes a live mutation or opens Hex.

## Round 2 result

The six corrections are implemented. Meetings has one Recording / Summaries / Tasks row. Hex summaries and their search destinations now belong to Summaries. Mixed pages explain marked Hex settings; Hex and dictation has one ownership line and plain section headings. Names and terms remains Hex-owned and names both readers. Model rows offer Use or Install through the existing authenticated Hex commands, with capability-gated Install in Hex as the fallback. Selected remains plain text. Shared kit switches use the Quiet ink primary-button fill and ink tokens; the shared button component is unchanged.

All requested native images and element trees are in `artifacts/settings-build/round2/`, with the twelve requested images enumerated in `render-manifest.json` alongside seven supporting captures. The four scrolled Hex captures verify that the active jump tab follows its section. Updates at dark 1280 is pixel-identical to the approved prior capture.

Executable gates are rung 4 evidence: formatting, Clippy with and without asr-runtime, workspace tests with and without asr-runtime, the single cargo build, and MCP smoke passed. Logs and commands are in `artifacts/settings-build/round2/gates/results.json`. The new unit tests cover model action selection, the install fallback, ownership and Meetings routing, and both switch palettes. Runtime capture checks are rung 5 evidence for the fixture UI only. No live model selection, installation or window action was invoked.

Visual review found an existing long Summary row title clipping at 390px. Its text bounds are unchanged from the prior capture, apart from the new ownership line's vertical offset. This final round leaves that pre-existing layout outside the six requested changes.

The unchanged legacy reachability test passed all 49 keys across 55 wide light trees. Separate typed searches passed for all 49 keys and their visible approved destination. No stored setting was deleted. Round 2 took about 25 minutes, including source verification, edits, the complete gate suite and capture review.
