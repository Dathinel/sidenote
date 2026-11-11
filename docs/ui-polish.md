# UI polish

Decisions needed: none.

## Purpose
Apply the owner's Quiet ink button system through shared controls and make Memory All filtering directly accessible.

## Success criteria
- Shared 32 px buttons (28 px dense controls), 8 px corners, 12 px horizontal padding, medium labels and Lucide 16 px icons.
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
