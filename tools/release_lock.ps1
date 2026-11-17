<#
Lock shared by release_gate.ps1 and local_build.ps1. Dot-source it from the tools folder.
The lock file sits in the repository's common git dir, so every worktree sees the same file.
Line 1 is the holder's identity (PID-StartTicks), line 2 its start time.
#>
$ReleaseLockWaitMinutes = 120
$ReleaseLockReportSeconds = 60
$ReleaseLockPollSeconds = 5
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

function Wait-ReleaseLock([string]$Path, [double]$MaxMinutes, [int]$ReportSeconds, [int]$PollSeconds) {
    $deadline = [DateTime]::UtcNow.AddMinutes($MaxMinutes)
    $nextReport = [DateTime]::UtcNow
    while ($true) {
        $held = Read-ReleaseLock $Path
        if (-not $held -or -not $held.Alive) { return }
        if ($env:SIDENOTE_RELEASE_GATE_PID -ceq [string]$held.Pid) { return }
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
