# Local updates

Sidenote uses Velopack 0.0.1298 (MIT). Packaging uses the matching CLI and the
per-user .NET SDK 8.0.427. These builds are unsigned.

Set `SIDENOTE_UPDATE_FEED` in the launching environment to an absolute local
folder or UNC file share containing `releases.win.json` and its packages.
There is no HTTP update source and no repository URL. A disconnected share
produces a retryable error. An unset feed disables automatic checks.

Run `tools/pack_release.ps1 -ReleaseNotes <markdown-file>` after changing the
package version in Cargo.toml. The script builds at BelowNormal priority with
the shared GNU target directory, copies its own binary and packages locally.
It retains three full versions and their deltas. Publish a previous build with
a higher version to roll back. Restrict write access to the feed: hashes detect
corruption, but an unsigned feed does not authenticate its publisher.

`--update-check` checks and downloads. `--update-status` reports the current
version, isolated paths, source and last status in JSON and writes the same
report to `local/update-report.json`. `--update-apply` follows the same safety
and listener path as the window, then restarts to `--update-status` for proof.
No CLI helper silently applies a cached update. The normal window tries a
verified pending package at its next startup. All apply paths recheck SHA256.
Folder flags are preserved on restart. A custom `--data-dir` must also be set
through `SIDENOTE_DATA_DIR` or `SIDENOTE_STATE_DIR` in the launcher, so Velopack's
early callbacks use that same folder. Apply refuses a mismatch before stopping
the listener.

Create a per-user scheduled task named `Sidenote meeting listener` whose sole
action is the stable installed `current/sidenote.exe` with `--listen`.
`SIDENOTE_LISTENER_TASK` can select another task beginning with `Sidenote `.
Apply refuses a task with another action or executable. It never edits a task.
The listener watches a stop request, closes its event stream and exits before
apply. The after-update hook keeps the resume marker. Only the restart hook starts
the validated task, after Velopack's process cleanup. An OS-held lock
allows only one listener per data folder. A stopped or missing task prevents
installation until configured, rather than guessing a replacement.

Jobs hold shared OS file handles. Apply requires an exclusive handle and a fresh
idle Hex status. Ask, routing, daily checks, runner jobs and listener-launched
notes work participate. A second updater refuses an open window. Unknown Hex
status blocks apply; an absent Hex is idle. Hex remains an external app and can
start recording just after a status check, so the final check narrows that race
but cannot atomically reserve Hex. No dictation setting is written.
An idle MCP connection does not block installation. A memory proposal holds the
job guard while it writes. Updates may disconnect idle MCP clients; reconnect
them using the stable executable path.

## Move the manual install later

1. Keep the manual install and its data in place. Back up its task definitions
   and MCP configuration before editing them.
2. Install the unsigned Setup for the chosen Sidenote version at
   `%LOCALAPPDATA%/sidenote`. Windows or managed-device policy may warn or refuse
   it until a publisher is signed or the administrator allows its hash.
3. Keep using the existing data folder. Do not create an empty replacement.
4. At a quiet time, quit the old window and stop the old listener through its existing owner workflow.
   Change its launcher to a new Sidenote task pointing to
   `%LOCALAPPDATA%/sidenote/current/sidenote.exe --listen`, with single-instance
   task policy. Keep the old task disabled for rollback.
5. Change known MCP configs to that same stable exe with `--mcp`. Set the local
   feed in the window's launching environment. Launch and check data and listener
   status before using Install and restart.
6. To reverse the migration, stop the new listener and restore the saved task and
   MCP definitions. The old install and data remain available.

No migration is performed by this lane. D1's hosted release and CI install job
remain deferred under the owner's local-only instruction. Nothing is uploaded.
