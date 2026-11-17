# Exercises the release lock and the free-space guard in a throwaway git repo, with a fake cargo and no Rust build.
$ErrorActionPreference = 'Stop'
$tools = $PSScriptRoot
$temp = Join-Path ([IO.Path]::GetTempPath()) ('release-lock-test-' + [guid]::NewGuid().ToString('N'))
$repo = Join-Path $temp 'repo'
$saved = @{}
foreach ($name in 'PATH', 'LOCALAPPDATA', 'SIDENOTE_RELEASE_GATE_PID') { $saved[$name] = [Environment]::GetEnvironmentVariable($name) }
$spawned = [Collections.Generic.List[object]]::new()

function Wait-Until([scriptblock]$Condition, [int]$Seconds, [string]$What) {
    $end = [DateTime]::UtcNow.AddSeconds($Seconds)
    while (-not (& $Condition)) {
        if ([DateTime]::UtcNow -ge $end) { throw "Timed out waiting for $What." }
        Start-Sleep -Milliseconds 200
    }
}
function Start-Hidden([string[]]$Arguments, [string]$Name) {
    $process = Start-Process -FilePath (Get-Command powershell.exe).Source `
        -ArgumentList (@('-NoProfile', '-ExecutionPolicy', 'Bypass') + $Arguments) `
        -WorkingDirectory $repo -WindowStyle Hidden -PassThru `
        -RedirectStandardOutput (Join-Path $temp "$Name.out") -RedirectStandardError (Join-Path $temp "$Name.err")
    $null = $process.Handle
    $spawned.Add($process)
    return $process
}
function Start-Holder([string]$Name) {
    $ready = Join-Path $temp "$Name.ready"
    $stop = Join-Path $temp "$Name.stop"
    $process = Start-Hidden @('-File', (Join-Path $temp 'holder.ps1'), $repo, $ready, $stop) $Name
    Wait-Until { Test-Path -LiteralPath $ready } 20 "holder $Name"
    return [pscustomobject]@{ Process = $process; Stop = $stop }
}
function Read-Text([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return '' }
    $stream = [IO.File]::Open($Path, 'Open', 'Read', 'ReadWrite, Delete')
    try { return (New-Object IO.StreamReader($stream)).ReadToEnd() } finally { $stream.Dispose() }
}

try {
    foreach ($script in 'release_lock.ps1', 'local_build.ps1', 'release_gate.ps1') {
        $errors = $null
        [void][Management.Automation.Language.Parser]::ParseFile((Join-Path $tools $script), [ref]$null, [ref]$errors)
        if ($errors.Count) { throw "$script has $($errors.Count) parse errors." }
    }
    Write-Output 'PASS: release_lock.ps1, local_build.ps1 and release_gate.ps1 parse with 0 errors.'

    New-Item -ItemType Directory -Path (Join-Path $repo 'tools'), (Join-Path $repo 'relay') -Force | Out-Null
    foreach ($script in 'release_lock.ps1', 'local_build.ps1', 'release_gate.ps1', 'prune_merged_targets.ps1') {
        Copy-Item -LiteralPath (Join-Path $tools $script) -Destination (Join-Path $repo "tools\$script")
    }
    Set-Content -LiteralPath (Join-Path $repo 'Cargo.toml') -Value "[package]`nversion = `"0.0.0`""
    Set-Content -LiteralPath (Join-Path $repo 'relay\keep') -Value 'x'
    Set-Content -LiteralPath (Join-Path $temp 'holder.ps1') -Value @'
param($Repo, $Ready, $Stop)
Set-Location $Repo
. (Join-Path $Repo 'tools\release_lock.ps1')
$path = Get-ReleaseLockPath
$lock = Enter-ReleaseLock $path
Set-Content -LiteralPath $Ready -Value 'held'
while (-not (Test-Path -LiteralPath $Stop)) { Start-Sleep -Milliseconds 200 }
Exit-ReleaseLock $lock.Stream $path
'@
    & git -C $repo init -q -b main
    & git -C $repo -c user.name=t -c user.email=t@example.invalid add -A
    & git -C $repo -c user.name=t -c user.email=t@example.invalid commit -q -m init
    $lockFile = Join-Path $repo '.git\sidenote-release-gate.lock'

    Add-Type -OutputAssembly (Join-Path $temp 'cargo.exe') -OutputType ConsoleApplication -TypeDefinition @'
class FakeCargo { static void Main(string[] args) { System.Console.WriteLine("fake-cargo-ran " + string.Join(" ", args)); } }
'@
    $env:PATH = "$temp;" + $env:PATH
    $env:LOCALAPPDATA = $temp
    $env:SIDENOTE_RELEASE_GATE_PID = $null

    $holder = Start-Holder 'holder1'
    $build = Start-Hidden @('-File', (Join-Path $repo 'tools\local_build.ps1'), '--version') 'build1'
    Wait-Until { (Read-Text (Join-Path $temp 'build1.out')) -match 'waiting for the release check' } 30 'the wait line'
    Start-Sleep -Seconds 3
    if ($build.HasExited) { throw 'local_build did not wait for the live release check.' }
    if ((Read-Text (Join-Path $temp 'build1.out')) -match 'fake-cargo-ran') { throw 'cargo started while the lock was held.' }
    Write-Output "PASS: local_build waits while a live release check (PID $($holder.Process.Id)) holds the lock, and cargo has not started."
    New-Item -ItemType File -Path $holder.Stop | Out-Null
    Wait-Until { $build.HasExited } 30 'local_build to proceed'
    if ($build.ExitCode -ne 0 -or (Read-Text (Join-Path $temp 'build1.out')) -notmatch 'fake-cargo-ran --version') { throw "local_build did not run cargo after the lock was released (exit $($build.ExitCode)): $(Read-Text (Join-Path $temp 'build1.out')) $(Read-Text (Join-Path $temp 'build1.err'))" }
    if (Test-Path -LiteralPath $lockFile) { throw 'The holder left its lock file behind.' }
    Write-Output 'PASS: after the holder ended, local_build ran cargo and exited 0, and the lock file is gone.'

    $holder = Start-Holder 'holder2'
    $env:SIDENOTE_RELEASE_GATE_PID = [string]$holder.Process.Id
    $build = Start-Hidden @('-File', (Join-Path $repo 'tools\local_build.ps1'), '--version') 'build2'
    Wait-Until { $build.HasExited } 30 'the release check own build'
    $env:SIDENOTE_RELEASE_GATE_PID = $null
    if ($build.ExitCode -ne 0 -or (Read-Text (Join-Path $temp 'build2.out')) -match 'waiting') { throw 'The release check own build waited or failed.' }
    Write-Output 'PASS: a build started by the lock holder passes through without waiting.'

    $env:SIDENOTE_RELEASE_GATE_PID = $null
    Push-Location $repo
    try {
        . (Join-Path $repo 'tools\release_lock.ps1')
        $threw = $null
        try { Wait-ReleaseLock (Get-ReleaseLockPath) 0.05 60 1 } catch { $threw = $_.Exception.Message }
    } finally { Pop-Location }
    if (-not $threw -or $threw -notmatch 'no cargo was started') { throw 'The bounded wait did not give up with an error.' }
    Write-Output "PASS: the bounded wait gives up with: $threw"

    $gate = Start-Hidden @('-File', (Join-Path $repo 'tools\release_gate.ps1'), '-PreflightOnly') 'gate-busy'
    Wait-Until { $gate.HasExited } 60 'release_gate against a live lock'
    if ($gate.ExitCode -ne 2 -or (Read-Text (Join-Path $temp 'gate-busy.out')) -notmatch 'another release check holds the lock') { throw 'release_gate did not refuse a live lock.' }
    Write-Output 'PASS: release_gate refuses to start while another release check holds the lock (exit 2).'

    Stop-Process -Id $holder.Process.Id -Force
    Wait-Until { $holder.Process.HasExited } 10 'the holder to die'
    if (-not (Test-Path -LiteralPath $lockFile)) { throw 'No stale lock file to take over.' }
    $gate = Start-Hidden @('-File', (Join-Path $repo 'tools\release_gate.ps1'), '-PreflightOnly') 'gate-stale'
    Wait-Until { $gate.HasExited } 60 'release_gate against a stale lock'
    $text = Read-Text (Join-Path $temp 'gate-stale.out')
    if ($gate.ExitCode -ne 0 -or $text -notmatch 'stale release lock' -or $text -notmatch 'preflight passed') { throw "release_gate did not take over the stale lock: $text" }
    if (Test-Path -LiteralPath $lockFile) { throw 'release_gate left its lock file behind.' }
    Write-Output 'PASS: release_gate reported and took over a stale lock, then removed its own lock on exit.'

    $source = [IO.File]::ReadAllText((Join-Path $repo 'tools\release_gate.ps1'))
    if ($source -notmatch '(?m)^\$MinFreeGb = 60\r?$') { throw 'The 60 GB constant is not where the test expects it.' }
    [IO.File]::WriteAllText((Join-Path $repo 'tools\release_gate.ps1'), ($source -replace '(?m)^\$MinFreeGb = 60\r?$', '$MinFreeGb = 1000000'))
    & git -C $repo -c user.name=t -c user.email=t@example.invalid commit -q -am 'fake threshold'
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $gate = Start-Hidden @('-File', (Join-Path $repo 'tools\release_gate.ps1')) 'gate-space'
    Wait-Until { $gate.HasExited } 60 'release_gate with a fake threshold'
    $watch.Stop()
    $text = Read-Text (Join-Path $temp 'gate-space.out')
    if ($gate.ExitCode -ne 2 -or $text -notmatch 'Stopping before any build step' -or $watch.Elapsed.TotalSeconds -gt 30) { throw "The space guard did not stop the check: $text" }
    if (Test-Path -LiteralPath $lockFile) { throw 'The space guard ran after the lock was taken.' }
    Write-Output ("PASS: with the threshold set above the free space, release_gate stopped in {0:N1}s with exit 2: {1}" -f $watch.Elapsed.TotalSeconds, $text.Trim())
    Write-Output 'PASS: the real threshold passes (the stale-lock run above reached "preflight passed" with the 60 GB constant).'
} finally {
    foreach ($process in $spawned) { if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue } }
    foreach ($name in $saved.Keys) { [Environment]::SetEnvironmentVariable($name, $saved[$name]) }
    Set-Location $tools
    if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue }
}
