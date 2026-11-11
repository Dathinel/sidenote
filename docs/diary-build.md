Decisions needed: none.

# Diary build contract

Purpose: implement the approved Diary redesign from `diary-redesign/docs/diary-redesign.md`. Diary shows a daily history beside current follow-ups across days. The real sidebar, Search, Annotations and top-right New meeting stay in place. Diary is a top-level page and shows no breadcrumb. The shared breadcrumb component belongs to the Settings lane and is not changed here. Shared Quiet ink buttons are consumed without changing their component.

## Success criteria

1. A flexible timeline and 320 px rail, one date heading, wrapping meeting titles, people from Meeting summary and a provenance-aware Untitled meeting fallback.
2. One inline outcomes disclosure per day, with source links, normalized decision counts, current pending proposals and known completion events. Summaries expand separately when long.
3. A Monday-first month calendar with meeting dots, Today, month movement, keyboard movement and selection of empty, future and unloaded dates.
4. Current attention across report days, deduplicated by stable source, with current triage and source status respected. Read failures remain visible. Compact copy never uses raw report briefs or internal identifiers.
5. Needs you, Due this week, Waiting on others and Memory proposals use real data, hide when empty and route to their source or a matching destination filter. Diary return context and unsaved edits are preserved.
6. Below 1100 px, This week folds the same rail into normal page flow. Desktop rail scrolling remains independent and keyboard reachable. Verify native offscreen renders at 1440, 1280 and 390 in both themes.

## Edge cases

- Empty Diary retains the calendar and a short invitation to create a meeting.
- A day with only an untitled meeting says Meeting summary title when available, otherwise Untitled meeting. Time appears once.
- Very long titles wrap; unknown people, duration and start time are not invented.
- More than 50 attention items retain deterministic ordering, a five-row preview and a full destination list.
- A future meeting and a quiet selected day remain navigable without changing current rail scope.
- Month/year boundaries, old dates beyond pagination, local timestamp conversion, unknown ownership/due dates, snoozed/handled findings, completed tasks, deleted sources, feature flags and failed reads retain explicit behavior.
- Decisions duplicated within a meeting count once. Accepted decision memories remain separate acceptance events. Completion history without a timestamp stays omitted.

## Out of scope

No model calls on page open, global shell redesign, button restyling, follow/watch model, complete activity journal, version bump, packaging, installation, push, visible windows or desktop input. Synthetic fixtures only. Work stays in `feat/diary-build` with this worktree's target folder and GNU toolchain first on PATH.

## Verification and delivery

One commit per slice with the requested co-author trailer, hooks enabled. Run formatting, both Clippy modes, both test modes, locked build, MCP smoke and the 49-key legacy settings check. Save PNGs and native element trees under `artifacts/diary-build/`. Record exact results, proposal differences, data limits and elapsed time here after verification.


## Data gaps and remaining limits

| Gap from the approved spec | Implemented minimum | Remaining limit |
| --- | --- | --- |
| Meeting dates and empty dates | In-memory meeting-day index, Monday-first month, selected date, Today and pagination-aware jumps. Undated meetings have a Date unknown group. | There is no calendar integration or inferred meeting date. |
| Title provenance | `Meeting.diary_title` is resolved during loading. Explicit user/calendar/call titles are retained. Known generated time/date placeholders use Meeting summary, then Untitled meeting. | The original producer of legacy time-only titles remains unknown. Legacy classification recognizes exact placeholder forms rather than arbitrary words in authored titles. |
| Daily display summary | Two sentences with a 280-character initial budget, Show more, and a conservative display boundary that withholds copy containing technical identifiers. Existing report summaries and Meeting summary context are reused. | Long first sentences may stop at the display budget until expanded. There is no new model call or human-authored display-summary field. Reports are already read for attention, so their summaries are cached in that refresh rather than fetched a second time. |
| Owned tasks, due dates and destination filter | `mine == Some(true)`, unfinished tasks, parsed dates, local Monday-to-Sunday boundary, stable task keys and a preserved due-date range at destination. | Unknown ownership and unparsed dates stay excluded. Existing producer-assigned default dates remain marked as such in the task source. |
| Waiting on others | Owned unfinished waiting tasks require explicit dependency or nonempty supplied reason. The source panel shows that dependency. | No person is inferred from free text. There is no follow/watch model for somebody else's task. |
| Cross-day attention | Read all available report dates, newest first, with effective durable triage and legacy date-scoped marks. Deduplicate guarded task sources and memory sources. Explicit resolutions and completed/retired/rejected sources suppress older rows. Rank high actionable findings, owned overdue work, due-week work and changed findings, then sort by due date and stable key. | There is no fixed scan horizon because unresolved legacy work can be older than a horizon. Large report libraries cost more on refresh. A missing finding in a newer report is not treated as resolution. Unverifiable source identity opens the original check. |
| Plain-language changes | Typed reasons use current due dates, owner gaps, conflicting decisions, or Review the latest update. Raw titles, explanations, briefs, revisions and subject identifiers are not compact rail copy. | Existing reports do not contain reliable field-level before/after values. The UI does not claim that a proposed due date was applied or invent an actual delta. |
| Routes and return context | Typed Task, Memory, Proposal, Meeting and original-check routes. Task fingerprints guard regenerated indexes, then stable identity is resolved again on open. Destination drafts are guarded. Tasks, Memory and check destinations provide Back to Diary; meeting citations retain the existing return path. | A deleted or unmapped source opens its original check or shows a source-unavailable message. Needs you See all opens task-backed attention in Tasks; Review all follow-ups includes memory and unmapped items. Source return restores the selected day anchor, rather than persisting an exact within-day pixel offset. |
| Decisions | Normalize decision/decided kinds and deduplicate text per source meeting. Open the source Meeting summary. Accepted decision memories are counted as acceptance events rather than a second decision. | Decisions without a typed Meeting summary point are not inferred from prose or Memory. |
| Task activity history | Meeting-associated tasks, manual added dates and known tick-journal completion dates. | Extracted task dates are meeting dates. Manual completion dates and older ticks without timestamps remain absent. No new historical journal is created. |
| Proposal queue and accepted claims | Current pending status, individual proposal routes, accepted-event dates and current claim text. Empty sections disappear. | Pending counts on older days can shrink after triage. Accepted rows show current claim text rather than an immutable acceptance snapshot. |

Diary has no breadcrumb. The existing sidebar, Annotations, sidebar Search and top-right New meeting remain. No shared button or breadcrumb component is modified.

## Verification record

Rung 4: all eight gates passed. All 62 Diary PNGs were re-read and checked against their recorded hashes and exact dimensions. All 32 required state, width and theme combinations are present, with no native tree warnings. The additional 24 renders cover routes, return, keyboard controls, a 101-day jump, a full queue above 50 items, feature flags, failed reads, HiDPI, breakpoint boundaries and independent short-window rail scrolling. The settings fixture produced 23 native trees, with all 49 legacy keys and nine pages reachable.

Rung 5: inspected the native desktop and narrow primary views, expanded outcomes, calendar selection, narrow week expansion, empty and long-title views, the old selected day, short rail scrolling and the folded failure state against the proposal. This evidence is synthetic and offscreen; it does not claim an installed-app or recording-hardware test.

| Gate | Result | Seconds |
| --- | --- | --- |
| cargo fmt --all --check | PASS | 3.73 |
| Clippy, workspace / locked / all targets / warnings denied | PASS | 43.77 |
| Clippy with asr-runtime, same strict flags | PASS | 30.88 |
| cargo test --workspace --locked | PASS | 61.42 |
| Tests with asr-runtime | PASS | 46.89 |
| cargo build --locked | PASS | 33.45 |
| MCP smoke against the worktree binary | PASS | 1.05 |
| Legacy settings reachability, all 49 keys | PASS | 0.20 |

Primary PNG paths, in review order:

- `artifacts/diary-build/diary-1440-dark.png`
- `artifacts/diary-build/diary-390-light.png`
- `artifacts/diary-build/diary-outcomes-1440-dark.png`
- `artifacts/diary-build/diary-calendar-1440-light.png`
- `artifacts/diary-build/diary-rail-1440-dark.png`
- `artifacts/diary-build/diary-week-open-390-light.png`
- `artifacts/diary-build/diary-empty-390-light.png`
- `artifacts/diary-build/diary-untitled-390-light.png`
- `artifacts/diary-build/diary-long-390-light.png`
- `artifacts/diary-build/diary-future-1440-dark.png`
- `artifacts/diary-build/diary-stress-all-1440-light.png`
- `artifacts/diary-build/diary-history-1440-light.png`

`render-manifest.json` enumerates every main PNG and native tree. `interaction-manifest.json`, `legacy-settings-manifest.json`, `gates.json` and `verification.json` contain the remaining evidence under the same artifact folder.



## Differences from the proposal

- The native 248 px sidebar remains, including Annotations and sidebar Search. The proposal used a shorter 208 px sidebar and top Search. The wider sidebar and native shell borders leave less width for the timeline.
- Diary is a top-level page with no breadcrumb, following the owner's update. The existing top-right New meeting stays. It uses the shared 8 px Quiet ink button, not the proposal's pill. The isolated recording fixture leaves it disabled.
- Native shared typography, icons, sidebar controls and section spacing replace the HTML shell. At 390 px the existing sidebar toggle remains reachable and the shell abbreviates New meeting to New.
- Counts and summary wording come from the real synthetic store. They differ from the illustrative proposal totals. The outcome line also names accepted memory and known completions when present.
- The rail shows actual section counts and up to two pending proposal previews. The proposal used one aggregate proposal row. Mixed and unmapped findings have a full follow-up review list, alongside the filtered Tasks destination.
- Change metadata uses typed current facts and Review the latest update where there is no reliable field-level delta. No generic Changed badge, raw report brief, revision hash or decision code appears in Diary.

## Reproduction

Run from this worktree with the GNU bin directory first on PATH and `CARGO_TARGET_DIR` set to this worktree's `target`:

```powershell
python tools/verify_diary_build.py fmt clippy clippy-asr test test-asr build smoke
python tools/render_diary_build.py target/x86_64-pc-windows-gnu/debug/sidenote.exe
python tools/render_diary_build.py target/x86_64-pc-windows-gnu/debug/sidenote.exe --interactions
python tools/render_diary_build.py target/x86_64-pc-windows-gnu/debug/sidenote.exe --settings
python tools/verify_diary_build.py settings
python tools/render_diary_build.py target/x86_64-pc-windows-gnu/debug/sidenote.exe --audit
```

Every native process uses `--snap`, isolated data/meeting/peer roots and hidden process creation. Input scripts run only in the offscreen renderer. No visible window or desktop input is part of this verification.

Verification builds use one Cargo job, no incremental cache and no debug symbols after the drive ran out of space. Debug assertions and every requested test, target and lint remain enabled. No repository build profile or shared component is changed.

Additional verification includes native `--scale 2` rendering. This is a HiDPI capture, not a test of the separate Windows text-size setting. Recording hardware and real meeting data are not exercised.
