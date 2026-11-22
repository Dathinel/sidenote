# Exercise the real wrapper and gate calls against a native argv recorder, without compiling Rust.
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
$temp = Join-Path ([IO.Path]::GetTempPath()) ('local-build-test-' + [guid]::NewGuid().ToString('N'))
$saved = @{}
foreach ($name in 'PATH','APPDATA','LOCALAPPDATA','CARGO_BUILD_JOBS','CARGO_TARGET_DIR','CARGO_BUILD_TARGET_DIR') {
    $saved[$name] = [Environment]::GetEnvironmentVariable($name)
}
function Get-CimInstance {
    if ($listenerRunning) { [pscustomobject]@{ Name = 'sidenote.exe'; CommandLine = 'sidenote.exe --listen' } }
    if ($hexName) { [pscustomobject]@{ Name = $hexName; CommandLine = $hexName } }
}
function Assert-MeetingStatus([bool]$Expected, [string]$Case, [DateTimeOffset]$Now = [DateTimeOffset]::UtcNow) {
    function Get-Date { $Now.UtcDateTime }
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
    $listenerRunning = $false
    $hexName = 'hex.exe'
    Write-MeetingStatus recording -Time ([DateTimeOffset]::UtcNow.AddMinutes(-1))
    Assert-MeetingStatus $true 'stale recording without the listener, with Hex running'
    $hexName = $null
    Assert-MeetingStatus $false 'stale recording without the listener or Hex'

    $listenerRunning = $true
    $hexName = 'hex.exe'
    foreach ($state in 'starting','recording','paused','stopping','finishing') {
        Write-MeetingStatus $state
        Assert-MeetingStatus $true "fresh $state row"
    }
    Write-MeetingStatus idle
    Assert-MeetingStatus $false 'fresh idle row'
    $listenerRunning = $false
    Assert-MeetingStatus $true 'fresh idle row from a stopped listener'

    $listenerRunning = $true
    $now = [DateTimeOffset]::UtcNow
    Write-MeetingStatus idle -Time ($now.AddSeconds(-15))
    Assert-MeetingStatus $false '15 seconds old is fresh' -Now $now
    Assert-MeetingStatus $true '15 seconds plus one tick is stale' -Now ($now.AddTicks(1))
    foreach ($hexName in @($null, 'hex.com')) {
        foreach ($poll in 1..2) {
            Write-MeetingStatus -StatusError 'not_running'
            Assert-MeetingStatus ([bool]$hexName) "not_running poll $poll with Hex presence $([bool]$hexName)"
        }
    }

    Write-MeetingStatus idle
    [IO.File]::AppendAllText($statusFile, '{"timestamp":')
    Assert-MeetingStatus $true 'partial last line with Hex running'
    $hexName = $null
    Assert-MeetingStatus $false 'partial last line with Hex closed'
    [IO.File]::AppendAllText($statusFile, [Environment]::NewLine)
    Write-MeetingStatus idle
    $hexName = 'hex.exe'
    $held = [IO.File]::Open($statusFile, 'Open', 'ReadWrite', 'None')
    try {
        Assert-MeetingStatus $true 'unreadable status file with Hex running'
        $hexName = $null
        Assert-MeetingStatus $false 'unreadable status file with Hex closed'
    } finally { $held.Dispose() }
    Remove-Item -LiteralPath $statusFile
    Assert-MeetingStatus $false 'absent status file with Hex closed'
    $hexName = 'hex.exe'
    Assert-MeetingStatus $true 'absent status file with Hex running'

    $processMock = 'function Get-CimInstance { ' + (Get-Command Get-CimInstance).Definition + ' }; $listenerRunning = $true; $hexName = ''hex.exe''; '
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
        $output = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ($processMock + $command))
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
    Write-MeetingStatus -StatusError 'not_running'
    $waitCommand = @'
function Start-Sleep([int]$Seconds) {
    if ($Seconds -ne 10 -or $script:waited) { throw 'Unexpected wrapper wait.' }
    $script:waited = $true
    $script:hexName = $null
    'waited for meeting'
}
& tools/local_build.ps1 check
'@
    $output = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ($processMock + $waitCommand))
    if ($LASTEXITCODE -or $output[0] -cne 'waited for meeting' -or $output -cnotcontains 'jobs:2') {
        throw 'Wrapper must wait while Hex is running and preserve the caller job count.'
    }
    Write-MeetingStatus idle
    $ErrorActionPreference = 'Continue'
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ($processMock + '& tools/local_build.ps1 check --config build.target-dir=unchecked') 2>$null
    $ErrorActionPreference = 'Stop'
    if ($LASTEXITCODE -ne 1) { throw 'Unchecked Cargo config was accepted.' }
    $ErrorActionPreference = 'Continue'
    $outside = Join-Path $temp 'outside'
    $output = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ($processMock + "& tools/local_build.ps1 check --target-dir '$outside'") 2>$null)
    $ErrorActionPreference = 'Stop'
    if ($LASTEXITCODE -ne 1 -or @($output | Where-Object { $_.StartsWith('arg:') }).Count) { throw 'Target outside the worktree was accepted.' }

    # Give default-target tests their own Git common directory and cache.
    $project = Join-Path $temp 'project'
    & git init --quiet $project
    if ($LASTEXITCODE) { throw 'Could not initialize the shared-target fixture.' }
    Copy-Item -LiteralPath (Join-Path $root 'Cargo.lock') -Destination $project
    $env:CARGO_TARGET_DIR = $null
    $env:CARGO_BUILD_TARGET_DIR = $null
    $target = "${project}-shared-target\slot-1"
    $localUnit = Join-Path $target 'x86_64-pc-windows-gnu\debug\.fingerprint\spake2-0123456789abcdef\lib-spake2'
    $registryUnit = Join-Path $target 'debug\.fingerprint\serde-0123456789abcdef\lib-serde'
    foreach ($file in $localUnit, $registryUnit) {
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $file) | Out-Null
        Set-Content -LiteralPath $file -Value 'old fingerprint'
    }
    Set-Content -LiteralPath (Join-Path $target '.sidenote-worktree') -Value 'different worktree'

    $defaultCommand = "& '$PSScriptRoot\local_build.ps1' check"
    function Assert-SharedTarget([int]$Number) {
        $target = "${project}-shared-target\slot-$Number"
        Write-MeetingStatus idle
        $output = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ($processMock + $defaultCommand))
        $targets = @($output | Where-Object { $_.StartsWith('target:') })
        if ($LASTEXITCODE -or $targets.Count -ne 3 -or @($targets | Where-Object { $_ -cne "target:$target" }).Count -or
            @($output | Where-Object { $_ -ceq "local_build: target directory: $target" }).Count -ne 1) {
            throw "Wrong shared target for slot $Number."
        }
    }
    Push-Location $project
    try {
        Assert-SharedTarget 1
        if ((Test-Path -LiteralPath $localUnit) -or -not (Test-Path -LiteralPath $registryUnit)) {
            throw 'The first build should invalidate only path package fingerprints.'
        }
        Set-Content -LiteralPath $localUnit -Value 'rebuilt fingerprint'
        Assert-SharedTarget 1
        if (-not (Test-Path -LiteralPath $localUnit)) { throw 'The same worktree lost its fresh fingerprint.' }
        $heldSlot = Join-Path $temp "sidenote-dev\build-slots\$PID-$((Get-Process -Id $PID).StartTime.ToUniversalTime().Ticks)"
        New-Item -ItemType Directory -Path $heldSlot | Out-Null
        Set-Content -LiteralPath (Join-Path $heldSlot 'number') -Value '1' -NoNewline
        Assert-SharedTarget 2
    } finally { Pop-Location }
    Write-Output 'PASS: meeting status, Cargo jobs and arguments, target overrides and refusals, both shared slots, and worktree cache invalidation.'
} finally {
    foreach ($name in $saved.Keys) { [Environment]::SetEnvironmentVariable($name, $saved[$name]) }
    if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Recurse -Force }
}
