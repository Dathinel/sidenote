# Releases and updates

Sidenote uses Velopack 0.0.1298 (MIT). Packaging uses the matching CLI and the
per-user .NET SDK 8.0.427. These builds are unsigned.

`tools/release_gate.ps1` takes read-only process snapshots of the installed
listener before and after its checks. Both states appear in its report table.
If a listener was running at the start and is absent at the end, the gate fails
with a recorder warning. An unreadable process snapshot also fails the gate.
The snapshots never start, stop or reconfigure the installed app or its task.

Windows installs check their channel's HTTPS feed in `harshanandak/sidenote`.
Stable reads `/releases/latest/download/releases.stable.json`. Nightly reads
`/releases/download/nightly/releases.nightly.json` from the rolling Nightly
prerelease. Settings > Updates can change the feed. Switching to Stable keeps
the current build installed until Stable has a newer version. Checks run at
startup and every six hours.
Window focus also wakes the existing update worker when it is idle; the worker
checks only if its last attempt (including a failure) was over 30 minutes ago.
Manual builds do not start the worker. Manual checks can retry sooner.

For development, set `SIDENOTE_UPDATE_FEED` in the launching environment to an
absolute local folder or UNC file share containing the selected channel's JSON and its
packages. This replaces the public source; URLs and relative paths are rejected.
Clear the override to return to GitHub Releases. Restrict write access to local
feeds: hashes detect corruption but do not authenticate an unsigned publisher.

## Pack and publish a release

1. Set the package version in `Cargo.toml` and update the workspace package's
   entry in `Cargo.lock`. Stable starts at `0.1.0`, above `0.0.1-dev.47`.
   Nightly starts at `0.1.1-dev.1`. Increment its numeric suffix for each build.
   When Stable reaches the Nightly base version, advance Nightly to the next
   patch base so every published Nightly sorts above the latest Stable.
2. Install the matching Velopack CLI 0.0.1298 and .NET SDK. From the checkout,
   run `tools/local_build.ps1 -Pack` with the pinned release arguments and
   `-Channel stable` or `-Channel nightly`. Gate with the same channel.
   The script builds and packs locally; it does not upload anything.
3. Use a fresh output folder for each channel's first release. Put
   `sidenote-<channel>-Setup.exe`, `releases.<channel>.json`, and **every
   referenced `.nupkg`** on the channel's GitHub release, preserving filenames.
   The feed and all referenced packages must be assets on this same release,
   even when a package came from an older release. The app downloads full packages.
4. Mark the version-tagged Stable release as latest. Use the `nightly` tag
   for the rolling Nightly prerelease. Verify each feed and its assets.

The package includes `LICENSE`, `THIRD_PARTY.md`, the Inter font licence,
the webpki-roots CDLA licence, `DEPENDENCY_NOTICES.md`, and upstream licence,
copyright and notice files under `dependency-notices/<crate>-<version>/`.
Notices cover the locked Windows GNU dependency graph with the same default
features used by the release build. Cargo metadata uses
`--filter-platform x86_64-pc-windows-gnu`; the generator also follows the returned
resolve-node IDs, so packages from other platform graphs are not bundled.
For crates that omit licence files, `tools/dependency_notices.ps1` generates a
notice from the crate's name, version, authors (or repository) and complete
declared licence expression. The common permissive texts committed under
`tools/licenses` are copied once under `dependency-notices/standard-licenses`.
The generated notice supplies its own metadata attribution; it does not borrow
the cached text donor's copyright holder. Unknown or non-permissive identifiers,
licence exceptions and missing attribution stay uncovered.
Owner-approved choices of declared standalone OR alternatives are recorded in
`tools/license_choices.json`, scoped to an exact crate version and expression.
The notice records both the full declaration and the selected licence.
The packer writes `DEPENDENCY_NOTICE_COVERAGE.json` and refuses every uncovered dependency before
building or packing. `tools/dependency_notices_proof.ps1` checks both generated
notices and refusal cases in fresh OS temp folders, and runs in CI.
For Git workspace dependencies, shared notices are collected from the checkout root. Original relative paths
are preserved. Fetch the locked sources before the offline packaging step.

The default release output is `<checkout>/artifacts/releases`. Set `-Feed` to
choose another absolute folder. The script uses Cargo's configured target
directory (including `CARGO_TARGET_DIR`), `DOTNET_ROOT` or `-DotnetRoot`, and
`RUST_GNU_BIN` or `-GnuBin` rather than machine-specific tool paths. Each pack
uses a fresh staging folder under `artifacts/updates-core`.

The script retains three full versions and their deltas in its local output.
Publish a previous build with a higher version to roll back. These installers
and packages are unsigned. No release is packed or uploaded by ordinary builds.

`--update-check` checks and downloads. `--update-status` reports the current
version, isolated paths, source and last status in JSON and writes the same
report to `%LOCALAPPDATA%/Sidenote/updates/update-report.json`.
The report is also refreshed when the window's update worker starts, after an
update hook, and whenever update status is saved. `current_version` comes from
the running binary; `reported_at` is a UTC RFC 3339 timestamp. Reports are replaced
atomically so readers do not see a partially written JSON file. `--update-apply` follows the same safety
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
Registration adds a user-logon trigger and an indefinite five-minute repeating
trigger, with `MultipleInstances IgnoreNew` and no execution time limit. The
repeating trigger can recover a stopped listener while the window is closed.
Startup registration migrates an existing task only after checking its action,
user and privilege level, then re-reads and validates the saved definition.
Expected duplicate starts quietly exit after the listener's three-second lock
retry. Other lock errors remain logged.
The listener writes start, last-heartbeat and controlled-stop timestamps to
`<state-root>/local/recorder-heartbeat.json`, refreshing every 30 seconds. On
restart, a gap over two minutes is logged and shown in the window as a
dismissible "Recorder was off from X to Y" line in local time. After an abrupt
exit the start of the gap is the last observed heartbeat, not an exact crash
time. Dismissal is stored separately in `local/recorder-gap-dismissed.json`.
These files contain timestamps only, with no meeting content.
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
If PowerShell task validation reaches its ten-second deadline while the listener
lock is held, the window keeps recording available and retries validation on its
background worker after 30 seconds. A definite task validation failure remains
visible until a successful validation; a later timeout cannot clear it. A free
listener lock still reports that the listener is absent.
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
5. Change known MCP configs to that same stable exe with `--mcp`. Clear any development feed
   override in the window's launching environment. Launch and check data and listener
   status before using Install and restart.
6. To reverse the migration, stop the new listener and restore the saved task and
   MCP definitions. The old install and data remain available.

Packing and publication are manual release steps. Ordinary app startup never
uploads packages or changes GitHub releases.
