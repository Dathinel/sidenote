# Exercise the real wrapper and gate calls against a native argv recorder, without compiling Rust.
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
$temp = Join-Path ([IO.Path]::GetTempPath()) ('local-build-test-' + [guid]::NewGuid().ToString('N'))
$saved = @{}
foreach ($name in 'PATH','APPDATA','LOCALAPPDATA','CARGO_BUILD_JOBS','CARGO_TARGET_DIR','CARGO_BUILD_TARGET_DIR') {
    $saved[$name] = [Environment]::GetEnvironmentVariable($name)
}
function Get-CimInstance {
    if ($listenerRunning) { [pscustomobject]@{ CommandLine = 'sidenote.exe --listen' } }
}
function Assert-MeetingStatus([bool]$Expected, [string]$Case) {
    if ((& "$PSScriptRoot\local_build.ps1" -MeetingStatus) -cne $Expected) { throw "Wrong meeting status: $Case" }
}
function Write-MeetingStatus([string]$State, [DateTimeOffset]$Time = [DateTimeOffset]::UtcNow, $StatusError = $null) {
    @{
        timestamp = $Time.ToString('o'); app_version = '0.0.0-dev-1109'; protocol = 4
        meeting_state = $State; capture_state = 'idle'; capture_kind = $null
        has_elapsed = $false; has_id = $false; status_error = $StatusError
        window_visible = $false
    } | ConvertTo-Json -Compress | Add-Content -LiteralPath $statusFile
}
try {
    New-Item -ItemType Directory -Path $temp | Out-Null
    $env:APPDATA = $temp
    $env:LOCALAPPDATA = $temp
    $statusDir = Join-Path $temp 'hex-meetings\local'
    New-Item -ItemType Directory -Path $statusDir | Out-Null
    $statusFile = Join-Path $statusDir 'status.ndjson'
    $legacy = Join-Path $temp 'hex-meeting-watch.log'
    $listenerRunning = $true
    Write-MeetingStatus idle
    Write-MeetingStatus recording
    Assert-MeetingStatus $true 'latest row is recording'
    Set-Content -LiteralPath $legacy -Value 'recording'
    Write-MeetingStatus idle
    Assert-MeetingStatus $false 'latest row is idle despite the legacy log'
    Write-MeetingStatus idle -Time ([DateTimeOffset]::UtcNow.AddMinutes(-1))
    Assert-MeetingStatus $true 'stale idle row with listener running'
    Write-MeetingStatus idle -StatusError 'timeout'
    Assert-MeetingStatus $true 'status error with listener running'
    Add-Content -LiteralPath $statusFile -Value '{"timestamp":'
    Assert-MeetingStatus $true 'unreadable row with listener running'
    $listenerRunning = $false
    Assert-MeetingStatus $false 'unreadable row without listener ignores legacy log'
    Remove-Item -LiteralPath $statusFile
    $listenerRunning = $true
    Set-Content -LiteralPath $legacy -Value 'done'
    Assert-MeetingStatus $true 'missing file with listener running despite legacy done'
    $listenerRunning = $false
    Assert-MeetingStatus $false 'legacy done when status file is absent'
    Set-Content -LiteralPath $legacy -Value 'recording'
    Assert-MeetingStatus $true 'legacy active when status file is absent'
    Write-MeetingStatus idle
    Add-Type -OutputAssembly (Join-Path $temp 'cargo.exe') -OutputType ConsoleApplication -TypeDefinition @'
using System;
class CargoArgs {
    static void Main(string[] args) {
        foreach (string arg in args) Console.WriteLine("arg:" + Convert.ToBase64String(System.Text.Encoding.UTF8.GetBytes(arg)));
        foreach (string name in new [] {"CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR", "CARGO_BUILD_BUILD_DIR"})
            Console.WriteLine("target:" + Environment.GetEnvironmentVariable(name));
        Console.WriteLine("jobs:" + Environment.GetEnvironmentVariable("CARGO_BUILD_JOBS"));
    }
}
'@
    $env:PATH = "$temp;" + $env:PATH
    $env:CARGO_BUILD_JOBS = $null
    # local_build refuses targets outside the worktree, so the fallbacks live inside it.
    $fallback = Join-Path $root ('target\local-build-test-' + [guid]::NewGuid().ToString('N'))
    $env:CARGO_TARGET_DIR = Join-Path $fallback 'fallback'
    $env:CARGO_BUILD_TARGET_DIR = Join-Path $fallback 'other-fallback'
    $gate = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'release_gate.ps1'))
    $calls = [regex]::Matches($gate, "(?m)^Gate 'clippy(?:-asr)?' \{ (& tools/local_build.ps1 .+) \}\r?$")
    if ($calls.Count -ne 2) { throw 'Expected both Clippy gate calls.' }
    $commands = @($calls | ForEach-Object { $_.Groups[1].Value })
    $commands += "& tools/local_build.ps1 clippy --target-dir 'target with spaces\' '--' -D warnings --config tool-only"
    $commands += "& tools/local_build.ps1 clippy '--target-dir=target equals' '--' -D warnings"
    foreach ($command in $commands) {
        Write-MeetingStatus idle
        $output = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -Command $command)
        if ($LASTEXITCODE) { throw "Wrapper exited $LASTEXITCODE" }
        $expectedJobs = if ($command -match ' -j 1 ') { 'jobs:' } else { 'jobs:4' }
        if ($output -cnotcontains $expectedJobs) { throw "Wrong Cargo jobs default: $command" }
        $actual = @($output | Where-Object { $_.StartsWith('arg:') } | ForEach-Object {
            [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($_.Substring(4)))
        })
        $tokens = [Management.Automation.PSParser]::Tokenize($command, [ref]$null)
        $expected = @($tokens | Select-Object -Skip 2 | ForEach-Object { $_.Content })
        if (($actual | ConvertTo-Json -Compress) -cne ($expected | ConvertTo-Json -Compress)) {
            throw "Native argument mismatch: $command"
        }
        $target = if ($command.Contains('--target-dir=')) { Join-Path $root 'target equals' }
                  elseif ($command.Contains('--target-dir ')) { Join-Path $root 'target with spaces\' }
                  else { $env:CARGO_TARGET_DIR }
        $targets = @($output | Where-Object { $_.StartsWith('target:') })
        if ($targets.Count -ne 3 -or @($targets | Where-Object { $_ -cne "target:$target" }).Count) {
            throw 'Checked target directory did not reach the child.'
        }
    }
    $env:CARGO_BUILD_JOBS = '2'
    Set-Content -LiteralPath $legacy -Value 'done'
    Write-MeetingStatus recording
    $output = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -Command @'
function Start-Sleep([int]$Seconds) {
    if ($Seconds -ne 10 -or $script:waited) { throw 'Unexpected wrapper wait.' }
    $script:waited = $true
    $file = Join-Path $env:APPDATA 'hex-meetings\local\status.ndjson'
    $status = Get-Content -LiteralPath $file -Tail 1 | ConvertFrom-Json
    $status.meeting_state = 'idle'
    $status.timestamp = [DateTimeOffset]::UtcNow.ToString('o')
    $status | ConvertTo-Json -Compress | Add-Content -LiteralPath $file
    'waited for meeting'
}
& tools/local_build.ps1 check
'@)
    if ($LASTEXITCODE -or $output[0] -cne 'waited for meeting' -or $output -cnotcontains 'jobs:2') {
        throw 'Wrapper must wait for the meeting to end and preserve the caller job count.'
    }
    $ErrorActionPreference = 'Continue'
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -Command '& tools/local_build.ps1 check --config build.target-dir=unchecked' 2>$null
    $ErrorActionPreference = 'Stop'
    if ($LASTEXITCODE -ne 1) { throw 'Unchecked Cargo config was accepted.' }
    $ErrorActionPreference = 'Continue'
    $outside = Join-Path $temp 'outside'
    $output = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "& tools/local_build.ps1 check --target-dir '$outside'" 2>$null)
    $ErrorActionPreference = 'Stop'
    if ($LASTEXITCODE -ne 1 -or @($output | Where-Object { $_.StartsWith('arg:') }).Count) { throw 'Target outside the worktree was accepted.' }
    Write-Output 'PASS: meeting status, Cargo job counts, both gate calls, separator suffix, target overrides, config rejection, and outside-target rejection.'
} finally {
    foreach ($name in $saved.Keys) { [Environment]::SetEnvironmentVariable($name, $saved[$name]) }
    if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Recurse -Force }
}
