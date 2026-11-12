<# Run cargo through the two local build slots, or use -Pack for release_pack.ps1. #>
param(
    [switch]$Pack,
    [Parameter(ValueFromRemainingArguments = $true)] [string[]]$Arguments
)
$ErrorActionPreference = 'Stop'
$slotRoot = Join-Path $env:LOCALAPPDATA 'sidenote-dev\build-slots'
$watchLog = Join-Path $env:LOCALAPPDATA 'hex-meeting-watch.log'
$mutex = New-Object Threading.Mutex($false, 'Local\SidenoteBuildSlots')
$slot = $null
$child = $null

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
try {
    if (-not $Pack -and -not $Arguments) { throw 'Supply cargo arguments or -Pack.' }
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
                if ($Pack) {
                    $exe = (Get-Command powershell.exe -ErrorAction Stop).Source
                    $runArgs = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $PSScriptRoot 'release_pack.ps1')) + $Arguments
                } else {
                    $exe = (Get-Command cargo.exe -ErrorAction Stop).Source
                    $runArgs = @($Arguments)
                    if (-not $env:CARGO_BUILD_JOBS -and -not @($Arguments | Where-Object { $_ -match '^(--jobs(?:=|$)|-j)' }).Count) {
                        $env:CARGO_BUILD_JOBS = '1'
                    }
                }
                $info = New-Object Diagnostics.ProcessStartInfo
                $info.FileName = $exe
                $info.Arguments = (@($runArgs | ForEach-Object { QuoteArg $_ }) -join ' ')
                $info.UseShellExecute = $false
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
    if ($slot -and (-not $child -or $child.HasExited)) {
        LockSlots
        try { Remove-Item -LiteralPath $slot -Recurse -Force -ErrorAction SilentlyContinue }
        finally { $mutex.ReleaseMutex() }
    }
    $mutex.Dispose()
}
