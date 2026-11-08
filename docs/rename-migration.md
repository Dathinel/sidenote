# Sidenote rename migration

Decisions needed: none for this code change. The owner performs the live steps below at release.

The owner chose Sidenote on 2026-11-08. It remains a companion to Hex.
The repo folder and GitHub remote stay named hex-meetings until release.
This checklist records actions only. This lane does not change the installed app,
real app data, pipeline scripts, scheduled tasks, listener or MCP registrations.

## Data selection and compatibility

- New installs use `%APPDATA%\sidenote`. If that folder does not exist and
  `%APPDATA%\hex-meetings` exists, use the old folder in place. Do not move or copy it.
- Apply the same choice independently to `%APPDATA%\sidenote-peers` and
  `%APPDATA%\hex-meetings-peers`. Hex recordings remain `%APPDATA%\hex\meetings`.
- If both new and old roots exist, the new root wins. To stay on old data, pass
  `--data-dir` and, if needed, `--peers-dir` explicitly. Do not create an empty new
  root on an existing install. That would change the default selection.
- Exports, reports, config and device journals use the selected state root. New
  listener and daily-check logs use `<state-root>\sidenote.log`. If it does not
  exist and `%LOCALAPPDATA%\hex-meeting-watch.log` exists, keep that file in place.
  An explicit data override isolates logs under that root. The external pipeline
  keeps its own legacy log paths listed below.
- Keep `device.json`, journal ids, file formats and UUID namespaces unchanged.
- Every `SIDENOTE_*` environment control accepts `HEX_MEETINGS_*` as an alias.
  For the same suffix the nonempty new name wins. DATA_DIR precedes STATE_DIR.
  CLI folder flags precede environment values. `--mcp`, `--daily-check`, `--listen`
  and listener options retain their behavior.

## Owner release checklist

- [ ] Record the current executable path, executable backup, shortcuts, task actions
  and arguments, listener invocation and MCP commands before changing them. Do not
  include credentials in the record. Keep the previous binary for rollback.
- [ ] The existing install is `%LOCALAPPDATA%\Programs\hex-meetings\hex-meetings.exe`.
  Choose a stable path for `sidenote.exe`, for example
  `%LOCALAPPDATA%\Programs\sidenote\sidenote.exe`. Keep the old executable available
  until every caller has been checked. Do not just rename the old binary in place.
- [ ] Close the window during the owner's release maintenance period. Update
  shortcuts to the new path. Verify the window title says Sidenote and the Meetings
  page still says Meetings. The old mutex prevents competing old and new windows.
- [ ] Inspect every scheduled task action referencing hex-meetings, including
  `Hex meetings daily check` and any listener startup task. Preserve triggers,
  account, permissions and flags. Change only the executable path where required.
  Keep the daily-check task name so a second daily writer is not registered.
  The repo helper accepts `-Exe <path>` and retains that task identity; do not run
  it until the owner has recorded the existing task definition.
- [ ] Record the listener's exact existing command and its pipeline-script path.
  Stop and replace it only during the owner's release. Preserve `--listen` and all
  listener flags. Verify one listener runs afterwards, using the intended binary.
  The Windows engine event name stays compatible with an old listener.
- [ ] Claude Code currently registers the server as `meetings`. Record its scope
  and command using the client's MCP listing. First point that registration at the
  new executable with `--mcp`; its client-side name can remain `meetings`. If the
  owner wants the name `sidenote`, remove the old registration in that same scope
  and add `sidenote` using the command in [mcp.md](mcp.md). Verify tools load once.
- [ ] Codex currently uses `meetings`. Record the existing `[mcp_servers.meetings]`
  command and arguments without exposing secrets. Update its executable path and
  keep `--mcp`. To adopt the new name, change the table to `[mcp_servers.sidenote]`
  and remove the old entry so the client starts one server. Reconnect and confirm
  server info reports `sidenote`, with meeting search and memory proposal tools.
- [ ] Review every pipeline-script hit below. Preserve legacy data and log roots
  on this install. Process-name checks must recognize the new executable before
  running import or rerun jobs against it. Archive scripts and backups deliberately.
  This lane does not edit or execute any of those scripts.
- [ ] Leave Syncthing folder ids, paths and ignore rules unchanged on existing
  installs. In particular, keep `%APPDATA%\hex-meetings`,
  `%APPDATA%\hex-meetings-peers\<device>` and `%APPDATA%\hex\meetings` where they
  already are. New devices may use Sidenote roots. Keep `device.json` device-local.
- [ ] Confirm notes, tasks, memory, reports, exports and logs still use the intended
  existing root. Check one explicit legacy environment override and one normal
  launch. Do not infer migration success just from an empty window opening.

## Read-only pipeline inventory

Searched case-insensitively for `hex-meetings` under
`C:\Users\harsha_befach\.config\hex\scripts` on 2026-11-08. Each matching line is
listed once, including backups. Paths below are relative to that directory.
No script was edited or executed.

- `hex-meeting-compare-granola.py:5`
- `hex-meeting-compare-granola.py.bak-20261108-3:5`
- `hex-meeting-import-granola.py:667`
- `hex-meeting-import-granola.py.bak-20261108-2:667`
- `hex-meeting-import-granola.py.bak-20261108-3:665`
- `hex-meeting-rerun-notes.py:348`
- `hex-meeting-rerun-notes.py.bak-20261108-3:348`
- `hex-meeting-tasks.py:5`
- `hex-meeting-tasks.py:19`
- `hex-meeting-tasks.py.bak-20261108-2:5`
- `hex-meeting-tasks.py.bak-20261108-2:18`
- `hex-meeting-tasks.py.bak-20261108-3:5`
- `hex-meeting-tasks.py.bak-20261108-3:18`
- `run-import-2026-11-07.ps1:2`
- `run-import-2026-11-07.ps1:4`
- `run-import-2026-11-07.ps1:8`
- `run-import-2026-11-07.ps1:17`
- `run-import-2026-11-07b.ps1:3`
- `run-import-2026-11-07b.ps1:7`
- `run-rerun-2026-11-07.ps1:3`
- `run-rerun-2026-11-07.ps1:5`
- `run-verify-2026-11-07.ps1:3`
- `run-verify-2026-11-07.ps1:9`

## Rollback

- [ ] Close the new window and stop the replacement listener during owner maintenance.
  Restore the recorded shortcut, scheduled-task and listener commands, and the
  prior MCP registrations. Launch the saved old executable at its old path.
- [ ] Keep the selected legacy data root unchanged. This rename changes no stored
  format and moves no data. If an empty Sidenote root was accidentally created,
  select the legacy root explicitly rather than deleting or merging either folder.
- [ ] If any later release step edited data or scripts, restore that step's own
  backup separately. Confirm one window, one listener and the expected task actions.

## Deliberate legacy names

- `meetings-domain`, its UUID namespace URLs and domain tests remain unchanged.
  Renaming identity inputs would change deterministic ids.
- `Local\HexMeetingsWindow` and `Local\HexMeetingsEngineChanged` remain unchanged
  for coexistence with old windows and listeners. The focus lookup accepts both titles.
- Legacy folder names, `hex-meeting-watch.log` and `HEX_MEETINGS_*` appear in
  compatibility code and docs.
- `Hex meetings daily check` remains the scheduled task identity. The helper falls
  back to the installed old executable and accepts an explicit new path.
- `assets/icon/meetings.ico` and meeting fixture/temp-folder names remain internal
  filenames. They are not product labels and need no asset or data migration.
- `hex-meetings` scene keys mean Hex's Meetings settings section, not the product.
  The Meetings page, recording nouns, tool names, `meeting://` resources, data
  schemas and planned internal `meetings-*` crate/service names retain their meaning.
- Existing sync examples use legacy roots so owners keep shares in place. The
  distribution ADR retains the historical install path, and the ASR ADR retains
  its existing review-folder reference. Repo paths and the remote remain unchanged.

## Verification boundary

Only source changes, required local Cargo gates and isolated headless renders belong
in this lane. No test suites run. The installed app and owner migration remain
unverified in the live system until the owner performs this checklist.
