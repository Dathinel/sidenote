# Local updates

Sidenote uses Velopack 0.0.1298 (MIT). Packaging uses the matching CLI and the
per-user .NET SDK 8.0.427. These builds are unsigned.

Set `SIDENOTE_UPDATE_FEED` in the launching environment to an absolute local
folder or UNC file share containing `releases.win.json` and its packages.
There is no HTTP update source and no repository URL. Apply preparation makes
loopback-only HTTP requests to Hex's local status API. There is no other network
request from the updater; a UNC file share uses Windows file I/O. A missing feed
produces a retryable error. An unset feed disables automatic checks.

Run `tools/pack_release.ps1 -ReleaseNotes <markdown-file>` after changing the
package version in Cargo.toml. The script builds at BelowNormal priority with
the shared GNU target directory, copies its own binary and packages locally.
It retains three full versions and their deltas. Publish a previous build with
a higher version to roll back. Restrict write access to the feed: hashes detect
corruption, but an unsigned feed does not authenticate its publisher.

`--update-check` checks and downloads. `--update-status` reports the current
version, isolated paths, source and last status in JSON and writes the same
report to `%LOCALAPPDATA%/Sidenote/updates/update-report.json`. `--update-apply` follows the same safety
and listener path as the window, then restarts to `--update-status` for proof.
No CLI helper silently applies a cached update. The normal window tries a
verified pending package at its next startup. All apply paths recheck SHA256.
Folder flags are preserved on restart. All updater locks, status, markers and
reports live under `%LOCALAPPDATA%/Sidenote/updates`, independently of meeting
and app data. Update checks and hooks never create `%APPDATA%/sidenote`.
If neither app-data root exists, the app defaults to the legacy `hex-meetings`
root. An existing `sidenote` data root remains readable without creating a new one.
A manual copy is detected before bookkeeping and quietly disables updates.
Settings says "Updates are off for this manual install". Its listener, MCP and
daily-check commands do not acquire updater locks.

Create a per-user scheduled task named `Sidenote meeting listener` whose sole
action is the stable installed `current/sidenote.exe` with `--listen`.
`SIDENOTE_LISTENER_TASK` can select another task beginning with `Sidenote `.
Apply refuses a task with another action or executable. It never edits a task.
The listener watches a stop request, closes its event stream and exits before
apply. The after-update hook keeps the resume marker. Only the restart hook starts
the validated task, after Velopack's process cleanup. An OS-held lock
allows only one installed listener per user. The stop marker contains a PID and
UTC timestamp. During preparation that PID is the app; after handoff it is the
actual Velopack Update.exe child. Listeners delete and ignore markers whose PID
is gone, whose timestamp is over ten minutes old, or whose contents are invalid.
Every window start checks the listener lock and starts the configured task when
no listener holds it. This also recovers after an asynchronous update failure.
Missing or mismatched task definitions produce a plain error, never a guessed
replacement task.

Jobs hold shared OS file handles. Apply requires an exclusive handle and a fresh
idle Hex status. Ask, routing, daily checks, runner jobs and listener-launched
notes work participate. A second updater refuses an open window. Unknown Hex
status blocks apply when a Hex process is present, including an unreadable or
missing local status API. Failure to inspect the process list also blocks apply.
An entirely absent Hex process is allowed. The recording probe is best effort:
Hex can start recording after the final check because it does not participate
in a reservation. The updater cannot eliminate that race.

The listener therefore reconciles completed meetings on every start, before
connecting to the event stream. It queues completed meetings for the configured
Rust router and runs the normal idempotent notes watcher, which scans completed
manifests and skips meetings that already have notes. A meeting that finishes
while the listener is stopped is processed after restart, even if the event API
is still unavailable. Existing pipeline retry and audio-size policies remain.
No dictation setting is written.

Known limit: once Update.exe has been started, Sidenote exits and never rolls
back, because a restarted listener could be mid-job when files are replaced. If
Update.exe then crashes without restarting Sidenote, nothing restarts the
listener until the next Sidenote window or the listener task's next trigger (for
example sign-in). Meetings recorded meanwhile are not lost; the startup catch-up
above processes them then. A separate watchdog would close this gap; it is not
built.

Apply preparation, checksum reads, status
probes, task commands and listener-stop waits run on a worker. The window
publishes "Preparing update" and "Stopping the listener" while staying usable.
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
