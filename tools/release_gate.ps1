<#
Release gate for local Sidenote releases.

Run from the root of a release worktree:
  powershell -NoProfile -ExecutionPolicy Bypass -File tools\release_gate.ps1 [-GnuBin <dir>]

It refuses to run from anywhere else, always builds into this worktree's own
target folder, installs relay dependencies first, then runs the fixed check
list once. It writes artifacts\release-gate-<commit>.md with a pass table and
exits 1 if any check fails.
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

$rows = New-Object System.Collections.Generic.List[object]
function Gate([string]$Name, [scriptblock]$Run) {
    $t0 = Get-Date
    & $Run
    $code = $LASTEXITCODE
    $secs = [int]((Get-Date) - $t0).TotalSeconds
    $rows.Add([pscustomobject]@{ Check = $Name; Exit = $code; Seconds = $secs })
    Write-Output "=== $Name exit=$code (${secs}s)"
}

Push-Location (Join-Path $root 'relay')
Gate 'relay-deps' { bun install --frozen-lockfile }
Pop-Location
Gate 'fmt' { cargo fmt --check }
Gate 'clippy' { cargo clippy --locked --all-targets -- -D warnings }
Gate 'clippy-asr' { cargo clippy --locked --all-targets --features asr-runtime -- -D warnings }
Gate 'test' { cargo test --locked --quiet }
Gate 'test-asr' { cargo test --locked --quiet --features asr-runtime }
(Get-Item (Join-Path $root 'src\main.rs')).LastWriteTime = Get-Date
Gate 'build-locked' { cargo build --locked }
$exe = Join-Path $env:CARGO_TARGET_DIR 'x86_64-pc-windows-gnu\debug\sidenote.exe'
if (-not (Test-Path $exe)) { $exe = Join-Path $env:CARGO_TARGET_DIR 'debug\sidenote.exe' }
Gate 'mcp-smoke' { python tools/mcp-smoke.py $exe }
Gate 'settings-render' { python tools/render_settings_build.py }
Gate 'settings-reachability' { python tools/settings-reachability.py artifacts/settings-build }
Push-Location (Join-Path $root 'relay')
Gate 'relay-tests' { bun test tests }
Pop-Location

$failed = @($rows | Where-Object { $_.Exit -ne 0 })
$artifacts = Join-Path $root 'artifacts'
New-Item -ItemType Directory -Force $artifacts | Out-Null
$lines = @(
    "# Release gate: Sidenote $version ($commit)",
    '',
    "Started $($started.ToString('yyyy-MM-dd HH:mm:ss zzz')), finished $((Get-Date).ToString('HH:mm:ss')) local time.",
    '',
    '| Check | Exit | Seconds |',
    '|---|---|---|'
)
foreach ($row in $rows) { $lines += "| $($row.Check) | $($row.Exit) | $($row.Seconds) |" }
$lines += ''
$lines += $(if ($failed.Count) { "FAILED: $($failed.Check -join ', ')" } else { 'PASSED: all checks' })
$report = Join-Path $artifacts "release-gate-$commit.md"
[IO.File]::WriteAllLines($report, $lines)
Write-Output ("SUMMARY: " + (($rows | ForEach-Object { "$($_.Check) exit=$($_.Exit)" }) -join '; '))
Write-Output "Report: $report"
if ($failed.Count) { exit 1 } else { exit 0 }
