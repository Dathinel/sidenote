Decisions needed: none.

# Diary build contract

Purpose: implement the approved Diary redesign from `diary-redesign/docs/diary-redesign.md`. Diary shows a daily history beside current follow-ups across days. The real sidebar, Search, Annotations, breadcrumb and top-right New meeting stay in place. Shared Quiet ink buttons are consumed without changing their component.

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
