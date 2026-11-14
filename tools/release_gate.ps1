<#
Release gate for local Sidenote releases.

Run from the root of a release worktree:
  powershell -NoProfile -ExecutionPolicy Bypass -File tools\release_gate.ps1 [-GnuBin <dir>]

It refuses to run from anywhere else, always builds into this worktree's own
target folder, installs relay dependencies first, then runs the fixed check
list once. It writes artifacts\release-gate-<commit>.md with a pass table,
links full logs in artifacts\release-gate-<commit>\, and exits 1 if any check fails.
#>
param(
    [string]$GnuBin = 'E:\Personal-Projects\tools\hex-meetings-gnu\w64devkit\bin'
)
$ErrorActionPreference = 'Continue'

$root = (Get-Location).Path
$top = (git rev-parse --show-toplevel 2>$null)
if (-not $top -or ((Resolve-Path $top).Path.TrimEnd('\') -ne $root.TrimEnd('\'))) {
    Write-Output 'release_gate: run this from the root of the release worktree.'
    exit 2
}
if (-not (Test-Path (Join-Path $root 'Cargo.toml')) -or -not (Test-Path (Join-Path $root 'relay'))) {
    Write-Output 'release_gate: Cargo.toml or relay\ is missing; this is not a Sidenote worktree.'
    exit 2
}
if (git status --porcelain) {
    Write-Output 'release_gate: the worktree has uncommitted changes; commit them first.'
    exit 2
}
if (Test-Path $GnuBin) { $env:PATH = "$GnuBin;" + $env:PATH }
# Always this worktree's own build folder. A shared folder can hold another worktree's exe.
$env:CARGO_TARGET_DIR = Join-Path $root 'target'
$commit = (git rev-parse --short HEAD)
$version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$started = Get-Date
$artifacts = Join-Path $root 'artifacts'
$logDirectory = Join-Path $artifacts "release-gate-$commit"
New-Item -ItemType Directory -Force $logDirectory | Out-Null

$rows = New-Object System.Collections.Generic.List[object]
function Gate([string]$Name, [scriptblock]$Run) {
    $t0 = Get-Date
    $log = Join-Path $logDirectory "$Name.log"
    # Capture OS handles too: local_build's cargo child bypasses PowerShell streams.
    $command = '$ErrorActionPreference = "Continue"; & { ' + $Run.ToString() + ' }; exit $LASTEXITCODE'
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
    $process = Start-Process -FilePath (Get-Command powershell.exe).Source `
        -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-OutputFormat', 'Text', '-EncodedCommand', $encoded) `
        -WorkingDirectory (Get-Location).Path -WindowStyle Hidden `
        -RedirectStandardOutput "$log.stdout" -RedirectStandardError "$log.stderr" -Wait -PassThru
    $code = $process.ExitCode
    $output = [IO.File]::Create($log)
    try {
        foreach ($path in @("$log.stdout", "$log.stderr")) {
            $inputStream = [IO.File]::OpenRead($path)
            try { $inputStream.CopyTo($output) } finally { $inputStream.Dispose() }
        }
    } finally { $output.Dispose() }
    Remove-Item -LiteralPath "$log.stdout", "$log.stderr"
    Get-Content -LiteralPath $log
    $secs = [int]((Get-Date) - $t0).TotalSeconds
    $rows.Add([pscustomobject]@{ Check = $Name; Exit = $code; Seconds = $secs; Detail = '' })
    Write-Output "=== $Name exit=$code (${secs}s)"
}

function RecorderSnapshot([string]$Name) {
    $t0 = Get-Date
    $state = 'unknown'
    $pids = @()
    try {
        if (-not $env:LOCALAPPDATA) { throw 'LocalAppData is unavailable' }
        $installedExe = Join-Path $env:LOCALAPPDATA 'Sidenote\current\sidenote.exe'
        $candidates = @(Get-CimInstance Win32_Process -Filter "Name = 'sidenote.exe'" -ErrorAction Stop)
        $uncertain = $false
        foreach ($candidate in $candidates) {
            if (-not $candidate.ExecutablePath) { $uncertain = $true; continue }
            if (-not [string]::Equals($candidate.ExecutablePath, $installedExe, [StringComparison]::OrdinalIgnoreCase)) { continue }
            if (-not $candidate.CommandLine) { $uncertain = $true; continue }
            if ($candidate.CommandLine -match '(?i)(?:^|\s)"?--listen"?(?=\s|$)') { $pids += $candidate.ProcessId }
        }
        if ($pids.Count) { $state = 'running' }
        elseif (-not $uncertain) { $state = 'stopped' }
    } catch {
        # No raw process command lines or exception text in the release report.
        $state = 'unknown'
    }
    $code = if ($state -eq 'unknown') { 1 } else { 0 }
    $detail = if ($pids.Count) { "$state (PID $($pids -join ', '))" } else { $state }
    $detail | Set-Content -LiteralPath (Join-Path $logDirectory "$Name.log")
    $rows.Add([pscustomobject]@{ Check = $Name; Exit = $code; Seconds = [int]((Get-Date) - $t0).TotalSeconds; Detail = $detail })
    return [pscustomobject]@{ State = $state }
}

$listenerStart = RecorderSnapshot 'listener-start'
Write-Output "=== listener-start $($listenerStart.State)"

Push-Location (Join-Path $root 'relay')
Gate 'relay-deps' { bun install --frozen-lockfile }
Pop-Location
Gate 'pipeline-tests' { python tools/pipeline-tests.py }
Gate 'linux-boundaries' { python tools/prove_linux_boundaries.py . --self-test }
Gate 'pipeline-install-dryrun' { python tools/pipeline-install-dryrun.py }
Gate 'build-arguments' { powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/local-build-tests.ps1 }
Gate 'fmt' { & tools/local_build.ps1 +stable-x86_64-pc-windows-gnu fmt --check }
Gate 'clippy' { & tools/local_build.ps1 +stable-x86_64-pc-windows-gnu clippy --locked --all-targets -j 1 '--' -D warnings }
Gate 'clippy-asr' { & tools/local_build.ps1 +stable-x86_64-pc-windows-gnu clippy --locked --all-targets --features asr-runtime -j 1 '--' -D warnings }
Gate 'test' { & tools/local_build.ps1 +stable-x86_64-pc-windows-gnu test --locked --no-fail-fast -j 1 --quiet }
Gate 'test-asr' { & tools/local_build.ps1 +stable-x86_64-pc-windows-gnu test --locked --no-fail-fast -j 1 --quiet --features asr-runtime }
(Get-Item (Join-Path $root 'src\main.rs')).LastWriteTime = Get-Date
Gate 'build-locked' { & tools/local_build.ps1 +stable-x86_64-pc-windows-gnu build --locked -j 1 }
Gate 'mcp-smoke' {
    $exe = Join-Path $env:CARGO_TARGET_DIR 'x86_64-pc-windows-gnu\debug\sidenote.exe'
    if (-not (Test-Path $exe)) { $exe = Join-Path $env:CARGO_TARGET_DIR 'debug\sidenote.exe' }
    python tools/mcp-smoke.py $exe
}
Gate 'settings-render' { python tools/render_settings_build.py }
Gate 'settings-reachability' { python tools/settings-reachability.py artifacts/settings-build }
Push-Location (Join-Path $root 'relay')
Gate 'relay-tests' { bun test tests }
Pop-Location

$listenerEnd = RecorderSnapshot 'listener-end'
Write-Output "=== listener-end $($listenerEnd.State)"
$recorderLost = $listenerStart.State -eq 'running' -and $listenerEnd.State -ne 'running'
"start=$($listenerStart.State); end=$($listenerEnd.State)" | Set-Content -LiteralPath (Join-Path $logDirectory 'listener-presence.log')
$rows.Add([pscustomobject]@{ Check = 'listener-presence'; Exit = [int]$recorderLost; Seconds = 0; Detail = "start=$($listenerStart.State); end=$($listenerEnd.State)" })
if ($recorderLost) {
    Write-Output "RECORDER CHECK FAILED: installed listener was running at start; end state is $($listenerEnd.State)."
}
$failed = @($rows | Where-Object { $_.Exit -ne 0 })
$lines = @(
    "# Release gate: Sidenote $version ($commit)",
    '',
    "Started $($started.ToString('yyyy-MM-dd HH:mm:ss zzz')), finished $((Get-Date).ToString('HH:mm:ss')) local time.",
    '',
    '| Check | Exit | Seconds | Detail | Log |',
    '|---|---|---|---|---|'
)
foreach ($row in $rows) { $lines += "| $($row.Check) | $($row.Exit) | $($row.Seconds) | $($row.Detail) | [log](release-gate-$commit/$($row.Check).log) |" }
$lines += ''
$lines += $(if ($failed.Count) { "FAILED: $($failed.Check -join ', ')" } else { 'PASSED: all checks' })
$report = Join-Path $artifacts "release-gate-$commit.md"
[IO.File]::WriteAllLines($report, $lines)
Write-Output ("SUMMARY: " + (($rows | ForEach-Object { "$($_.Check) exit=$($_.Exit) $($_.Detail)" }) -join '; '))
Write-Output "Report: $report"
if ($failed.Count) { exit 1 } else { exit 0 }
