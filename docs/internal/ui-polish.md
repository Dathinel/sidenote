Decisions needed: none.

# UI polish

## Purpose
Apply the owner's Quiet ink button system through shared controls and make Memory All filtering directly accessible.

## Success criteria
- Shared 32 px buttons (28 px dense controls), 8 px corners, 12 px horizontal padding, weight 500 labels and Lucide 16 px icons.
- Primary ink reverses in dark mode; secondary, ghost and destructive roles have hover, pressed, focus and disabled states.
- Actions use button roles; navigation stays blue.
- Memory All has a search field capped at 480 px, visible immediate filter menus, removable active filters, Clear all, local persistence, a working Memory breadcrumb, labelled unconfirmed metadata and collapsible groups.
- Offscreen synthetic fixtures render at 1280 and 390 in light and dark and compare with dev.15.
- All requested formatting, lint, test, build, MCP smoke and 49-entry settings reachability gates pass.
- Separate commits for buttons and Memory. No push or version change.

## Edge cases
Narrow windows wrap controls; empty filter results retain controls; stale saved projects do not crash; missing or invalid saved settings fall back to defaults; unconfirmed rows have explicit meaning; disabled buttons cannot activate; collapsed groups remain keyboard accessible.

## Out of scope
New data filters, data migrations, unrelated UI redesign, production state, packaging, installation and visible windows or screen input.

## Baseline
The owner explicitly designated release/dev15 as the comparison ref. Existing Memory supports type, state, project and a meeting scope. Person and date are not standalone filters. The metadata dot represents Not confirmed.

The designated baseline Sync page contains Devices and Sync folder, with no pairing card. The matrix renders the existing Sync page and records that limit rather than adding a pairing feature.

## Button action conversions

The shared action helper supplies grey ghost buttons, and recognises Remove, Delete and Forget labels as destructive. Navigation keeps the existing blue link helper.

| File | Actions moved from blue text to buttons |
| --- | --- |
| `src/ui/annotations/workflow.rs` | Cancel; Undo box |
| `src/ui/annotations/notebooks.rs` | New notebook; Cancel in picker; Edit details; Open capture; Remove capture; Add captures; Add a capture by its title; Cancel note generation; Edit note; Save changes |
| `src/ui/annotations.rs` | Add to notebook; Refresh / Refreshing |
| `src/ui/updates.rs` | Later in page and footer; What changed / Hide what changed in page and footer; Check now; Try again; help details; Restart to update |
| `src/ui/companies.rs` | Undo company change |
| `src/ui/hex/settings.rs` | Edit terms |
| `src/ui/hex.rs` | Dismiss engine notice; Save; Add vocabulary word |
| `src/ui/ask.rs` | Check again; Ask anything |
| `src/ui/ask_settings.rs` | Save model choice |
| `src/ui/models.rs` | Remove in confirmation and installed card; Cancel download; Download / Resume download; Remove partial download |
| `src/ui/widgets.rs` | Dismiss notice |
| `src/ui/mod.rs` | Undo / Redo toast action |
| `src/ui/check.rs` | Details / Hide details; Show / Hide story |
| `src/ui/diary.rs` | Show earlier days |
| `src/ui/tasks.rs` | Add task; Open meeting group; Use the meeting's date; select project; Create project; Add subtask; Show / Hide cleared tasks; Change cutoff / choose age |
| `src/ui/project.rs` | Select project; Create project |
| `src/ui/teams.rs` | Add team |
| `src/ui/people.rs` | Choose company |
| `src/ui/area.rs`, `src/ui/project_merge.rs` | Shared outlined action builders now delegate to Quiet ink rather than drawing blue labels |

## Differences from dev.15 renders

- New meeting and New annotation use 32 px rounded rectangles. Enabled primaries use flat dark ink in light mode and a light neutral fill in dark mode. Focus, hover, press and disabled examples appear in the button sheet.
- Settings Updates uses the same primary and ghost roles. Later, Check now and update details no longer look like navigation links.
- Sync renders the existing Devices and Sync folder sections. This baseline does not supply the requested pairing card.
- Notebook detail uses the shared primary for writing a note, ghost actions for editing and opening captures, and red outlined Remove controls. The narrow fixture closes its sidebar before capture.
- Recording controls and Ask suggestions use the shared button shape. Meeting More, transcript close and Ask send/stop use the shared icon button with 16 px Lucide marks.
- Memory All caps search at 480 px and shows Type, State and Project menus beside it, wrapping below search at 390 px. Saved choices show removable chips and Clear all. The Memory breadcrumb returns to the landing view. Group counts stay visible with collapse controls, and the unconfirmed dot is replaced by a labelled icon.
- Company overview inherits the shared controls and a ghost Undo action. Navigation links keep their blue colour.

The native matrix contains 36 current PNGs (nine scenes, two widths, two themes) and 32 comparable baseline PNGs. The baseline has no button-sheet scene. `tools/verify_memory_polish.py` also exercises the filter menus, persistence after restart, individual removal, Clear all, empty results, collapse and breadcrumb navigation through the offscreen runtime.

The full image path list is `artifacts/ui-polish/png-paths.md`, starting with the three requested filenames. Native render trees and binary metadata are in `artifacts/ui-polish/current/`. Pixel comparisons are in `artifacts/ui-polish/pixel-comparison.json`.

## Verification

| Gate | Result |
| --- | --- |
| `cargo fmt --all --check` | Pass |
| `cargo clippy --workspace --locked --all-targets -- -D warnings` | Pass |
| Clippy with `--features asr-runtime` | Pass |
| `cargo test --workspace --locked` | Pass |
| Tests with `--features asr-runtime` | Pass |
| `cargo build --locked` | Pass |
| `python tools/mcp-smoke.py target/x86_64-pc-windows-gnu/debug/sidenote.exe` | Pass |
| Legacy settings reachability | All 49 entries, nine pages and search reveal pass in 25 native render trees |
| UI polish matrix | 36 current render trees pass; 32 baseline comparisons |
| Memory interactions | Immediate selection, two active filters, restart persistence, collapse, removal, Clear all, empty results and breadcrumb pass |

Automated commands and runtime unit tests reached evidence rung 4. Native offscreen renders and scripted application interactions reached rung 5. No visible app session was opened.

The shared disabled-state test checks that pointer and keyboard input do not activate a disabled button, even if its handler is attached afterward, and that it activates after being enabled again.
