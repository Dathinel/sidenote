$ErrorActionPreference = 'Stop'
$wrapper = Join-Path $PSScriptRoot 'install_release.ps1'
$root = Join-Path ([IO.Path]::GetTempPath()) ('sidenote-install-tests-' + [guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($root) | Out-Null
. $wrapper -Feed $root -Version '0.0.1-dev.29' -Log (Join-Path $root 'setup.log')
$realHidden = ${function:Run-Hidden}
& $realHidden (Get-Command powershell.exe).Source @('-NoProfile', '-NonInteractive', '-Command', 'exit 0')
$failed = $false
try { & $realHidden (Get-Command powershell.exe).Source @('-NoProfile', '-NonInteractive', '-Command', 'exit 7') }
catch { $failed = $true }
if (-not $failed) { throw 'Native child failure was accepted' }
function Get-InstallPaths { return $script:case.Paths }
function hex.com {
    $script:case.HexCalls++
    $global:LASTEXITCODE = 0
    if ($script:case.HexCalls -eq 2) {
        if ($script:case.Mode -eq 'late-notes') {
            [IO.File]::WriteAllText((Join-Path $script:case.Paths.State 'local/notes-jobs/100-1.json'), '{"state":"queued"}')
        }
        if ($script:case.Mode -eq 'late-router') {
            [IO.File]::WriteAllText((Join-Path $script:case.Paths.State 'local/router-queue/100-1.json'), '{}')
        }
    }
    $meeting = if ($script:case.Mode -eq 'hex-busy' -or ($script:case.Mode -like 'late-hex*' -and $script:case.HexCalls -eq 2)) { 'recording' } else { 'idle' }
    $jobs = if ($script:case.Mode -eq 'late-capture' -and $script:case.HexCalls -eq 2) { 1 } else { 0 }
    return (@{meeting=@{state=$meeting}; capture=@{pending_jobs=$jobs}} | ConvertTo-Json -Compress)
}
function Start-ScheduledTask { $script:case.TaskStarts++ }
function Get-CimInstance {
    if ($script:case.Mode -eq 'no-listener') { return }
    [pscustomobject]@{ ExecutablePath=$script:case.Paths.Exe; CommandLine='sidenote.exe --listen'; ProcessId=123 }
}
function Invoke-InstallCheck([string]$Expected) {
    $script:case.Checked++
    if ($Expected -ne '0.0.1-dev.29' -or $script:case.Mode -eq 'check-fails') { throw 'Checker refused' }
}
function Run-Hidden([string]$Exe, [string[]]$Arguments, [scriptblock]$Started = {}) {
    if ([IO.Path]::GetFileName($Exe) -eq 'sidenote-win-Setup.exe') {
        if ($script:case.HexCalls -ne 2) { throw 'Setup did not get two Hex checks' }
        # The real OS reservation must cover process creation, then be released for hooks.
        $blocked = $false
        try { $file=Open-InstallLock (Join-Path $script:case.Paths.Updates 'update-activity.lock'); $file.Dispose() }
        catch { $blocked = $true }
        if (-not $blocked) { throw 'Setup started without its activity reservation' }
        $stop = [IO.File]::ReadAllText((Join-Path $script:case.Paths.Updates 'update-listener.stop')) | ConvertFrom-Json
        if ($stop.pid -ne $PID) { throw 'Listener stop handoff missing' }
        $script:case.Setup = $true
        & $Started
        $file=Open-InstallLock (Join-Path $script:case.Paths.Updates 'update-activity.lock'); $file.Dispose()
        if ($script:case.Mode -eq 'setup-fails') { throw 'Setup failed' }
    } else {
        if ($Arguments[0] -ne '--veloapp-updated' -or $Arguments[1] -ne '0.0.1-dev.29') { throw 'Wrong hook invocation' }
        if ($script:case.Mode -eq 'hook-fails') { throw 'Hook failed' }
    }
    # Simulate the real reconciler retiring the existing handoff markers.
    Remove-Item -LiteralPath (Join-Path $script:case.Paths.Updates 'update-listener.stop') -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath (Join-Path $script:case.Paths.Updates 'update-listener.resume') -ErrorAction SilentlyContinue
}
function Trial([string]$Mode, [string]$ExpectedStage, [string]$Notes = '', [string]$Held = '') {
    $folder = Join-Path $root $Mode
    $script:case = @{ Mode=$Mode; HexCalls=0; Setup=$false; TaskStarts=0; Checked=0
        Paths=@{ State=(Join-Path $folder 'state'); Updates=(Join-Path $folder 'updates'); Exe=(Join-Path $folder 'sidenote.exe') } }
    $Feed = $folder
    $Log = Join-Path $folder 'setup.log'
    foreach ($dir in @($folder, $script:case.Paths.Updates,
        (Join-Path $script:case.Paths.State 'local/notes-jobs'), (Join-Path $script:case.Paths.State 'local/router-queue'))) {
        [IO.Directory]::CreateDirectory($dir) | Out-Null
    }
    [IO.File]::WriteAllText((Join-Path $folder 'sidenote-win-Setup.exe'), 'synthetic')
    [IO.File]::WriteAllText($script:case.Paths.Exe, 'synthetic')
    if ($Notes) { [IO.File]::WriteAllText((Join-Path $script:case.Paths.State 'local/notes-jobs/100-1.json'), $Notes) }
    if ($Mode -eq 'router-pending') { [IO.File]::WriteAllText((Join-Path $script:case.Paths.State 'local/router-queue/100-1.json'), '{}') }
    if ($Mode -eq 'other-install') { Write-NewMarker (Join-Path $script:case.Paths.Updates 'update-listener.stop') '{"pid":999999,"created":0}' }
    if ($Mode -eq 'late-hex-existing-resume') { Write-NewMarker (Join-Path $script:case.Paths.Updates 'update-listener.resume') 'existing-resume' }
    $heldFile = $null
    if ($Held) {
        $base = if ($Held -like 'update-*') { $script:case.Paths.Updates } else { $script:case.Paths.State }
        $heldFile = Open-InstallLock (Join-Path $base $Held)
    }
    $refused = $false
    $elapsed = [Diagnostics.Stopwatch]::StartNew()
    try { Invoke-ReleaseInstall *> $null }
    catch { $refused = $true }
    finally { if ($heldFile) { $heldFile.Dispose() } }
    if ($ExpectedStage) {
        if (-not $refused -or $script:stage -notlike "$ExpectedStage*") { throw "Wrong refusal stage for $Mode" }
    } elseif ($refused -or -not $script:case.Setup -or $script:case.Checked -ne 1) { throw "Success case failed: $Mode" }
    if ($ExpectedStage -and $ExpectedStage -notin @('silent Setup: require exit 0', 'after-update hook', 'listener start', 'install_check') -and $script:case.Setup) {
        throw "Busy preflight reached Setup: $Mode"
    }
    if (($Mode -like 'late-*' -or $Mode -eq 'held-listener') -and ($script:case.TaskStarts -ne 1 -or
        (Test-Path -LiteralPath (Join-Path $script:case.Paths.Updates 'update-listener.stop')) -or
        ($Mode -ne 'late-hex-existing-resume' -and (Test-Path -LiteralPath (Join-Path $script:case.Paths.Updates 'update-listener.resume'))))) { throw 'Pre-Setup rollback failed' }
    if ($Mode -eq 'late-hex-existing-resume' -and [IO.File]::ReadAllText((Join-Path $script:case.Paths.Updates 'update-listener.resume')) -ne 'existing-resume') { throw 'Existing resume state was changed' }
    if ($Mode -eq 'other-install' -and -not (Test-Path -LiteralPath (Join-Path $script:case.Paths.Updates 'update-listener.stop'))) { throw 'Another install marker was removed' }
    if ($Mode -eq 'no-listener' -and $elapsed.Elapsed.TotalSeconds -gt 33) { throw 'Listener wait exceeded its bound' }
    if ($Mode -eq 'held-listener' -and $elapsed.Elapsed.TotalSeconds -gt 11) { throw 'Listener stop exceeded its bound' }
    $file=Open-InstallLock (Join-Path $script:case.Paths.Updates 'update-activity.lock'); $file.Dispose()
    Write-Output "PASS $Mode"
}
try {
    Trial 'success' ''
    Trial 'completed-notes' '' '{"state":"done","sorting_pending":false}'
    Trial 'notes-queued' 'Sidenote idle check' '{"state":"queued"}'
    Trial 'notes-running' 'Sidenote idle check' '{"state":"running"}'
    Trial 'notes-retry' 'Sidenote idle check' '{"state":"backend_error","next_try_ms":123}'
    Trial 'notes-sort' 'Sidenote idle check' '{"state":"done","sorting_pending":true}'
    Trial 'notes-invalid' 'Sidenote idle check' 'invalid-json'
    Trial 'notes-unknown' 'Sidenote idle check' '{"state":"unknown"}'
    Trial 'router-pending' 'Sidenote idle check'
    foreach ($lock in @('update-activity.lock','update-window.lock','update-check.lock','local/notes-jobs/worker.lock','local/meeting-router.lock','local/notes-script.lock')) {
        Trial ('held-' + $lock.Replace('/','-')) 'Sidenote idle check' '' $lock
    }
    Trial 'hex-busy' 'Hex idle check'
    Trial 'late-hex' 'final Hex idle check'
    Trial 'late-capture' 'final Hex idle check'
    Trial 'late-notes' 'final Sidenote idle check'
    Trial 'late-router' 'final Sidenote idle check'
    Trial 'late-hex-existing-resume' 'final Hex idle check'
    Trial 'held-listener' 'listener handoff' '' 'update-listener.lock'
    Trial 'other-install' 'listener handoff'
    Trial 'setup-fails' 'silent Setup: require exit 0'
    Trial 'hook-fails' 'after-update hook'
    Trial 'check-fails' 'install_check'
    Trial 'no-listener' 'listener start'
    & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $wrapper -Feed synthetic -Version 0.0.1-dev.29 -Log synthetic.log -WhatIf
    if ($LASTEXITCODE -ne 0) { throw 'Dry run failed' }
    Write-Output 'PASS native child exits, all 27 isolated install cases, and wrapper dry run'
} finally {
    $resolved = [IO.Path]::GetFullPath($root)
    $temp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    if (-not $resolved.StartsWith($temp, [StringComparison]::OrdinalIgnoreCase) -or
        [IO.Path]::GetFileName($resolved) -notlike 'sidenote-install-tests-*') { throw 'Test cleanup escaped its temp root' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
