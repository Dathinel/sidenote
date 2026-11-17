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
$notesEngine = Get-ScriptsNotesEngine
if ($notesEngine -cne 'scripts') { Log (Format-NotesSkip $notesEngine); return }

$stateRoot = if ($env:SIDENOTE_DATA_DIR) { $env:SIDENOTE_DATA_DIR } elseif (Test-Path (Join-Path $env:APPDATA 'sidenote')) { Join-Path $env:APPDATA 'sidenote' } else { Join-Path $env:APPDATA 'hex-meetings' }
$ownedParts = @{}
$pendingStarts = @()
$partStore = Join-Path $stateRoot 'local/meeting-parts'
if (Test-Path -LiteralPath $partStore) {
    foreach ($file in Get-ChildItem -LiteralPath $partStore -Filter '*.json' -File) {
        $session = [IO.File]::ReadAllText($file.FullName) | ConvertFrom-Json
        if ($null -ne $session.pending_start_ms) { $pendingStarts += [long]$session.pending_start_ms }
        foreach ($part in $session.parts) { $ownedParts[[string]$part.id] = $true }
    }
}
function Test-PendingPart($Directory) {
    if ($null -ne $allowed -or $pendingStarts.Count -eq 0) { return $false }
    $manifest = Join-Path $Directory.FullName 'manifest.json'
    if (-not (Test-Path -LiteralPath $manifest)) { return $false }
    $created = ([IO.File]::ReadAllText($manifest) | ConvertFrom-Json).created_at_ms
    foreach ($since in $pendingStarts) {
        if ($null -ne $created -and [long]$created -ge $since -and [long]$created -le ($since + 25000)) { return $true }
    }
    return $false
}
$directories = if ($null -ne $allowed) { @($MeetingFolders | ForEach-Object { Get-Item -LiteralPath $_ }) } else { @(Get-ChildItem -LiteralPath $root -Directory) }

$watcherClock = [Diagnostics.Stopwatch]::StartNew()
$failed = $null
try {
$mutex = New-Object Threading.Mutex($false, 'Local\HexMeetingWatch')
if (-not $mutex.WaitOne(0)) { exit 0 }
try {
    foreach ($dir in $directories | Sort-Object Name) {
        $failed = $null
        if (($ownedParts.ContainsKey($dir.Name) -and $null -eq $allowed) -or (Test-PendingPart $dir)) { continue }
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
            $deferred = $false
            $notesOnly = Test-Path -LiteralPath (Join-Path $dir.FullName 'sidenote-parts.json')
            if ($notesOnly) {
                $parts=[IO.File]::ReadAllText((Join-Path $dir.FullName 'sidenote-parts.json')) | ConvertFrom-Json
                $joined=New-Object Text.StringBuilder
                $offset=0L
                foreach ($part in $parts) {
                    if ([string]$part.id -cnotmatch '\A[0-9][0-9-]{0,99}\z') { throw 'Invalid meeting part' }
                    $partFolder=Join-Path $root ([string]$part.id)
                    & (Join-Path $PSScriptRoot 'hex-meeting-english.ps1') -MeetingDir $partFolder -TranscriptOnly
                    $reading=Join-Path $partFolder 'english-transcript.md'
                    if (-not $? -or -not (Test-Path -LiteralPath $reading)) { throw 'Part transcript is not ready' }
                    foreach ($line in [IO.File]::ReadAllLines($reading,[Text.Encoding]::UTF8)) {
                        if ($line -match '^\[(\d+):(\d+):(\d+)(?:\.(\d+))?\](.*)$') {
                            $fraction=([string]$Matches[4]).PadRight(3,'0').Substring(0,3)
                            $ms=([long]$Matches[1]*3600000)+([long]$Matches[2]*60000)+([long]$Matches[3]*1000)+[long]$fraction+$offset
                            $seconds=[long][Math]::Floor($ms/1000)
                            $time='{0:00}:{1:00}:{2:00}.{3:000}' -f [Math]::Floor($seconds/3600),([Math]::Floor($seconds/60)%60),($seconds%60),($ms%1000)
                            [void]$joined.AppendLine("[$time]$($Matches[5])")
                        } elseif ($line.Trim() -and $line -notlike '# Full English transcript*') { [void]$joined.AppendLine($line) }
                    }
                    $offset += [long]$part.duration_ms
                }
                [IO.File]::WriteAllText((Join-Path $dir.FullName 'english-transcript.md'),$joined.ToString(),(New-Object Text.UTF8Encoding($false)))
            }
            & (Join-Path $PSScriptRoot 'hex-meeting-english.ps1') -MeetingDir $dir.FullName -NotesOnly:$notesOnly *>&1 | ForEach-Object {
                $line = "$_"
                if ($line -ceq 'notes deferred engine busy' -or $line -cmatch '\Anotes skipped (?:engine=(app|off)|category=(settings_invalid|settings_unavailable|query_unavailable))\z') { $deferred = $true; Log $line }
                if ($line -cmatch '\Anotes engine=scripts meeting=[0-9][0-9-]{0,99}\z') { Log $line }

                if ($line.StartsWith('pipeline-failure ')) {
                    Log (Get-PipelineFailure ([Management.Automation.ErrorRecord]::new([Exception]::new($line), 'Pipeline', 'NotSpecified', $null)))
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
            if ($deferred) { continue }
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
        foreach ($dir in $directories | Where-Object { $_.LastWriteTime -gt (Get-Date).AddDays(-2) } | Sort-Object Name) {
            if (($ownedParts.ContainsKey($dir.Name) -and $null -eq $allowed) -or (Test-PendingPart $dir)) { continue }
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
