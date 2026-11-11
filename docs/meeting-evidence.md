# Meeting evidence Lab

Decisions needed: none. Live performance is unverified until the owner runs a consented test call.

The background listener samples Hex status every three seconds, even when Sidenote has no open window. `local/status.ndjson` contains timestamp, Hex app and protocol versions, meeting and capture states, capture kind, id and elapsed presence flags, status error category and window visibility. It never stores titles, id values, elapsed values, audio, notes or transcript text. Rotation keeps one current file and one previous file, each at most 1 MiB. `listener::status::subscribe()` exposes start and stop transitions in the listener process for a later subscriber.

Settings > Diagnostics > Labs has **Process meetings in 5-minute windows (test)**, off by default. The listener reads both growing tracks with shared read access. It uses complete physical frames, including extensible float32 and PCM16, converts to 16 kHz mono and cuts near 300 seconds using the lowest energy in a two-second search on either side. The next window includes two seconds of preceding audio. The live cut waits until the search region is on disk. Missing or buffered headers wait for another poll.

A single worker groups a ready batch of up to four track windows by Telugu (`custom-9ac644a9878e7415`, `te`), then Hindi (`nemotron_3_5_asr`, `hi`). Each window uses silence-aware requests of at most 24.4 seconds, matching the owner script's request size. Per-clip receipts resume partial passes. It posts WAVs directly to Hex's loopback `/transcriptions` API without a proxy. Discovery is reread for each request. Tokens and response errors are never logged. The worker yields before new requests during capture, model preparation, unknown status, shutdown or Lab disablement. In-flight work may finish. Below-normal caller thread priority does not lower Hex's inference priority.

All measurement files stay in Sidenote's `local/evidence/<meeting-id>/`. Each track-window directory holds `job.json`, normalized `audio.wav`, model results `te.json` and `hi.json`, and `timing.json`. Timings name track and index, audio seconds including overlap, queue wait, per-model seconds in Telugu/Hindi order, and backlog counted as track windows. `lag.ndjson` samples elapsed versus the rolling header's `covered_ms`. It reads exactly that header line without buffering ahead into text. `summary.json` contains all completed window timings and totals after Stop. Missing tracks are listed in `collection.json`. A failed request leaves `error.json` and pending work for retry; it does not publish a success summary. Keep the listener running until the summary appears.

If Hex omits the meeting id, collection requires one unambiguous folder whose creation clock matches the status start within 90 seconds. An ambiguous match waits without reading another meeting's audio.

The collector waits for two stable file-length polls after confirmed Stop (transcribing or idle) before queuing the tail. It reconstructs saved ranges and resumes pending API work after restart. Timing and result receipts prevent completed calls from running twice. Audio and text results are local test artifacts and are not pruned automatically. Remove the isolated test folder when no longer needed. The Lab never publishes meeting notes or modifies the existing notes pipeline.

## Run a consented test without changing the installed pipeline

Do not replace the installed app for this test. The following PowerShell commands create a fresh Sidenote test data folder with notes disabled in that folder only and start the worktree build headlessly. They do not start or stop Hex, record a meeting, change Hex settings or read credentials. The normal installed listener can keep running. Hex's meeting-finished event Lab is not required for measurement.

```powershell
$evidenceExe = 'E:\Personal-Projects\hex-meetings\.worktrees\meeting-evidence\target\x86_64-pc-windows-gnu\debug\sidenote.exe'
$evidenceData = Join-Path $env:LOCALAPPDATA ('Sidenote\evidence-test-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Path $evidenceData -ErrorAction Stop | Out-Null
@{ version = 1; features = @{}; notes = @{ engine = 'off' }; labs = @{ process_meeting_windows = $true } } |
    ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $evidenceData 'config.json') -Encoding UTF8
$evidenceProcess = Start-Process -FilePath $evidenceExe -ArgumentList @('--listen', '--data-dir', ('"' + $evidenceData + '"')) -WindowStyle Hidden -PassThru
$evidenceData
```

1. Confirm the two specified models and the local API are already available in Hex. This build does not install models or change Hex settings.
2. Obtain consent from everyone on a test call. In Hex, start a recording using its normal controls. Use both microphone and system audio, include silence near five-minute boundaries, and continue for at least 11 minutes so there are full windows plus a partial tail.
3. While recording, check for new window folders under the printed data path's `local/evidence`. The process is headless; a visible Sidenote window is not needed. This isolated run reports `window_visible: false` unless its own UI is opened with the same data directory.
4. Stop the recording yourself in Hex. Leave the measurement listener running while it finishes. Check that `summary.json` exists and lists every expected window for each available track, including the final partial window. Inspect timings and header lag samples, rather than assuming completion means it kept pace.
5. Compare against a separate, similarly paced consented call with the Lab off before concluding it does not slow Hex. Compare live transcript lag, visible transcription/dictation responsiveness and any audio gaps. This Lab measures ASR and queue timing, not CPU/GPU utilization or audio quality.
6. After the summary is saved, stop only the test listener with `Stop-Process -Id $evidenceProcess.Id`. Keep or remove the isolated data folder as appropriate. The installed app configuration and the owner's scripts are unchanged.

The synthetic tests exercise file sharing and append visibility on this Windows toolchain, format conversion, whole-frame reads, boundaries, overlap, tail collection, durable ranges, grouped model calls through a fake transport, retries, summary completion and content-free status rotation. They do not establish real Hex throughput, exact installed-build status mapping, model quality or live latency.
