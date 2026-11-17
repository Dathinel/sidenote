<#
Lock shared by release_gate.ps1 and local_build.ps1. Dot-source it from the tools folder.
The lock file sits in the repository's common git dir, so every worktree sees the same file.
Line 1 is the holder's identity (PID-StartTicks), line 2 its start time.
#>
$ReleaseLockWaitMinutes = 120
$ReleaseLockReportSeconds = 60
$ReleaseLockPollSeconds = 5
$RunningBuildWaitMinutes = 120
$BuildMarkerDirectoryName = 'sidenote-builds'
$ReleaseLockFileName = 'sidenote-release-gate.lock'
$ReleaseLockUnreadableGraceSeconds = 10

function Get-ReleaseLockPath {
    $common = (& git rev-parse --path-format=absolute --git-common-dir 2>$null)
    if ($LASTEXITCODE -ne 0 -or -not $common) { throw 'release lock: run this from inside a git worktree.' }
    return Join-Path $common $ReleaseLockFileName
}

function Get-ProcessIdentity([int]$ProcessId) {
    $process = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
    if (-not $process) { return $null }
    try { return "$ProcessId-$($process.StartTime.ToUniversalTime().Ticks)" }
    catch { return "$ProcessId-unknown" }
}

function Read-ReleaseLock([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return $null }
    $text = $null
    try {
        $stream = [IO.File]::Open($Path, 'Open', 'Read', 'ReadWrite, Delete')
        try { $text = (New-Object IO.StreamReader($stream)).ReadToEnd() } finally { $stream.Dispose() }
    } catch { return $null }
    $lines = @($text -split '\r?\n')
    $started = if ($lines.Count -gt 1) { $lines[1] } else { '' }
    if ($lines[0] -notmatch '^(\d+)-(\d+|unknown)$') {
        $age = ([DateTime]::UtcNow - (Get-Item -LiteralPath $Path).LastWriteTimeUtc).TotalSeconds
        return [pscustomobject]@{ Pid = 0; Started = $started; Alive = ($age -lt $ReleaseLockUnreadableGraceSeconds) }
    }
    $holderPid = [int]$Matches[1]
    $now = Get-ProcessIdentity $holderPid
    $alive = $false
    if ($now) { $alive = ($now -ceq $lines[0]) -or ($Matches[2] -ceq 'unknown') -or $now.EndsWith('-unknown') }
    return [pscustomobject]@{ Pid = $holderPid; Started = $started; Alive = $alive }
}

function Enter-ReleaseLock([string]$Path) {
    $identity = Get-ProcessIdentity $PID
    $started = (Get-Date).ToString('yyyy-MM-dd HH:mm:ss zzz')
    for ($attempt = 0; $attempt -lt 5; $attempt++) {
        try {
            $stream = [IO.File]::Open($Path, 'CreateNew', 'Write', 'Read')
        } catch [IO.IOException] {
            $held = Read-ReleaseLock $Path
            if ($held -and $held.Alive) { return [pscustomobject]@{ Stream = $null; Busy = $held } }
            if ($held) {
                Write-Host "release_gate: stale release lock (PID $($held.Pid), started $($held.Started)) is gone; taking it over."
            }
            Remove-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue
            continue
        }
        $writer = New-Object IO.StreamWriter($stream)
        $writer.WriteLine($identity)
        $writer.WriteLine($started)
        $writer.Flush()
        return [pscustomobject]@{ Stream = $stream; Busy = $null }
    }
    throw "release lock: could not take $Path after 5 attempts."
}

function Exit-ReleaseLock($Stream, [string]$Path) {
    if ($Stream) { $Stream.Dispose() }
    Remove-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue
}

function Get-BlockingReleaseLock([string]$Path) {
    $held = Read-ReleaseLock $Path
    if (-not $held -or -not $held.Alive) { return $null }
    if ($env:SIDENOTE_RELEASE_GATE_PID -ceq [string]$held.Pid) { return $null }
    return $held
}

function Get-BuildMarkerDirectory {
    $common = (& git rev-parse --path-format=absolute --git-common-dir 2>$null)
    if ($LASTEXITCODE -ne 0 -or -not $common) { throw 'build marker: run this from inside a git worktree.' }
    return Join-Path $common $BuildMarkerDirectoryName
}

function Register-BuildMarker {
    $directory = Get-BuildMarkerDirectory
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
    $identity = Get-ProcessIdentity $PID
    $path = Join-Path $directory "$identity.build"
    Set-Content -LiteralPath $path -Value @($identity, (Get-Date).ToString('yyyy-MM-dd HH:mm:ss zzz'))
    return $path
}

function Unregister-BuildMarker([string]$Path) {
    if ($Path) { Remove-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue }
}

function Get-LiveBuildMarkers([int]$ExceptPid) {
    $directory = Get-BuildMarkerDirectory
    if (-not (Test-Path -LiteralPath $directory)) { return @() }
    $live = @()
    foreach ($file in @(Get-ChildItem -LiteralPath $directory -Filter '*.build' -File)) {
        $lines = @()
        try {
            $stream = [IO.File]::Open($file.FullName, 'Open', 'Read', 'ReadWrite, Delete')
            try { $lines = @((New-Object IO.StreamReader($stream)).ReadToEnd() -split '\r?\n') } finally { $stream.Dispose() }
        } catch { continue }
        if ($lines[0] -match '^(\d+)-(\d+|unknown)$') {
            $now = Get-ProcessIdentity ([int]$Matches[1])
            $alive = $now -and (($now -ceq $lines[0]) -or ($Matches[2] -ceq 'unknown') -or $now.EndsWith('-unknown'))
            if (-not $alive) {
                Remove-Item -LiteralPath $file.FullName -Force -ErrorAction SilentlyContinue
            } elseif ([int]$Matches[1] -ne $ExceptPid) {
                $live += [pscustomobject]@{ Pid = [int]$Matches[1]; Started = $(if ($lines.Count -gt 1) { $lines[1] } else { '' }) }
            }
        } elseif (([DateTime]::UtcNow - $file.LastWriteTimeUtc).TotalSeconds -ge $ReleaseLockUnreadableGraceSeconds) {
            Remove-Item -LiteralPath $file.FullName -Force -ErrorAction SilentlyContinue
        }
    }
    return $live
}

function Wait-RunningBuilds([double]$MaxMinutes, [int]$ReportSeconds, [int]$PollSeconds) {
    $deadline = [DateTime]::UtcNow.AddMinutes($MaxMinutes)
    $nextReport = [DateTime]::UtcNow
    while ($true) {
        $running = @(Get-LiveBuildMarkers $PID)
        if (-not $running.Count) { return }
        if ([DateTime]::UtcNow -ge $deadline) {
            throw "local build(s) still running after $MaxMinutes minutes (PID $(($running | ForEach-Object { $_.Pid }) -join ', ')); the release check did not start."
        }
        if ([DateTime]::UtcNow -ge $nextReport) {
            Write-Host "release_gate: waiting for $($running.Count) running local build(s) to finish (PID $(($running | ForEach-Object { $_.Pid }) -join ', '), oldest started $($running[0].Started))."
            $nextReport = [DateTime]::UtcNow.AddSeconds($ReportSeconds)
        }
        Start-Sleep -Seconds $PollSeconds
    }
}

function Wait-ReleaseLock([string]$Path, [double]$MaxMinutes, [int]$ReportSeconds, [int]$PollSeconds) {
    $deadline = [DateTime]::UtcNow.AddMinutes($MaxMinutes)
    $nextReport = [DateTime]::UtcNow
    while ($true) {
        $held = Get-BlockingReleaseLock $Path
        if (-not $held) { return }
        if ([DateTime]::UtcNow -ge $deadline) {
            throw "a release check (PID $($held.Pid), started $($held.Started)) still holds the build lock after $MaxMinutes minutes; no cargo was started."
        }
        if ([DateTime]::UtcNow -ge $nextReport) {
            Write-Host "local_build: waiting for the release check (PID $($held.Pid), started $($held.Started)) to finish."
            $nextReport = [DateTime]::UtcNow.AddSeconds($ReportSeconds)
        }
        Start-Sleep -Seconds $PollSeconds
    }
}
