<# Install only while Hex is idle, then require complete post-install evidence. #>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Feed,
    [Parameter(Mandatory)][ValidatePattern('^\d+\.\d+\.\d+(?:-[A-Za-z0-9.-]+)?(?:\+[A-Za-z0-9.-]+)?$')]
    [string]$Version,
    [Parameter(Mandatory)][string]$Log,
    [switch]$WhatIf
)
$ErrorActionPreference = 'Stop'
$stage = 'preflight'
function Quote-Argument([string]$Value) {
    return '"' + ([regex]::Replace($Value, '(\\*)"', '$1$1\"') -replace '(\\+)$', '$1$1') + '"'
}
function Run-Hidden([string]$Exe, [string[]]$Arguments) {
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $Exe
    $info.Arguments = (@($Arguments | ForEach-Object { Quote-Argument $_ }) -join ' ')
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $child = [Diagnostics.Process]::Start($info)
    try {
        # Drain both streams without exposing paths, environment values or hook errors.
        $stdout = $child.StandardOutput.ReadToEndAsync()
        $stderr = $child.StandardError.ReadToEndAsync()
        $child.WaitForExit()
        [void]$stdout.GetAwaiter().GetResult()
        [void]$stderr.GetAwaiter().GetResult()
        if ($child.ExitCode -ne 0) { throw 'Child failed' }
    } finally { $child.Dispose() }
}
try {
    if ($WhatIf) {
        Write-Output '1. Require hex.com status --json: meeting.state=idle and capture.pending_jobs=0.'
        Write-Output '2. Run <Feed>/sidenote-win-Setup.exe --silent --log <Log>; wait and require exit 0.'
        Write-Output "3. Run installed sidenote.exe --veloapp-updated $Version; wait and require exit 0."
        Write-Output '4. Start Sidenote meeting listener; wait up to 30 seconds for the installed sidenote.exe --listen.'
        Write-Output "5. Run tools/install_check.ps1 -Expect $Version; require every row to pass."
        exit 0
    }
    $stage = 'Hex idle check: require readable status, meeting.state idle and capture.pending_jobs 0'
    $raw = & hex.com status --json 2>$null
    if ($LASTEXITCODE -ne 0) { throw 'Status failed' }
    $status = ($raw -join "`n") | ConvertFrom-Json
    if ($status.meeting.state -cne 'idle' -or
        ($status.capture.pending_jobs -isnot [int] -and $status.capture.pending_jobs -isnot [long]) -or
        $status.capture.pending_jobs -ne 0) { throw 'Hex is busy or status is incomplete' }

    $stage = 'silent Setup: require the feed installer and exit 0; inspect the requested log on failure'
    $setup = Join-Path $Feed 'sidenote-win-Setup.exe'
    if (-not (Test-Path -LiteralPath $setup -PathType Leaf)) { throw 'Installer missing' }
    Run-Hidden $setup @('--silent', '--log', [IO.Path]::GetFullPath($Log))

    $stage = 'after-update hook: require the installed app and exit 0'
    $exe = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'sidenote/current/sidenote.exe'
    if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw 'Installed app missing' }
    Run-Hidden $exe @('--veloapp-updated', $Version)

    $stage = 'listener start: require the installed sidenote.exe --listen within 30 seconds'
    Start-ScheduledTask -TaskPath '\' -TaskName 'Sidenote meeting listener' *> $null
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
        $listeners = @(Get-CimInstance Win32_Process -Filter "Name = 'sidenote.exe'" | Where-Object {
            $_.ExecutablePath -ieq $exe -and $_.CommandLine -match '(?i)(?:^|\s)"?--listen"?(?=\s|$)'
        })
        if ($listeners.Count) { break }
        if ([DateTime]::UtcNow -ge $deadline) { throw 'Listener did not start' }
        Start-Sleep -Milliseconds 250
    } while ($true)

    $stage = 'install_check: every row must pass'
    & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'install_check.ps1') -Expect $Version
    if ($LASTEXITCODE -ne 0) { throw 'Install check failed' }
    Write-Output 'Install checks passed.'
    exit 0
} catch {
    [Console]::Error.WriteLine("install_release: failed at $stage.")
    exit 1
}
