Decisions needed: none.

# Idle CPU implementation and qualification

Head is ad2f9c5b10d8ef6072cd04652c2d4d2a20340072 on fix/idle-cpu, based on c0d97d25a09f519fc812cbda61bc9501ee0c4792 (release/dev32). Six local commits exist; the tracked working tree is clean; no pushes or GitHub writes. Final isolated release qualification passes the CPU budget: headless UI 0.990% of one core, recorder 0.313% empty / 0.311% with 300 completed meetings. Evidence is rung 4 (production paths run in isolation), not rung 5 installed-system verification. Earlier investigation notes are preserved in PREVIOUS-REPORT.md.

The installed native-window high CPU and the owner's Ctrl+Space symptom remain unverified. Headless UI processor use did not decrease: its baseline already met the budget. The verified window changes eliminate idle rebuilds and a delayed-empty-poll self-wakeup defect. No installed app, setting, model, Hex priority or owner recorder log was changed. Only the required build wrapper read its permitted recorder-log meeting gate.

| Commit | Purpose | git diff --shortstat against parent |
| --- | --- | --- |
| 3fb505d | Recorder history reads and production regressions | 4 files changed, 153 insertions(+), 1 deletion(-) |
| 8748434 | Informational install CPU row | 1 file changed, 27 insertions(+) |
| cc76868 | UI idle change detection and regressions | 4 files changed, 328 insertions(+), 5 deletions(-) |
| b4ad7ae | Opt-in isolated release CPU qualification | 6 files changed, 202 insertions(+), 6 deletions(-) |
| b572299 | Avoid repeated metadata ancestry reads for accepted final outputs | 1 file changed, 8 insertions(+), 5 deletions(-) |
| ad2f9c5 | Settle real refresh initialization before the idle assertion | 2 files changed, 24 insertions(+), 1 deletion(-) |

## Verified baseline (rung 4)

The release baseline was built through tools/local_build.ps1, with the prescribed GNU tools first on PATH, using `build --release --target x86_64-pc-windows-gnu --bin sidenote`. Exit 0; the cold build reported 50m 22s. Existing seven library warnings were retained.

A byte copy is artifacts/idle-cpu/baseline-sidenote.exe, SHA-256 C6F99F872EEE5DFB24F2CDA9AF5C42D9592FD0C4D6C54353F5BF72A48FD6B58D.

Recorder trials launch that release executable with --listen and explicit temporary data, meetings and peers paths. APPDATA is also temporary, and a made-up notes configuration prevents legacy script migration. A loopback synthetic Hex endpoint reports protocol 4, idle capture and idle meeting state; its event endpoint returns 404. There is no audio and no actual Hex process is contacted. Notes use the app engine with no helper; Labs retain defaults. No native window is created because there is no current or pending recording. After five seconds, the historical-data trials also wait for all synthetic completions to be acknowledged before the CPU sample. Each sample uses a fixed 30-second deadline and reports processor seconds and simultaneous whole-system CPU load from GetSystemTimes. Percentage is of one core, not divided by the logical processor count.

| Baseline configuration | Actual seconds | Processor seconds | One-core CPU | System load | Synthetic status requests |
| --- | ---: | ---: | ---: | ---: | ---: |
| Empty meetings | 30.000 | 0.015625 | 0.0521% | 98.2161% | 28 |
| 300 completed meetings, short three-word transcripts | 30.094 | 0.890625 | 2.9595% | 96.2888% | 30 |
| 300 completed meetings, eligible 25-word transcripts | 30.015 | 1.078125 | 3.5920% | 87.3828% | 30 |

The eligible-meeting trial asserts CPU below 1% and failed with exit 1. Its raw result is in recorder-before-eligible-300.log. The other raw samples are recorder-before-empty.log and recorder-before-300.log. These trials exclude native UI/GPU rendering, real meeting data, active recording, audio inference, connected SSE and the owner's installed configuration. They establish a synthetic recorder reproduction, not the exact cause in the installed process.

Read-only supporting observations of the installed processes were also taken. A 10-second informational row reported window 12.12% and recorder 9.60%; activity was not checked and no simultaneous system-load value was collected, so these numbers are not a before/after comparison. A separate 30-second read-only observation reported window 4.109375 CPU seconds (13.653%) and recorder 1.500000 CPU seconds (4.995%), with system load 100% at the start only. No owner data or recorder log was read for these observations.

## Recorder regression: red run (rung 4)

The integration source is tests/idle_cpu.rs. An artifact Cargo harness references that same source and the production Sidenote library as a path dependency. It uses the same release profile: opt-level s, thin LTO, one codegen unit, stripped symbols and unwind panic. This avoids launching the desktop app. All cargo commands still go through tools/local_build.ps1 with target x86_64-pc-windows-gnu.

Command, after setting temporary data/meetings/peers/APPDATA and the prescribed GNU PATH:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/local_build.ps1 test --manifest-path artifacts/idle-cpu/harness/Cargo.toml --release --target x86_64-pc-windows-gnu --lib idle_cpu --no-fail-fast -- --nocapture --test-threads=1
```

The original production code failed both tests:

```text
idle_cpu_completed_source_does_not_reread_acknowledged_transcripts
acknowledged source idle read bytes=400173
FAILED: idle discovery reread historical transcript payloads: 400173

idle_cpu_notes_catch_up_skips_acknowledged_final_outputs_but_sees_new_meetings
acknowledged notes idle read bytes=800504
FAILED: idle notes catch-up reread historical transcript payloads: 800504

test result: FAILED. 0 passed; 2 failed; 0 ignored
error: test failed
```

The tests create public synthetic files, acknowledge them through the production Session::drain and notes queue, and measure actual process file-read bytes with GetProcessIoCounters. They also require new meetings, deleted final outputs and malformed-cursor fallback to remain discoverable. The full red output is recorder-red.log, exit 101. The first green run (recorder-green.log) passed both tests. The final exact-source conventional integration binary also passed both tests, exit 0 (recorder-final-green.log):

```text
acknowledged source idle read bytes=69
ok
acknowledged notes idle read bytes=270
ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 filtered out
```

## Cause and production-path UI proof

Source inspection (rung 2) found repeated transcript capture before the durable-cursor duplicate check in core/recording_source.rs and core/recording_source/folders.rs. The periodic notes catch-up in core/notes/queue.rs also re-enters eligibility classification for accepted historical outputs. The small recorder change moves acknowledgement checks ahead of these no-op historical reads. It retains new-input validation, root enumeration, cursor retention, missing-output recovery and fallback when the cursor cannot be read. Reparse-path rejection is kept before the final-output metadata shortcut.

At the committed head, the recorder call sites are [Session::drain](../../src/core/recording_source.rs#L144), [folder catch-up](../../src/core/recording_source/folders.rs#L33) and [notes catch-up](../../src/core/notes/queue.rs#L837). The first fix still repeatedly checked receipt/job path ancestry through [reject_reparse](../../src/core/store_io.rs#L227). The final queue shortcut checks the meetings root once and uses non-linked child metadata to skip acknowledged final outputs before those repeated reads. Queued jobs still enter [start_worker](../../src/core/notes/queue.rs#L883) and its existing validation. No worker gate was removed. The periodic caller remains [listener.rs:616](../../src/listener.rs#L616), with its existing 30 s sleep; the completion watcher retains its channel/30 s wait at [meeting_watch.rs:558](../../src/core/meeting_watch.rs#L558). The Linux event adapter retains its prior default catch-up path; this Windows fix does not redesign it.

The window's [apply_pending](../../src/ui/mod.rs#L1683) previously marked every scan/status event changed, including no-data scans and identical status. It also checked its 4 ms deadline before processing any event, then woke itself when that budget expired even with an empty queue. Under scheduling delay it could reschedule without consuming anything. The fix requires event progress before a budget-yield wake and reports changes only for actual refresh data or changed Hex/display state. [hex_status_event](../../src/ui/hex.rs#L545) still processes every status sample, including retries, installation detection, visibility and live progress. These mechanisms are reproduced by the red/green tests (rung 4). The UI is Rune, not egui. This does not conclusively attribute the owner's much larger native-window CPU figure.

The UI test module src/ui/idle_cpu.rs uses the production Rune Runtime, view, poll, event queue and refresh worker. It creates 300 synthetic meetings and no native window or GPU. Tests cover identical Hex results over 30 simulated seconds, unchanged refresh results, an empty poll delayed past its four-millisecond budget, and RPC status transitions including dictation and meeting progress. The opt-in idle-cpu-proof feature enters before native-window startup, configures its own temporary folders and APPDATA, and runs those same checks in a normal optimized release executable. An ignored unit measurement is also exposed by that opt-in entry with --measure.

The actual release baseline red run is in ui-red.log: delayed empty poll FAIL (1 extra wake, expected 0); status and meeting progress PASS; simulated idle FAIL (20 rebuilds / 20 wakes, no animation continuation requests); unchanged production refresh FAIL (commit count 3 versus expected 2). Total 1 passed / 3 failed, exit 1. This is a test assertion failure, not a compiler failure.

The fixed-period 30-second headless baseline (ui-before.log) reported 30.079 actual seconds, 0.281250 processor seconds, 0.935% of one core, system load 58.689%, 23 UI rebuilds, 24 loop wakeups, 0 animation continuation requests and 20 status calls. Priority class 32 (normal). The synthetic endpoint is absent in this sample, so these are real HexEngine discovery/status errors against the temporary root. The kernel status-event watcher and 500 ms visibility forwarder are excluded to avoid consuming the installed app's shared Windows event. Native rendering/GPU are excluded. The headless baseline already falls below 1%; it reproduces wasteful work, not the owner's much larger native-window CPU figure. I did not establish a guaranteed hidden native launch path and used the brief's headless alternative. The UI fixture has 300 synthetic summary folders; initialization and font loading are excluded by a five-second settling period. No audio or inference runs.

The UI release baseline build succeeded in 19m57s. Its executable is ui-before-sidenote.exe, SHA-256 578A87E3DACB594E281126CAF99DAE76D6ED5824546349A4D5538DF231DA35FE. Its production UI was unchanged; the first recorder fix and opt-in qualification instrumentation were already present. Production UI changes preserve status processing on every poll and only suppress no-change view rebuilds; live updates remain active.

The first fixed release built in 14m53s (b4ad7ae; first-fix-sidenote.exe SHA-256 587BD34F45ACFBC3780147275113E67E36E7C4694B37D04BC18883A867518F9D). Its headless UI sample (ui-after-first-fix.log) used 0.296875 CPU s over 30.009 s, 0.989% of one core, load 83.518%, 0 UI rebuilds / 24 wakes / 0 animation continuation requests / 20 status calls. Its empty-recorder sample (recorder-after-empty-first-fix.log) used 0.140625 CPU s over 30.000 s, 0.46875%, load 80.6966%, 30 status calls.

The first fixed 300-meeting recorder sample still FAILED: 1.046875 CPU s / 30.000 s, 3.48958%, load 87.5024%, 30 status requests (recorder-after-eligible-300-attempt1.log). A diagnostic repetition reported 0.968750 CPU s over 30.093 s, of which 0.906250 were kernel CPU, only 17,944 read bytes and one periodic thread consuming 0.890625 CPU s; load 97.9992% (recorder-diagnostic.log). The payload-read regression alone did not identify the full cost. Repeated ancestor metadata checks for old completed outputs remained. The final metadata shortcut described above removes that cost in the isolated reproduction; final results follow below. Both failed attempts remain in the artifacts.

The first fixed functional release run passed delayed-empty-poll, status/progress and simulated idle checks (2 rebuilds / 20 wakes), but failed the unchanged-refresh fixture. The fixture was not yet known to be idle after applying initialization data. It now completes real initialization scans until the worker reports an unchanged snapshot, then explicitly checks the next snapshot is unchanged before the strict no-rebuild assertion. No test result is fabricated and the assertion is retained. The corrected fixture passes in debug and final release and fails after reverting the production fix. The earlier attempt is preserved as ui-green-attempt1.log.

## Final release qualification (rung 4)

The release at ad2f9c5 built successfully through the wrapper in 14m55s, exit 0 (final-release-build.log). The seven existing library warnings remain. The executable is final-sidenote.exe, SHA-256 B035B07E31D256C6950F0F47C53F268CEA54ACC0533E21ACC5C2A0AFB29BDF4C. It includes the opt-in idle-cpu-proof feature for the headless entry; default shipping features are also enabled. Recorder samples use its ordinary --listen path. All final samples passed their under-1% assertion, exit 0.

| Final release configuration | Actual seconds | Processor seconds | One-core CPU | System load | Status calls |
| --- | ---: | ---: | ---: | ---: | ---: |
| Headless UI, same 300 synthetic summary folders | 30.002 | 0.296875 | 0.990% | 67.728% | 20 |
| Recorder, empty | 30.000 | 0.093750 | 0.3125% | 90.9831% | 30 |
| Recorder, same 300 eligible complete meetings | 30.140 | 0.093750 | 0.3110% | 80.2124% | 30 |

Raw files: ui-after.log, recorder-after-empty.log, recorder-after-eligible-300.log. The final UI sample had 0 rebuilds, 24 loop wakes and 0 animation continuation requests. Its CPU did not decrease relative to the headless baseline; native rendering remains outside this measurement. There is no latency or accuracy result; all trials have no audio. Whole-process CPU includes the recorder's normal background loops, so the improvement is not a transfer to another thread in that process. This does not measure real Hex's CPU.

The final release functional entry passed all four checks (ui-release-final-green.log):

```text
delayed empty poll: PASS
Hex state and meeting progress: PASS
simulated 30 s: animation repaint requests=0, UI rebuilds=2, loop wakeups=20
simulated 30 s idle: PASS
unchanged snapshot verified
unchanged refresh: PASS
idle CPU proof: 4 passed; 0 failed
proof_exit=0
```

## Polls and Hex source (rung 2)

The existing UI status watcher waits 1.5 seconds and uses a 750 ms status timeout. The recorder has its separate approximately three-second status evidence poll and a floating-icon status poll at 1.5 seconds. Their information overlaps while the window is visible; the recorder still needs independent operation when the window is closed or hidden. No poll interval, timeout, Hex setting, model or priority is changed by this work.

The upstream status path was inspected read-only at `<hex-source>`: src/control/host.rs:124 dispatches Status and :544 constructs its response from published state, model-install metadata, memory footprint and meeting state. src/windows/rune.rs:425 clones published runtime state and reads visibility; the status branch does not send a dictation shortcut command or run inference. Model-installed metadata is read at src/transcription/models.rs:913. Rung-2 inference: this fixed work should be small relative to transcription. Whether it meaningfully loads the owner's Hex during dictation is unknown without a live CPU/latency measurement. No live Hex endpoint was contacted. The Ctrl+Space HUD symptom remains unverified. UI samples made 20 status calls before and after; final recorder samples each made 30, so this fix adds no status requests.

## Install-check row (rung 4)

The added row is informational and does not fail installation. It samples existing window and recorder processes for ten seconds, reports percentage of one core and does not launch an application. The exact row expression from tools/install_check.ps1 was extracted with the PowerShell parser and executed, together with the function extracted from that file:

```powershell
$rows.Add([pscustomobject]@{ Check = 'Idle CPU (informational)'; Result = 'INFO'; Observed = (Measure-IdleCpu $exe) })
```

Observed: `INFO 10 s idle observation; % of one core; window=12.12%; recorder=9.60%; activity not checked`. Exit 0. The full install check was deliberately not run because its existing log row reads the prohibited owner recorder log. The build wrapper's required meeting gate is the only permitted exception and was left intact.

## Round-one threat model

1. UI status or meeting progress could stop updating: exercise production RPC transitions and preserve live-meeting updates and worker notifications.
2. CPU could move elsewhere or to Hex: measure whole-process CPU including workers, retain poll cadences, and report synthetic endpoint request counts. Live Hex cost remains unmeasured.
3. Recorder detection could be delayed: retain original polling and enumeration, skip only durably acknowledged history, and test new completions plus missing-output and corrupt-cursor fallback.

## Current conventional test results (rung 4)

All cargo invocations use tools/local_build.ps1, GNU first on PATH, target x86_64-pc-windows-gnu, --no-fail-fast and --test-threads=1. The ordinary test profile is unoptimized + debuginfo, not a CPU benchmark. Temporary data, meetings, peers and APPDATA are used.

- `test --bin sidenote idle_cpu -- --nocapture`: final corrected fixture, 4 passed / 0 failed / 1 ignored / 120 filtered; exit 0 (ui-final-green.log).
- `test --bin sidenote -- --test-threads=1`: whole UI test binary, 123 passed / 0 failed / 2 ignored; exit 0 (ui-final-whole.log). Ignored: the release processor sample and an existing production-corruption receipt proof.
- `test --lib recording_source`: 21 passed / 0 failed; exit 0 (source-unit-green.log).
- `test --lib core::notes::tests`: 64 passed / 0 failed; exit 0 (notes-unit-green.log). An earlier `notes::queue` filter matched zero tests; it is not counted as verification.
- `test --test idle_cpu`: whole new integration binary, 2 passed / 0 failed; source reads 69 bytes, notes reads 270 bytes; exit 0 (recorder-final-green.log).

The corrected UI fixture was also run after reverting only production apply_pending to dev.32. It explicitly verified an unchanged producer snapshot, then FAILED: delayed empty poll 1 wake rather than 0; simulated idle 20 rebuilds; unchanged scan commit 4 rather than 3. Total 1 passed / 3 failed / 1 ignored, exit 101 (ui-reversion-red.log). The original production body was restored afterward. Copying the backup preserved a timestamp older than the red binary, so Cargo initially reused the red binary for two subsequent runs; those are preserved as ui-stale-restoration-filter.log and ui-stale-restoration-whole.log and are not counted as green. Setting the restored source's modification time to now caused an actual rebuild (27.12 s) and the passing final runs above. No test, hook or build gate was bypassed.

```text
Reversion red (debug, ui-reversion-red.log):
assertion failed: an empty poll rescheduled itself after its budget expired
  left: 1
 right: 0
simulated 30 s: animation repaint requests=0, UI rebuilds=20, loop wakeups=20, commits=20
refresh initialization scans=1; unchanged snapshot verified
assertion failed: unchanged production scan rebuilt the UI
  left: 4
 right: 3
test result: FAILED. 1 passed; 3 failed; 1 ignored
exit 101

Restored green (debug, ui-final-green.log):
idle_cpu_delayed_empty_poll_sleeps ... ok
idle_cpu_hex_state_changes_reach_the_view ... ok
simulated 30 s: animation repaint requests=0, UI rebuilds=2, loop wakeups=20, commits=2
idle_cpu_simulated_thirty_seconds ... ok
refresh initialization scans=1; unchanged snapshot verified
idle_cpu_unchanged_refresh_does_not_rebuild ... ok
test result: ok. 4 passed; 0 failed; 1 ignored
exit 0
```

The exact install-check row and function were extracted with the PowerShell AST and executed again. Result: `INFO 10 s idle observation; % of one core; window=12.46%; recorder=14.60%; activity not checked`, exit 0 (install-cpu-row.log). An initial extraction used a child function scope and failed to resolve the helper; the corrected run dot-sourced the exact function, with ErrorActionPreference Stop. The successful row is the only result counted.

## Commands and remaining verification limits

Every command below was run from this worktree, with `<toolchain>/bin` first on PATH. All cargo commands use the wrapper and GNU target; all test commands retain --no-fail-fast. Builds do not override USERPROFILE, LOCALAPPDATA, HOME, CARGO_HOME or RUSTUP_HOME. Test/trial folder and APPDATA overrides are temporary.

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/local_build.ps1 build --release --target x86_64-pc-windows-gnu --bin sidenote
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/local_build.ps1 build --release --target x86_64-pc-windows-gnu --bin sidenote --features idle-cpu-proof
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/local_build.ps1 test --target x86_64-pc-windows-gnu --bin sidenote idle_cpu --no-fail-fast -- --nocapture --test-threads=1
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/local_build.ps1 test --target x86_64-pc-windows-gnu --bin sidenote --no-fail-fast -- --test-threads=1
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/local_build.ps1 test --target x86_64-pc-windows-gnu --lib recording_source --no-fail-fast -- --test-threads=1
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/local_build.ps1 test --target x86_64-pc-windows-gnu --lib core::notes::tests --no-fail-fast -- --test-threads=1
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/local_build.ps1 test --target x86_64-pc-windows-gnu --test idle_cpu --no-fail-fast -- --nocapture --test-threads=1
python artifacts/idle-cpu/run_ui_proof.py artifacts/idle-cpu/ui-before-sidenote.exe
python artifacts/idle-cpu/run_ui_proof.py artifacts/idle-cpu/ui-before-sidenote.exe --measure
python artifacts/idle-cpu/run_ui_proof.py artifacts/idle-cpu/final-sidenote.exe
python artifacts/idle-cpu/run_ui_proof.py artifacts/idle-cpu/final-sidenote.exe --measure
python artifacts/idle-cpu/recorder_sample.py artifacts/idle-cpu/baseline-sidenote.exe --mock
python artifacts/idle-cpu/recorder_sample.py artifacts/idle-cpu/baseline-sidenote.exe --mock --meetings 300
python artifacts/idle-cpu/recorder_sample.py artifacts/idle-cpu/final-sidenote.exe --mock
python artifacts/idle-cpu/recorder_sample.py artifacts/idle-cpu/final-sidenote.exe --mock --meetings 300
```

The original release recorder tests used the artifact Cargo harness command printed above; final tests use the ordinary repository target. The initial release UI test compile encountered private-field errors, then its corrected full release test-binary build terminated with rustc exit 0xffffffff and no diagnostic establishing the cause (ui-release-unit-build.log). The ordinary debug tests and isolated release entry succeeded. These failures were not counted as regression reds.

Fault disposition: repeated historical payload/metadata work and wasteful UI rebuild/empty-wake behavior are fixed here. Native-window CPU attribution and Ctrl+Space behavior remain open. Poll redundancy is noted and unchanged. No unrelated fixes were added.

Not verified: installed behavior; native/GPU CPU below 1%; the omitted shared-event/visibility-forwarder overhead; real Hex dictation CPU/latency/HUD; the owner's data/configuration/history scale; a real recording/audio/language pipeline; connected SSE; all-feature release check; Linux boundary self-test; independent review; installation/package acceptance. The lead still needs the prescribed independent review and merged-tree release/Linux checks. No installation, push or GitHub write is included.

Elapsed-work buckets: orientation covered source paths and isolation; edits produced six small commits; verification covered CPU samples, production reversion red/green runs and module/whole-binary tests; rework covered the incomplete initial recorder diagnosis, fixture settling, restore timestamps and AST function scope; waiting was dominated by cold compilation under load and wrapper gates. Largest avoidable waste: qualifying the first recorder fix before separating kernel metadata cost from payload reads, which required another release build. Recorded build times are verification costs, not product performance claims; inactive/interrupted time is not attributed to product runtime.

## Round 2 (2026-11-16)

Decisions needed: none. Both requested findings are implemented; all four final CPU samples passed. The additional release functional check has one unresolved failure, recorded below; this is not an unqualified release-acceptance claim.

The preceding sections are the round-one record at ad2f9c5. This round starts from that supplied head on fix/idle-cpu, with base c0d97d2. Code/qualification head is 09dc5fcbd6e4c2ce9c5ec35d81dede6679af051b; the subsequent documentation commit is identified in the handoff. BRIEF-RULES.md, the round-one report and the FIX review were read before editing. Source descriptions are rung 2; executed tests and samples are rung 4 (isolated production paths), not installed-system verification.

Threat model:

1. A meeting's notes are silently never produced (skip too much).
2. Idle CPU creeps back through a path the harness does not cover (skip too little).
3. A retry after a real failure stops happening.

The first finding is addressed through the whole handoff: metadata revisions in the durable cursor and notes jobs; same-name discovery after restart; generation-aware enqueue and acceptance; fresh English rendering for changed input; replacement of the previous generation's summary; short-input reclassification; and preservation of explicit retry pins. Input keys use manifest/transcript metadata, including the Hex-original transcript when present, plus directory identity stamps. No transcript content is read to decide whether an acknowledged input changed. Legacy name-only receipts are replayed once to establish revisions; legacy jobs without revisions can regenerate on that replay.

The second finding is addressed by checking readiness before eligibility capture or launch. Parsed scheduling records are cached behind file metadata; readiness is reevaluated with the current time, and actual work claims still reread the locked job. The retained-Done fixture exposed further history costs. Scheduling was cached, duplicate metadata queries were removed, and the UI status batch now caches unchanged records while retaining the original bounded parser on cache misses. UI metadata enumeration reuses directory-entry metadata and checks roots and linked entries before payload loading. No polling interval, timeout, helper/model setting or priority was changed.

The suppressed steps are unchanged-input capture/classification/enqueue, not-yet-due or terminal-job eligibility capture, idle --notes-pending launches and their child history scans, and repeated unchanged status payload reads. Metadata enumeration and normal status polls remain; queued work and due retries still run.

### Rule 16: production reversion failures and passing runs

The discovery/generation regression enters Session/Discovery, the real listener completion handoff, enqueue and notes drain. It verifies same-name rewrite and restored-folder cases across restarts, including a retained Done job and an old summary/rendering without a retained job. A Busy attempt must still retry using the changed text and publish changed notes. Short-to-eligible and missing-output recovery are also exercised.

With the production name-only duplicate check restored, `round2-generation-final-red.log` ended with exit 101:

```text
changed_same_name_input_replaces_production_done_notes_after_restart ... FAILED
changed Done job was not queued: left Done { fallback: false, tasks_failed: false }, right Queued
same_name_rewrite_and_restore_are_delivered_after_restart ... FAILED
same-name input skipped, restore=false: left [], right ["100-1"]
test result: FAILED. 1 passed; 2 failed; 0 ignored
```

Restoring revision comparison (`round2-generation-final-green.log`, exit 0):

```text
changed_same_name_input_replaces_production_done_notes_after_restart ... ok
same_name_short_input_can_become_eligible_and_missing_done_output_recovers ... ok
same_name_rewrite_and_restore_are_delivered_after_restart ... ok
test result: ok. 3 passed; 0 failed; 0 ignored
```

The readiness tests create Queued, failed and Done jobs through production enqueue/drain, using the existing synthetic helper seam. Only the final spawn boundary is replaced with a launch counter. With the readiness check absent (`round2-worker-red.log`, exit 101):

```text
future retry launched early: left 2, right 1
retained Done: read_bytes=200869, launches=1
terminal notes launched a worker: left 1, right 0
test result: FAILED. 1 passed; 2 failed; 0 ignored
```

The final worker filter before the extra cache work (`round2-worker-final-green.log`, exit 0) passed both worker tests, with `retained Done: read_bytes=456, launches=0`. The stricter warmed-cache test then failed without the scheduling cache (`round2-cache-red.log`, exit 101):

```text
retained Done: read_bytes=456, launches=0
unchanged terminal job was reread: left 456, right 0
test result: FAILED. 1 passed; 1 failed; 0 ignored
```

With the scheduling cache (`round2-cache-green.log`, exit 0):

```text
worker_keeps_queued_and_due_failure_retries_but_defers_future_retry ... ok
retained Done: read_bytes=0, launches=0
worker_skips_production_done_jobs_without_capture_or_launch ... ok
test result: ok. 2 passed; 0 failed; 0 ignored
```

The retry test advances the production readiness gate to the persisted due time without changing the cached job, and separately drains a real backend failure to completion. The terminal test then requeues the completed job and requires launch, proving metadata invalidation.

The status-batch test uses a production-completed Done job and then requeues it. Before the status cache (`round2-status-red.log`, exit 101), the warmed batch reread 456 bytes and failed; the other four idle tests passed. After the cache (`round2-status-green.log`, exit 0):

```text
retained_done_status_batch_is_idle_and_observes_requeued_work ... retained Done status: read_bytes=0
worker_skips_production_done_jobs_without_capture_or_launch ... retained Done: read_bytes=0, launches=0
test result: ok. 5 passed; 0 failed; 0 ignored
```

### Conventional checks and commands

Every cargo command uses tools/local_build.ps1, the prescribed GNU directory first on PATH, and target x86_64-pc-windows-gnu. Tests use isolated state/meetings/peers paths and synthetic data, one test thread, and --no-fail-fast. Ordinary test builds are unoptimized with debug information; their durations are verification cost, not product speed.

The shared cargo test form is:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/local_build.ps1 test -p sidenote --target x86_64-pc-windows-gnu <selector> --no-fail-fast -- --nocapture --test-threads=1
```

| Selector / check | Result | Raw log |
| --- | --- | --- |
| --lib same_name, name-only reversion / restore | 1 passed, 2 failed / 3 passed, 0 failed | round2-generation-final-red.log / round2-generation-final-green.log |
| --lib core::notes::tests::idle::worker | 2 passed, 0 failed before added status test | round2-worker-final-green.log |
| --lib core::notes::tests, scheduling-cache head | 68 passed, 0 failed | round2-cache-notes.log |
| Whole library binary after scheduling cache | 557 passed, 0 failed, 1 ignored | round2-cache-whole.log |
| Whole library binary with status batch test | 558 passed, 0 failed, 1 ignored | round2-metadata-library-whole.log |
| --bin sidenote, scheduling-cache head | 123 passed, 0 failed, 2 ignored | round2-cache-ui-whole.log |
| --test idle_cpu, scheduling-cache head | 2 passed, 0 failed; source 272 read bytes, notes catch-up 2941 | round2-cache-integration.log |
| --lib recording_source | 22 passed, 0 failed | round2-final-source.log |
| --lib recording_guard | 10 passed, 0 failed | round2-final-guard.log |
| --lib listener::completion | 4 passed, 0 failed | round2-final-completion.log |
| FINAL whole --lib binary (513.06 s) | 558 passed, 0 failed, 1 ignored | round2-qualified-library-whole.log |
| FINAL --lib core::notes::tests | 69 passed, 0 failed | round2-qualified-notes.log |
| FINAL --lib recording_source | 22 passed, 0 failed | round2-qualified-source.log |
| FINAL --bin sidenote ui::refresh | 6 passed, 0 failed | round2-qualified-ui-refresh.log |
| FINAL whole --bin sidenote binary | 123 passed, 0 failed, 2 ignored | round2-qualified-ui-whole.log |
| FINAL --test idle_cpu | 2 passed, 0 failed; source 272 read bytes, notes 2941 | round2-qualified-integration.log |

All final cargo rows exited 0. An extra core::meeting_router::inputs selector matched zero tests (round2-qualified-inputs.log); it supplies no evidence. The whole library run covers the actual input tests. The final idle_cpu UI filter also passed 4 / failed 0 / ignored 1 (round2-metadata-ui-filter.log).

The whole-library executable was target/x86_64-pc-windows-gnu/debug/deps/sidenote-aa71e041b9fefb7f.exe, run with --test-threads=1. Full-binary runs without --nocapture still run every non-ignored test. The earlier full run found three compatibility failures (554 passed / 3 failed / 1 ignored): explicit retries losing a pin, a job retaining a pre-processing input stamp, and the expected absence of a job for a preserved summary. The first two were corrected. The last now asserts a Done receipt with zero attempts and no helper call while retaining the exact output-byte preservation check. Logs are retained, including the first failed generation attempt that exposed stale english-transcript.md reuse.

### Isolated CPU qualification

The opt-in release uses default shipping features plus idle-cpu-proof; opt-level s, thin LTO, one codegen unit and stripped symbols. The --listen samples use its ordinary recorder path. All directories and APPDATA are temporary, the Hex endpoint is synthetic loopback protocol 4 with idle capture/meeting and an event endpoint returning 404. No audio, real meeting, real Hex process, GPU or native window is involved. Samples retain normal priority and original polling cadence. The 300-meeting fixture has eligible synthetic transcripts and completes notes and task steps with a synthetic helper through production enqueue/drain, retaining every Done job and final output. Setup and settling are excluded from the fixed 30-second idle interval.

Windows Job Object accounting includes descendants that exit between samples. The final `python tools/idle_cpu_job.py` exited 0: 6 lifetime processes, 0.500000 total CPU seconds, 0.031250 root CPU seconds, 0.468750 descendant CPU seconds (round2-qualified-accounting.log). Its earlier self-test also passed. The job is assigned while the root is suspended, then resumed; fixture cleanup terminates only that job. Samples also require one lifetime process, so a worker launch before the interval cannot escape the count.

The recorder measurement script was tried against the round-one executable first: 30.000 s, 0.062500 CPU s, 0.208333% of one core, 84.572321% system load, 29 status requests, no child processes; exit 0 (round2-measurement-script-trial.log). The first UI-script trial against that same executable failed the existing native proof's 1% limit: 0.343750 CPU s / 30.217 s = 1.138%, load 99.987%, zero rebuilds (round2-ui-script-trial.log). It was not discarded.

| Configuration / build | Actual seconds | Root CPU seconds | Child CPU seconds | One-core CPU | System load | Result |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| 300 production Done jobs, readiness gate only | 30.000 | 0.593750 | 0 | 1.979167% | 89.884130% | FAIL |
| Same fixture, scheduling cache | 30.015 | 0.312500 | 0 | 1.041146% | 64.700521% | FAIL |
| Summary history diagnostic, scheduling cache | 30.047 | 0.406250 | 0 | 1.352048% | 95.874008% | FAIL |
| Headless UI with 300 production Done jobs, before batched metadata/status | 30.013 | 0.703125 | 0 | 2.343% | 65.710% | FAIL |

Each recorder row had 30 synthetic status requests and one lifetime process. The diagnostic had 0.015625 user / 0.390625 kernel CPU seconds, 73,566 read bytes and most of the burst in the periodic metadata scan. The UI row had zero rebuilds, 24 loop wakeups and 20 status calls. The first UI wrapper also reported 1.609375 CPU seconds over 31.922 seconds including teardown; that is not an idle figure. It is retained in the raw log. The wrapper now uses the production fixed-interval counters, keeps the original strict CPU assertion, and still rejects any descendant process.

Raw failed samples: round2-recorder-done-300.log, round2-recorder-done-300-after-cache.log, round2-recorder-metadata-diagnostic.log, round2-ui-done-300.log. Preserved executables: round2-before-cache-sidenote.exe (SHA-256 A99EBA2518BB4D796096B7E6BCBBF4CFA48F30174441D9B7081A10403724F493) and round2-before-status-batch-sidenote.exe (245B6BD6B39E370884744E79B03A2D592B9D651E28A58CCA88C80D02EFB083A5). Their successful optimized builds took 21m17s and 16m52s respectively; both retained the existing seven library warnings. An earlier opt-in build failed at the seed CLI's String/io::Error conversion, before any qualification launch; it is recorded in round2-release-build.log.

Final optimized metadata/status build: wrapper exit 0, 21m44s, seven existing library warnings; round2-qualified-release-build.log. Command: `powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/local_build.ps1 build --release --target x86_64-pc-windows-gnu --features idle-cpu-proof --bin sidenote`. Executable: artifacts/idle-cpu/round2-sidenote.exe, SHA-256 BAE13837F87C8A0DA23EF6B5BB660ACC4E3A763615F9DB580E8DC59E34C7DAC8. No source changed between this build and the final tests/samples.

| Final configuration | Seconds | Root CPU s | Child CPU s | One-core CPU | System load | Status calls |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Recorder, 300 production-completed retained Done jobs | 30.000 | 0.250000 | 0 | 0.833333% | 81.171875% | 30 |
| Headless UI, 300 production-completed retained Done jobs | 30.009 | 0.140625 | 0 | 0.468609% | 75.050% | 20 |
| Recorder, empty | 30.032 | 0.109375 | 0 | 0.364195% | 77.039157% | 30 |
| Recorder, 300 summaries present before catch-up | 30.000 | 0.250000 | 0 | 0.833333% | 74.140625% | 30 |

Every row exited 0, had one lifetime process and zero child launches. All 300 Done jobs and summaries were enumerated before the retained-job recorder sample; the UI fixture verifies every Done job/output after production drain. The summary-history sample now creates 300 Done receipts during catch-up. The UI had zero rebuilds, zero animation continuation requests and 24 loop wakeups. CPU includes every descendant; no audio or accuracy result is claimed. The UI's fixed interval excludes teardown. These are isolated headless results under the listed simultaneous load, not installed native-window or real-Hex latency guarantees.

Exact final script commands (run sequentially; exit 0 unless stated):

```powershell
python tools/idle_cpu_sample.py artifacts/idle-cpu/round2-sidenote.exe --mock --meetings 300 --done-jobs
python tools/idle_cpu_ui.py artifacts/idle-cpu/round2-sidenote.exe
python tools/idle_cpu_sample.py artifacts/idle-cpu/round2-sidenote.exe --mock
python tools/idle_cpu_sample.py artifacts/idle-cpu/round2-sidenote.exe --mock --meetings 300
python artifacts/idle-cpu/run_ui_proof.py artifacts/idle-cpu/round2-sidenote.exe
python tools/idle_cpu_job.py
```

Raw logs are round2-qualified-recorder-done-300.log, round2-qualified-ui-done-300.log, round2-qualified-recorder-empty.log, round2-qualified-recorder-summary-300.log and round2-qualified-accounting.log. Rule 11: every changed tools script has been executed in its final form; the separate diagnostic --detail run is retained as additional evidence.

The additional release functional command above exited 1 (round2-qualified-ui-functional.log): delayed empty poll FAIL, `left: 1, right: 0`; Hex state/meeting progress PASS; simulated idle PASS; unchanged refresh PASS; total 3 passed / 1 failed. The whole ordinary UI binary passed the same empty-poll test. Source inspection shows the production budget wake remains guarded by events > 0, unchanged in Round 2; whether asynchronous fixture events caused this one wake is not verified. No rerun was used to obtain a passing result and no assertion was relaxed. This extra round-one proof failure remains open for the lead/independent reviewer; the four fixed-interval CPU assertions above passed independently.

### Commit layers and limits

| Commit | Purpose | Parent-to-commit shortstat |
| --- | --- | --- |
| f1b4b71 | Durable metadata identity | 6 files changed, 179 insertions(+), 21 deletions(-) |
| 6159824 | Generation-aware notes and worker readiness | 5 files changed, 316 insertions(+), 14 deletions(-) |
| 72c16ef | Scheduling cache and due-time invalidation | 2 files changed, 115 insertions(+), 23 deletions(-) |
| 9946f37 | Reused metadata and batched status reads | 9 files changed, 199 insertions(+), 27 deletions(-) |
| 09dc5fc | Production Done fixtures and descendant CPU harness | 7 files changed, 360 insertions(+) |

This report is a separate documentation-only layer, force-added because artifacts are ignored. Its exact shortstat and final head are supplied in the handoff. No push or GitHub write has occurred.

Not verified: installed recorder/window CPU, native rendering/GPU, real Hex latency/accuracy or Ctrl+Space behavior, a real recording or re-recording, connected SSE, all-feature release check, Linux boundary check, packaging or installation. Metadata-only identity does not detect deliberate content edits preserving every tracked metadata field. No owner setting, installed app, scheduled task, Hex state/priority or owner recorder log was changed. Only the required build wrapper read its permitted recorder-log gate.

Work buckets: orientation read the supplied rules/review/source; edits cover only the two findings and their qualification; verification covers reversion tests, whole binaries and process-tree CPU; rework covers the stale rendering path, retry-pin/input-stamp compatibility, seed-CLI type conversion, retained-job payload cost and metadata/status history cost; waiting is dominated by compilation and wrapper queues. Largest avoidable waste: starting the first opt-in optimized build before type-checking the new seed CLI. The CPU failures were kept and used to select the next fix, not repeated until a pass appeared.

## Final narrow retry fix after round 2

Decisions needed: none. The final-review replacement-retry defect is fixed here; the delayed-empty-poll wake remains the documented open item above. This layer starts at 7ed6f56; its final head and shortstat are supplied in the handoff. No other production path was edited.

Enqueue carries an unfinished job's replace_summary flag into its explicit retry, including Codex recovery, while leaving fresh-input detection and transcript-pin copying separate. A successfully completed replacement does not force subsequent requests to replace its completed summary. The production-path test creates A's notes through enqueue/drain, rewrites the synthetic transcript to B, triggers HelperLimit, and explicitly retries with A's summary present. Both Try again and UseCodex then survive a second TimedOut failure with A's bytes unchanged and the failure visible; another retry invokes notes and task steps and publishes B. Input is synthetic transcript.ndjson; English rendering and synthetic summary/task helpers run, with no audio, ASR or translation service.

The sibling-path search covered preserve_summary/replace_summary in notes and meeting_job: enqueue, processing, rendering and publication use the same persisted flag; the retry defect is corrected at shared enqueue. Rung 4 red/green output:

```text
RED (original production code): retry accepted the old summary
left: Done { fallback: false, tasks_failed: false }; right: Queued
test result: FAILED. 0 passed; 1 failed; exit 101
GREEN: replacement explicit retry: recovery=false, old summary retained on failure, generation B published
GREEN: replacement explicit retry: recovery=true, old summary retained on failure, generation B published
test result: ok. 1 passed; 0 failed; exit 0
```

Commands use the prescribed GNU PATH and isolated temporary data/meetings/peers/APPDATA. Through tools/local_build.ps1: `test -p sidenote --target x86_64-pc-windows-gnu --lib <selector> --no-fail-fast -- --nocapture --test-threads=1`, selector explicit_retry_of_failed_replacement for red/green, core::notes::tests for the changed module, and omitted for the whole library binary. Logs: replacement-retry-red.log, replacement-retry-green.log, replacement-retry-notes.log, replacement-retry-whole.log. Ordinary unoptimized test build, no audio; runtime is verification cost.

Changed notes module: 70 passed / 0 failed, exit 0. Whole library binary: 559 passed / 0 failed / 1 ignored, exit 0, 300.68 seconds. Installed behavior, real helpers, release/CPU qualification on this final layer and release/Linux/packaging checks were not rerun; the prior CPU results identify the earlier executable. No installation, Hex/owner-state change, push or GitHub write occurred.
