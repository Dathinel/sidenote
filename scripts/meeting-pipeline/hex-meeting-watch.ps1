param([string[]]$MeetingFolders)
$allowed = if ($PSBoundParameters.ContainsKey('MeetingFolders')) { @($MeetingFolders | ForEach-Object { Split-Path -Leaf $_ }) }
# Runs the English pipeline on each finished Hex meeting that has none yet. Started by
# hex-meeting-listen.py on Hex's "meeting finished" signal, and every 15 minutes as a backup by the
# "Hex meeting notes" scheduled task. Undo: Unregister-ScheduledTask "Hex meeting notes".
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'hex-pipeline-cli.ps1')
$root = Join-Path $env:APPDATA 'hex\meetings'
$log = Join-Path $env:LOCALAPPDATA 'hex-meeting-watch.log'
function Log($text) { Add-Content -LiteralPath $log -Value "$(Get-Date -Format s) $text" -Encoding utf8 }

$watcherClock = [Diagnostics.Stopwatch]::StartNew()
$failed = $null
try {
$mutex = New-Object Threading.Mutex($false, 'Local\HexMeetingWatch')
if (-not $mutex.WaitOne(0)) { exit 0 }
try {
    foreach ($dir in Get-ChildItem -LiteralPath $root -Directory | Sort-Object Name) {
        $failed = $null
        if ($null -ne $allowed -and $dir.Name -notin $allowed) { continue }
        $manifestPath = Join-Path $dir.FullName 'manifest.json'
        if (-not (Test-Path -LiteralPath $manifestPath)) { continue }
        if (Test-Path -LiteralPath (Join-Path $dir.FullName 'english-summary.json')) { continue }
        # Up to 3 tries, at least 15 minutes apart; then it waits for a manual retry.
        $failed = Join-Path $dir.FullName 'pipeline-failed.txt'
        if (Test-Path -LiteralPath $failed) {
            $tries = @(Get-Content -LiteralPath $failed | Where-Object { $_ -like 'try *' }).Count
            if ($tries -ge 3 -or ((Get-Date) - (Get-Item -LiteralPath $failed).LastWriteTime).TotalMinutes -lt 15) { continue }
        }
        $manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding utf8 | ConvertFrom-Json
        if ($manifest.status -ne 'complete') { continue }
        # About 4 hours of Hex audio is 1 GB per track; longer usually means a recording left running,
        # and loading it would need several GB of memory. Run those by hand.
        $wav = Get-ChildItem -LiteralPath $dir.FullName -Filter '*.wav' | Measure-Object Length -Maximum
        if ($wav.Maximum -gt 1GB) {
            if (-not (Test-Path -LiteralPath (Join-Path $dir.FullName 'pipeline-skipped.txt'))) {
                Set-Content -LiteralPath (Join-Path $dir.FullName 'pipeline-skipped.txt') -Value 'Longer than about 4 hours; run hex-meeting-english.ps1 by hand.' -Encoding utf8
                Log 'skipped: audio over 1 GB (about 4 hours)'
            }
            continue
        }
        $ndjson = Join-Path $dir.FullName 'transcript.ndjson'
        if (-not (Test-Path -LiteralPath $ndjson) -or (Get-Item -LiteralPath $ndjson).Length -eq 0) { continue }
        Log 'start'
        $pipelineClock = [Diagnostics.Stopwatch]::StartNew()
        try {
            & (Join-Path $PSScriptRoot 'hex-meeting-english.ps1') -MeetingDir $dir.FullName *>&1 | ForEach-Object {
                $line = "$_"
                if ($line -cmatch '\Apipeline-failure stage=(pipeline|prepare|transcribe|translate|notes|render|publish|tasks|compare|title) category=(timeout|exit|missing) deadline_seconds=([0-9]{1,4}) elapsed_seconds=([0-9]{1,5}\.[0-9]{3})\z') {
                    Log (Format-PipelineFailure $Matches[1] $Matches[2] ([int]$Matches[3]) ([double]::Parse($Matches[4], [Globalization.CultureInfo]::InvariantCulture)))
                }
                if ($line -match '^(Audio preparation|te transcription|hi transcription|Translation and notes|Total): ([0-9][0-9,.]*)s(?:;.*)?$') {
                    $label = $Matches[1]
                    $seconds = 0.0
                    if ([double]::TryParse($Matches[2], [Globalization.NumberStyles]::Number, [Globalization.CultureInfo]::CurrentCulture, [ref]$seconds)) {
                        Log ('pipeline-timing {0}: {1}s' -f $label,$seconds.ToString('0.0', [Globalization.CultureInfo]::InvariantCulture))
                        if ($dir.Name -cmatch '\A[0-9]+-[0-9]+\z') {
                            'pipeline-timing {0} {1}: {2}s' -f $dir.Name,$label,$seconds.ToString('0.0', [Globalization.CultureInfo]::InvariantCulture)
                        }
                    }
                }
            }
            Remove-Item -LiteralPath $failed -ErrorAction SilentlyContinue
            Log 'done'
        } catch {
            $safe = Get-PipelineFailure $_ 'pipeline' 0 $pipelineClock.Elapsed.TotalSeconds
            Add-Content -LiteralPath $failed -Value "try $(Get-Date -Format s): $safe" -Encoding utf8
            Log $safe
        }
    }
} finally { $mutex.ReleaseMutex() }

# Quick titles run after the main pass so a slow title never delays meeting notes.
# Failures only log.
# quick-title.failed.log (ignored by sync) stops a broken model call from retrying more than twice.
$failed = $null
$titleMutex = New-Object Threading.Mutex($false, 'Local\HexMeetingQuickTitle')
if ($titleMutex.WaitOne(0)) {
    try {
        foreach ($dir in Get-ChildItem -LiteralPath $root -Directory -ErrorAction SilentlyContinue | Where-Object { $_.LastWriteTime -gt (Get-Date).AddDays(-2) } | Sort-Object Name) {
            if ($null -ne $allowed -and $dir.Name -notin $allowed) { continue }
            $titleClock = [Diagnostics.Stopwatch]::StartNew()
            try {
                $quick = Join-Path $dir.FullName 'quick-title.json'
                $quickFailed = Join-Path $dir.FullName 'quick-title.failed.log'
                $ndjson = Join-Path $dir.FullName 'transcript.ndjson'
                if (-not (Test-Path -LiteralPath (Join-Path $dir.FullName 'manifest.json'))) { continue }
                if ((Test-Path -LiteralPath $quick) -or (Test-Path -LiteralPath (Join-Path $dir.FullName 'english-summary.json'))) { continue }
                if (-not (Test-Path -LiteralPath $ndjson) -or (Get-Item -LiteralPath $ndjson).Length -eq 0) { continue }
                if ((Test-Path -LiteralPath $quickFailed) -and @(Get-Content -LiteralPath $quickFailed).Count -ge 2) { continue }
                $m = Get-Content -LiteralPath (Join-Path $dir.FullName 'manifest.json') -Raw -Encoding utf8 | ConvertFrom-Json
                if ($m.status -ne 'complete') { continue }
                & (Join-Path $PSScriptRoot 'hex-meeting-quick-title.ps1') -MeetingDir $dir.FullName *>&1 | Out-Null
                if (-not (Test-Path -LiteralPath $quick)) { Add-Content -LiteralPath $quickFailed -Value "try $(Get-Date -Format s)" -Encoding utf8 }
            } catch { Log (Get-PipelineFailure $_ 'title' 60 $titleClock.Elapsed.TotalSeconds) }
        }
    } finally { $titleMutex.ReleaseMutex() }
}
} catch {
    $safe = Get-PipelineFailure $_ 'pipeline' 0 $watcherClock.Elapsed.TotalSeconds
    if ($failed) {
        try { Add-Content -LiteralPath $failed -Value "try $(Get-Date -Format s): $safe" -Encoding utf8 } catch {}
    }
    try { Log $safe } catch {}
    throw $safe
}
