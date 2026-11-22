<# Run cargo through the two local build slots, or use -Pack for release_pack.ps1. #>
# Plain $args, not a param block: an advanced script binds cargo flags such as -D and -- as
# PowerShell common parameters and fails before cargo runs.
$Pack = $false
$Arguments = @($args)
if ($Arguments.Count -and $Arguments[0] -ceq '-Pack') { $Pack = $true; $Arguments = @($Arguments | Select-Object -Skip 1) }
$ErrorActionPreference = 'Stop'

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
function BusyMeeting {
    try {
        # Match the Windows Hex process names in src/platform/windows.rs.
        $filter = "Name='hex.exe' OR Name='hex.com' OR Name='sidenote.exe' OR Name='hex-meetings.exe'"
        $processes = @(Get-CimInstance Win32_Process -Property Name,CommandLine -Filter $filter -ErrorAction Stop)
    } catch { return 'cannot check whether Hex is running' }
    $listener = $processes | Where-Object {
        $_.Name -in 'sidenote.exe','hex-meetings.exe' -and
        $_.CommandLine -match '(?:^|\s)"?--(?:listen|server)"?(?=\s|$)' -and
        $_.CommandLine -notmatch '(?:^|\s)"?--listener-isolated"?(?=\s|$)'
    }
    if ($listener) {
        try {
            $state = Join-Path $env:APPDATA 'sidenote'
            if (-not (Test-Path -LiteralPath $state -ErrorAction Stop)) { $state = Join-Path $env:APPDATA 'hex-meetings' }
            $statusPath = Join-Path $state 'local\status.ndjson'
            $status = Get-Content -LiteralPath $statusPath -Tail 1 -ErrorAction Stop | ConvertFrom-Json -ErrorAction Stop
            $age = ([DateTimeOffset](Get-Date) - [DateTimeOffset]$status.timestamp).TotalSeconds
            # The listener polls every three seconds, plus the status request time.
            if ($age -ge 0 -and $age -le 15 -and $status.protocol -eq 4 -and $null -eq $status.status_error) {
                switch -CaseSensitive -Regex ($status.meeting_state) {
                    '^(starting|recording|paused|stopping|finishing)$' { return 'Hex reports an active meeting' }
                    '^(idle|stopped|complete|finished|transcribing)$' { return }
                }
            }
        } catch { }
    }
    if ($processes.Name -match '^hex\.(exe|com)$') { return 'Hex is running and meeting status is unavailable' }
}
function Report-BuildWait([string]$Reason) {
    if ($waitClock.Elapsed -ge $script:nextWaitReport) {
        Write-Host "local_build: waiting: $Reason"
        $script:nextWaitReport = $waitClock.Elapsed.Add([TimeSpan]::FromMinutes(10))
    }
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
              else { $env:CARGO_BUILD_TARGET_DIR }
    if (-not $target) { return }
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
function Set-SharedTargetWorktree([string]$Directory) {
    $ownerFile = Join-Path $Directory '.sidenote-worktree'
    if ((Test-Path -LiteralPath $ownerFile) -and [IO.File]::ReadAllText($ownerFile) -ceq $worktree) { return }
    # Cargo compares path-package mtimes against the previous build, even across worktrees.
    $packages = [IO.File]::ReadAllText((Join-Path $worktree 'Cargo.lock')) -split '(?m)^\[\[package\]\]\s*$'
    $localNames = @($packages | ForEach-Object {
        if ($_ -notmatch '(?m)^source\s*=' -and $_ -match '(?m)^name\s*=\s*"([\w-]+)"') { $Matches[1] }
    })
    if (Test-Path -LiteralPath $Directory) {
        # Fingerprints live under each host profile and each target triple's profiles.
        $profiles = @(Get-ChildItem -LiteralPath $Directory -Directory)
        $profiles += @($profiles | ForEach-Object { Get-ChildItem -LiteralPath $_.FullName -Directory })
        foreach ($profile in $profiles) {
            $fingerprints = Join-Path $profile.FullName '.fingerprint'
            if (-not (Test-Path -LiteralPath $fingerprints)) { continue }
            foreach ($unit in Get-ChildItem -LiteralPath $fingerprints -Directory) {
                if ($unit.Name -match '^(.*)-[0-9a-f]{16}$' -and $Matches[1] -cin $localNames) {
                    # Removing Cargo's hash files makes it rebuild these units and their dependents.
                    Get-ChildItem -LiteralPath $unit.FullName -File |
                        Where-Object { $_.Name -match '^(lib|bin|test|example|bench|build-script|run-build-script|doc)-[^.]+$' } |
                        ForEach-Object { Remove-Item -LiteralPath $_.FullName }
                }
            }
        }
    } else {
        New-Item -ItemType Directory -Path $Directory -Force | Out-Null
    }
    [IO.File]::WriteAllText($ownerFile, $worktree)
}
function Assert-TargetSpace {
    $drive = New-Object IO.DriveInfo([IO.Path]::GetPathRoot($targetDirectory))
    if ($drive.AvailableFreeSpace -lt 40GB) { throw 'At least 40 GB free is required on the target drive.' }
}
if ($Arguments.Count -eq 1 -and $Arguments[0] -ceq '-MeetingStatus') {
    [bool](BusyMeeting)
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
    $sharedTarget = -not $targetDirectory
    if ($sharedTarget) {
        $worktree = (& git rev-parse --show-toplevel 2>$null)
        if (-not $worktree) { throw 'Run local_build from inside a git worktree.' }
        $worktree = [IO.Path]::GetFullPath($worktree)
        $sharedRoot = (Split-Path -Parent (Split-Path -Parent $lockPath)) + '-shared-target'
        $targetDirectory = $sharedRoot
    }
    while (-not $marker) {
        Wait-ReleaseLock $lockPath $ReleaseLockWaitMinutes $ReleaseLockReportSeconds $ReleaseLockPollSeconds
        $marker = Register-BuildMarker
        if (Get-BlockingReleaseLock $lockPath) { Unregister-BuildMarker $marker; $marker = $null }
    }
    Assert-TargetSpace
    New-Item -ItemType Directory -Force -Path $slotRoot | Out-Null
    (Get-Process -Id $PID).PriorityClass = 'BelowNormal'
    $waitClock = [Diagnostics.Stopwatch]::StartNew()
    $nextWaitReport = [TimeSpan]::FromMinutes(10)
    while (-not $slot) {
        while ($waitReason = BusyMeeting) {
            Report-BuildWait $waitReason
            Start-Sleep -Seconds 10
        }
        LockSlots
        try {
            $usedNumbers = @()
            foreach ($dir in @(Get-ChildItem -LiteralPath $slotRoot -Directory)) {
                $owners = @($dir.Name)
                $childFile = Join-Path $dir.FullName 'child'
                if (Test-Path -LiteralPath $childFile) { $owners += Get-Content -LiteralPath $childFile }
                if (-not @($owners | Where-Object { Alive $_ }).Count) {
                    Remove-Item -LiteralPath $dir.FullName -Recurse -Force
                } else {
                    $numberFile = Join-Path $dir.FullName 'number'
                    if (Test-Path -LiteralPath $numberFile) { $usedNumbers += [IO.File]::ReadAllText($numberFile) }
                }
            }
            $waitReason = 'both local build slots are occupied'
            if (@(Get-ChildItem -LiteralPath $slotRoot -Directory).Count -lt 2 -and -not ($waitReason = BusyMeeting)) {
                $number = if ('1' -in $usedNumbers) { '2' } else { '1' }
                $slot = Join-Path $slotRoot (Identity (Get-Process -Id $PID))
                New-Item -ItemType Directory -Path $slot | Out-Null
                [IO.File]::WriteAllText((Join-Path $slot 'number'), $number)
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
                if ($sharedTarget) { $targetDirectory = Join-Path $sharedRoot "slot-$number" }
                Assert-TargetSpace
                if ($sharedTarget) { Set-SharedTargetWorktree $targetDirectory }
                Write-Host "local_build: target directory: $targetDirectory"
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
        if (-not $slot) {
            Report-BuildWait $waitReason
            Start-Sleep -Seconds 5
        }
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
