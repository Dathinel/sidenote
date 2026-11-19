<# Run cargo through the two local build slots, or use -Pack for release_pack.ps1. #>
# Plain $args, not a param block: an advanced script binds cargo flags such as -D and -- as
# PowerShell common parameters and fails before cargo runs.
$Pack = $false
$Arguments = @($args)
if ($Arguments.Count -and $Arguments[0] -ceq '-Pack') { $Pack = $true; $Arguments = @($Arguments | Select-Object -Skip 1) }
$ErrorActionPreference = 'Stop'
$watchLog = Join-Path $env:LOCALAPPDATA 'hex-meeting-watch.log'

function Alive([string]$Identity) {
    if ($Identity -notmatch '^(\d+)-(\d+)$') { return $false }
    $process = Get-Process -Id ([int]$Matches[1]) -ErrorAction SilentlyContinue
    if (-not $process) { return $false }
    try { return $process.StartTime.ToUniversalTime().Ticks -eq [long]$Matches[2] }
    catch { return $true }
}
function Identity($Process) { return "$($Process.Id)-$($Process.StartTime.ToUniversalTime().Ticks)" }
function LockSlots {
    try { [void]$mutex.WaitOne() } catch [Threading.AbandonedMutexException] { }
}
function MeetingListenerRunning {
    try {
        foreach ($process in Get-CimInstance Win32_Process -Filter "Name='sidenote.exe' OR Name='hex-meetings.exe'" -ErrorAction Stop) {
            $command = $process.CommandLine
            if (-not $command) { return $true }
            if ($command -match '(?:^|\s)"?--(?:listen|server)"?(?=\s|$)' -and
                $command -notmatch '(?:^|\s)"?--listener-isolated"?(?=\s|$)') { return $true }
        }
        return $false
    } catch { return $true }
}
function BusyMeeting {
    $absent = $false
    try {
        $state = Join-Path $env:APPDATA 'sidenote'
        if (-not (Test-Path -LiteralPath $state -ErrorAction Stop)) { $state = Join-Path $env:APPDATA 'hex-meetings' }
        $statusPath = Join-Path $state 'local\status.ndjson'
        $status = Get-Content -LiteralPath $statusPath -Tail 1 -ErrorAction Stop | ConvertFrom-Json -ErrorAction Stop
        $age = ([DateTimeOffset]::UtcNow - [DateTimeOffset]$status.timestamp).TotalSeconds
        # The listener polls every three seconds, plus the status request time.
        if ($age -ge 0 -and $age -le 15 -and $status.protocol -eq 4 -and $null -eq $status.status_error) {
            switch -CaseSensitive -Regex ($status.meeting_state) {
                '^(starting|recording|paused|stopping|finishing)$' { return $true }
                '^(idle|stopped|complete|finished|transcribing)$' { return $false }
            }
        }
    } catch [Management.Automation.ItemNotFoundException] {
        $absent = $true
    } catch { }
    if (MeetingListenerRunning) { return $true }
    if (-not $absent) { return $false }
    if (-not (Test-Path -LiteralPath $watchLog)) { return $false }
    $item = Get-Item -LiteralPath $watchLog
    if ($item.LastWriteTimeUtc -lt [DateTime]::UtcNow.AddMinutes(-2)) { return $false }
    $last = Get-Content -LiteralPath $watchLog -Tail 1
    return $last -notmatch '^done\b'
}
function QuoteArg([string]$Value) {
    # Windows command-line quoting: preserve backslashes before quotes and at the end.
    return '"' + ([regex]::Replace($Value, '(\\*)"', '$1$1\"') -replace '(\\+)$', '$1$1') + '"'
}
function Resolve-TargetDirectory {
    if ($Pack) { return [IO.Path]::GetFullPath((Join-Path (Get-Location) 'target')) }
    $explicit = $null
    for ($i = 0; $i -lt $Arguments.Count; $i++) {
        $arg = [string]$Arguments[$i]
        if ($arg -ceq '--') { break }
        if ($arg -ceq '--config' -or $arg.StartsWith('--config=', [StringComparison]::Ordinal)) {
            throw 'Cargo --config overrides cannot be checked; use --target-dir or CARGO_TARGET_DIR instead.'
        }
        if ($arg -ceq '--target-dir') {
            $i++
            if ($i -ge $Arguments.Count -or -not $Arguments[$i] -or $Arguments[$i] -like '-*') { throw 'Supply a path after --target-dir.' }
            $explicit = [string]$Arguments[$i]
        } elseif ($arg.StartsWith('--target-dir=', [StringComparison]::Ordinal)) {
            $explicit = $arg.Substring('--target-dir='.Length)
            if (-not $explicit) { throw 'Supply a path after --target-dir=.' }
        }
    }
    $target = if ($explicit) { $explicit } elseif ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR }
              elseif ($env:CARGO_BUILD_TARGET_DIR) { $env:CARGO_BUILD_TARGET_DIR } else { Join-Path (Get-Location) 'target' }
    if (-not [IO.Path]::IsPathRooted($target)) { $target = Join-Path (Get-Location) $target }
    $target = [IO.Path]::GetFullPath($target)
    # Cargo hashes path dependencies relative to the workspace, so another worktree's build in
    # this directory looks fresh here and links stale code. Keep each target inside its own worktree.
    $top = (& git rev-parse --show-toplevel 2>$null)
    if (-not $top) { throw 'Run local_build from inside a git worktree.' }
    $top = [IO.Path]::GetFullPath($top).TrimEnd('\') + '\'
    if (-not $target.StartsWith($top, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Target directory $target is outside this worktree ($top); use a target inside it."
    }
    return $target
}
function Assert-TargetSpace {
    $drive = New-Object IO.DriveInfo([IO.Path]::GetPathRoot($targetDirectory))
    if ($drive.AvailableFreeSpace -lt 40GB) { throw 'At least 40 GB free is required on the target drive.' }
}
if ($Arguments.Count -eq 1 -and $Arguments[0] -ceq '-MeetingStatus') {
    BusyMeeting
    return
}
. (Join-Path $PSScriptRoot 'release_lock.ps1')
$slotRoot = Join-Path $env:LOCALAPPDATA 'sidenote-dev\build-slots'
$mutex = New-Object Threading.Mutex($false, 'Local\SidenoteBuildSlots')
$slot = $null
$child = $null
$marker = $null
try {
    if (-not $Pack -and -not $Arguments) { throw 'Supply cargo arguments or -Pack.' }
    $targetDirectory = Resolve-TargetDirectory
    $lockPath = Get-ReleaseLockPath
    while (-not $marker) {
        Wait-ReleaseLock $lockPath $ReleaseLockWaitMinutes $ReleaseLockReportSeconds $ReleaseLockPollSeconds
        $marker = Register-BuildMarker
        if (Get-BlockingReleaseLock $lockPath) { Unregister-BuildMarker $marker; $marker = $null }
    }
    Assert-TargetSpace
    New-Item -ItemType Directory -Force -Path $slotRoot | Out-Null
    (Get-Process -Id $PID).PriorityClass = 'BelowNormal'
    while (-not $slot) {
        while (BusyMeeting) { Start-Sleep -Seconds 10 }
        LockSlots
        try {
            foreach ($dir in @(Get-ChildItem -LiteralPath $slotRoot -Directory)) {
                $owners = @($dir.Name)
                $childFile = Join-Path $dir.FullName 'child'
                if (Test-Path -LiteralPath $childFile) { $owners += Get-Content -LiteralPath $childFile }
                if (-not @($owners | Where-Object { Alive $_ }).Count) {
                    Remove-Item -LiteralPath $dir.FullName -Recurse -Force
                }
            }
            if (@(Get-ChildItem -LiteralPath $slotRoot -Directory).Count -lt 2 -and -not (BusyMeeting)) {
                $slot = Join-Path $slotRoot (Identity (Get-Process -Id $PID))
                New-Item -ItemType Directory -Path $slot | Out-Null
                if (-not $env:CARGO_BUILD_JOBS -and ($Pack -or -not @($Arguments | Where-Object { $_ -match '^(--jobs(?:=|$)|-j)' }).Count)) {
                    $env:CARGO_BUILD_JOBS = '4'
                }
                if ($Pack) {
                    $exe = (Get-Command powershell.exe -ErrorAction Stop).Source
                    $runArgs = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $PSScriptRoot 'release_pack.ps1')) + $Arguments
                } else {
                    $exe = (Get-Command cargo.exe -ErrorAction Stop).Source
                    $runArgs = @($Arguments)
                }
                Assert-TargetSpace
                $info = New-Object Diagnostics.ProcessStartInfo
                $info.FileName = $exe
                $info.Arguments = (@($runArgs | ForEach-Object { QuoteArg $_ }) -join ' ')
                $info.WorkingDirectory = (Get-Location).Path
                $info.UseShellExecute = $false
                if (-not $Pack) {
                    # Pin config-file defaults and intermediate output to the checked directory.
                    $info.EnvironmentVariables['CARGO_TARGET_DIR'] = $targetDirectory
                    $info.EnvironmentVariables['CARGO_BUILD_TARGET_DIR'] = $targetDirectory
                    $info.EnvironmentVariables['CARGO_BUILD_BUILD_DIR'] = $targetDirectory
                }
                $child = [Diagnostics.Process]::Start($info)
                [IO.File]::WriteAllText((Join-Path $slot 'child'), (Identity $child))
            }
        } finally { $mutex.ReleaseMutex() }
        if (-not $slot) { Start-Sleep -Seconds 5 }
    }
    $child.WaitForExit()
    exit $child.ExitCode
} catch {
    [Console]::Error.WriteLine(('local_build: ' + $_.Exception.Message -replace '[\r\n]+', ' '))
    exit 1
} finally {
    Unregister-BuildMarker $marker
    if ($slot -and (-not $child -or $child.HasExited)) {
        LockSlots
        try { Remove-Item -LiteralPath $slot -Recurse -Force -ErrorAction SilentlyContinue }
        finally { $mutex.ReleaseMutex() }
    }
    $mutex.Dispose()
}
