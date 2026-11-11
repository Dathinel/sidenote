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
