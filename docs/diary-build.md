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
| Routes and return context | Typed Task, Memory, Proposal, Meeting and original-check routes. Task fingerprints guard regenerated indexes, then stable identity is resolved again on open. Destination drafts are guarded. Tasks, Memory and check destinations provide Back to Diary; meeting citations retain the existing return path. | A deleted or unmapped source opens its original check or shows a source-unavailable message. Needs you See all opens Tasks for task-only attention, or the full follow-up review for mixed queues including memory and unmapped items. Source return restores the selected day anchor, rather than persisting an exact within-day pixel offset. |
| Decisions | Normalize decision/decided kinds and deduplicate text per source meeting. Open the source Meeting summary. Accepted decision memories are counted as acceptance events rather than a second decision. | Decisions without a typed Meeting summary point are not inferred from prose or Memory. |
| Task activity history | Meeting-associated tasks, manual added dates and known tick-journal completion dates. | Extracted task dates are meeting dates. Manual completion dates and older ticks without timestamps remain absent. No new historical journal is created. |
| Proposal queue and accepted claims | Current pending status, individual proposal routes, accepted-event dates and current claim text. Empty sections disappear. | Pending counts on older days can shrink after triage. Accepted rows show current claim text rather than an immutable acceptance snapshot. |

Diary has no breadcrumb. The existing sidebar, Annotations, sidebar Search and top-right New meeting remain. No shared button or breadcrumb component is modified.

## First-round verification record

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
- Diary is a top-level page with no breadcrumb, following the owner's update. The existing top-right New meeting stays. It uses the shared 8 px Quiet ink button, not the proposal's pill. The enabled capture fixture supplies synthetic Hex readiness. Disabled actions keep a tooltip and a visible reason.
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


## Round 2 contract

Purpose: address the final review of c5bafe5 while retaining the approved layout and shared controls.

Success criteria:

- Reserve each full source set in priority order: Needs you, Due this week, Waiting on others, Memory proposals. Counts use the remaining unique items, preview limits apply afterward, and empty sections disappear.
- Keep only the Needs you header See all. It reaches every item, including memory, unmapped sources and overflow beyond 50 items.
- Show the existing enabled Quiet ink primary button in review captures using an isolated synthetic Hex fixture. Keep the real disabled state with a visible reason and tooltip.
- Name the available daily report as 1 check-in, including Monday's otherwise empty outcome line. Expanding it opens actual report content.

Edge cases: items beyond the first five Needs you rows must not leak into a lower section; due and waiting tasks share stable identities; repeated store rows do not inflate counts; pending proposals already in Needs you disappear from the proposal section; distinct tasks may share the same title; missing Hex readiness remains disabled with an explanation. The existing empty Diary, untitled-only day, long title, 50+ queue and future meeting contracts still apply.

Out of scope: changing attention eligibility to populate empty sections, changing shared button or breadcrumb styling, recording, screen input, pushing, packaging, installation or version changes. One explicit locked build runs after the code edits and strict checks.

Round 2 evidence will be recorded under `artifacts/diary-build/round2/`, separately from the first-round captures.


The main review fixture retains 9 unique Needs you items and 3 pending Memory proposals. Due this week and Waiting on others are empty after priority assignment and are hidden. A separate fixture retains all four sections using a dismissed finding for a still-due task and an explicit waiting task without a due date. Its section counts and destination routes are checked separately. A proposal-attention fixture verifies that proposals in Needs you leave no duplicate proposal section.

Round 2 reproduction, using the same worktree and toolchain environment as above:

```powershell
python tools/verify_diary_build.py --out artifacts/diary-build/round2 fmt clippy clippy-asr test test-asr build smoke
python tools/render_diary_build.py target/x86_64-pc-windows-gnu/debug/sidenote.exe --out artifacts/diary-build/round2 --round2
python tools/render_diary_build.py target/x86_64-pc-windows-gnu/debug/sidenote.exe --out artifacts/diary-build/round2 --settings
python tools/verify_diary_build.py --out artifacts/diary-build/round2 settings
python tools/render_diary_build.py target/x86_64-pc-windows-gnu/debug/sidenote.exe --out artifacts/diary-build/round2 --round2 --audit
```


## Round 2 verification

Rung 4: formatting, both strict Clippy modes, both locked workspace test modes, the one locked build, MCP smoke and all 49 legacy settings keys passed. Settings used 23 fresh offscreen trees, including the search reveal. The new 64-task regression checks full-set priority, destination exclusions, repeated store rows and distinct tasks with the same title.

| Gate | Result | Seconds |
| --- | --- | --- |
| cargo fmt --all --check | PASS | 2.28 |
| Clippy, workspace / locked / all targets / warnings denied | PASS | 61.61 |
| Clippy with asr-runtime, same strict flags | PASS | 90.12 |
| cargo test --workspace --locked | PASS | 279.16 |
| Tests with asr-runtime | PASS | 262.88 |
| cargo build --locked, one explicit build | PASS | 84.91 |
| MCP smoke against the final worktree binary | PASS | 1.58 |
| Legacy settings reachability, all 49 keys | PASS | 0.23 |

Rung 5: inspected the actual native 1440 light and dark Diary and the 390 light open fold against the proposal PNGs. Also inspected Monday's expanded report and the disabled New meeting state with its visible reason. Enabled captures use the existing Quiet ink primary fill and ink tokens at full opacity. Diary has no breadcrumb, and the shared breadcrumb and button components are unchanged.

Differences from the proposal after this pass:

- The real sidebar, Annotations and sidebar Search remain. The native narrow shell uses its existing sidebar toggle and abbreviated New button.
- New meeting uses the shared 8 px Quiet ink primary button in both enabled themes.
- The main fixture's Due this week and Waiting on others sections disappear because every candidate is already in Needs you. Counts are 9 Needs you and 3 Memory proposals. Each pending proposal has its own preview, rather than one aggregate proposal row.
- Monday's report is named 1 check-in. Available check-ins are also named alongside the other daily outcomes. Synthetic task, decision, memory and completion totals continue to differ from the illustrative mockup.

No new data gap was introduced. The existing limits above still apply, including unknown ownership or due dates, missing historical completion timestamps and unavailable before/after changes. No data was invented to keep a rail section visible.

Requested PNGs, in review order:

- `artifacts/diary-build/round2/diary-1440-dark.png`
- `artifacts/diary-build/round2/diary-week-open-390-light.png`
- `artifacts/diary-build/round2/diary-1440-light.png`
- `artifacts/diary-build/round2/diary-rail-1440-dark.png`
- `artifacts/diary-build/round2/diary-rail-1440-light.png`


The final audit re-read all 17 round 2 PNGs and 23 fresh Settings PNGs, checked exact dimensions and file hashes, verified every native tree had no warnings, and matched all renders to the final executable. `round2-manifest.json`, `legacy-settings-manifest.json`, `gates.json` and `verification.json` contain the evidence. Main and rail images are identical full-page captures in each theme because both sections fit together after deduplication.

Elapsed time: about 25 minutes. Approximate work buckets: orientation 4 minutes, edits 5 minutes, verification 16 minutes, rework 0 minutes. Waiting for compilation is included in verification; there was no separate idle wait. Repeated status reads while compilation ran were the main avoidable overhead. There was one explicit build and no application-code rework after the gates began.
