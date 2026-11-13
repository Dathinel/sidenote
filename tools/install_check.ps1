<#
Read-only post-install evidence. Run with -Expect <version> [-Before].
Do not claim "installed and verified" without pasting the complete after table.
No application is launched: sq.version supplies the installer's recorded version.
The embedded task checker runs in this PowerShell process, with repairs disabled;
normal completion/terminating error maps to exit code 0/1, just as powershell -File.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidatePattern('^\d+\.\d+\.\d+(?:-[A-Za-z0-9.-]+)?(?:\+[A-Za-z0-9.-]+)?$')]
    [string]$Expect,
    [switch]$Before
)
$ErrorActionPreference = 'Stop'
$rows = [Collections.Generic.List[object]]::new()
function Add-Check([string]$Name, [bool]$Pass, [string]$Observed) {
    $rows.Add([pscustomobject]@{ Check = $Name; Result = $(if ($Pass) { 'PASS' } else { 'FAIL' }); Observed = $Observed })
}
function Safe-Version($Value) {
    if ($Value -is [string] -and $Value -cmatch '^\d+\.\d+\.\d+(?:-[A-Za-z0-9.-]+)?(?:\+[A-Za-z0-9.-]+)?$') { return $Value }
    return 'missing or invalid'
}
function Invoke-ListenerCheck([string]$Exe) {
    $saved = @{}
    $flags = @{ SIDENOTE_TASK_NAME = 'Sidenote meeting listener'; SIDENOTE_TASK_EXE = $Exe
        SIDENOTE_TASK_ARGS = '--listen'; SIDENOTE_TASK_RUN = 'no'; SIDENOTE_TASK_REGISTER = 'no' }
    try {
        $source = [IO.File]::ReadAllText((Join-Path $PSScriptRoot '../src/updates_windows.rs'))
        $match = [regex]::Match($source, '(?s)const LISTENER_TASK_SCRIPT: &str = r#"(.*?)"#;')
        if (-not $match.Success) { throw 'Embedded checker unavailable' }
        foreach ($key in $flags.Keys) {
            $saved[$key] = [Environment]::GetEnvironmentVariable($key, 'Process')
            [Environment]::SetEnvironmentVariable($key, $flags[$key], 'Process')
        }
        # Discard every stream: task errors may contain private paths or account names.
        & ([scriptblock]::Create($match.Groups[1].Value)) *> $null
        return 0
    } catch { return 1 }
    finally {
        foreach ($key in $saved.Keys) { [Environment]::SetEnvironmentVariable($key, $saved[$key], 'Process') }
    }
}

# Only these two folder locations are consulted; private settings are never loaded.
$app = $null
$scripts = $null
$local = [Environment]::GetFolderPath('LocalApplicationData')
$profile = [Environment]::GetFolderPath('UserProfile')
if ($local) { $app = Join-Path $local 'Sidenote/current' }
if ($profile) { $scripts = Join-Path $profile '.config/hex/scripts' }
$exe = if ($app) { Join-Path $app 'sidenote.exe' } else { '' }
$listenerExit = Invoke-ListenerCheck $exe
if ($Before) {
    Add-Check 'Listener task check' ($listenerExit -eq 0) "exit=$listenerExit; run=no; register=no"
    $rows | Format-Table -AutoSize | Out-String -Width 240 | Write-Output
    exit $listenerExit
}

try {
    if (-not $app -or -not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw 'App unavailable' }
    # Disable XML entity resolution even though this is local installer metadata.
    $xml = [xml]::new()
    $xml.XmlResolver = $null
    $xml.Load((Join-Path $app 'sq.version'))
    $node = $xml.SelectSingleNode('//*[local-name()="metadata"]/*[local-name()="version"]')
    $version = Safe-Version $node.InnerText
    Add-Check 'Installed version' ($version -ceq $Expect) "$version (installer sq.version)"
} catch { Add-Check 'Installed version' $false 'unavailable or invalid installer metadata' }

try {
    $service = Get-CimInstance Win32_Service -Filter "Name = 'Schedule'"
    $parent = Get-CimInstance Win32_Process -Filter "ProcessId = $($service.ProcessId)"
    $recorders = @(Get-CimInstance Win32_Process -Filter "Name = 'sidenote.exe'" | Where-Object {
        $_.ExecutablePath -and $_.ExecutablePath -ieq $exe -and
        $_.CommandLine -match '(?i)(?:^|\s)"?--listen"?(?=\s|$)'
    })
    # Creation order rules out a recycled parent PID.
    $owned = @($recorders | Where-Object {
        $service.State -eq 'Running' -and $service.ProcessId -gt 0 -and
        $_.ParentProcessId -eq $service.ProcessId -and $parent.CreationDate -and
        $_.CreationDate -and $parent.CreationDate -le $_.CreationDate
    })
    Add-Check 'Recorder parent' ($recorders.Count -eq 1 -and $owned.Count -eq 1) "listeners=$($recorders.Count); scheduler-parent=$($owned.Count)"
} catch { Add-Check 'Recorder parent' $false 'process or scheduler query unavailable' }

try {
    $task = Get-ScheduledTask -TaskPath '\' -TaskName 'Sidenote meeting listener'
    $state = if ([string]$task.State -in @('Running', 'Ready', 'Disabled', 'Queued', 'Unknown')) { [string]$task.State } else { 'unknown' }
    Add-Check 'Listener task state' ($state -eq 'Running') "exists; $state"
} catch { Add-Check 'Listener task state' $false 'missing or unreadable' }
Add-Check 'Listener task check' ($listenerExit -eq 0) "exit=$listenerExit; run=no; register=no"

try {
    if (-not $scripts) { throw 'Scripts unavailable' }
    $pointer = Get-Content -LiteralPath (Join-Path $scripts '.pipeline-current.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    $versions = @($pointer.version, $pointer.generation, $pointer.current.version, $pointer.current.generation)
    $safe = @($versions | ForEach-Object { Safe-Version $_ } | Where-Object { $_ -ne 'missing or invalid' } | Select-Object -Unique)
    Add-Check 'Pipeline generation/version' ($safe -ccontains $Expect) $(if ($safe.Count) { $safe -join ', ' } else { 'exists; version missing or invalid' })
} catch { Add-Check 'Pipeline generation/version' $false 'missing or unreadable pointer' }

try {
    if (-not $local) { throw 'Local app data unavailable' }
    $pending = Test-Path -LiteralPath (Join-Path $local 'Sidenote/updates/pipeline-install.pending')
    Add-Check 'Pending pipeline install' (-not $pending) $(if ($pending) { 'present' } else { 'absent' })
} catch { Add-Check 'Pending pipeline install' $false 'unreadable' }

$installTime = $null
try {
    if (-not $app -or -not $scripts) { throw 'Folders unavailable' }
    $appFiles = @(Get-ChildItem -LiteralPath $app -File -Recurse -Force)
    if (-not $appFiles.Count) { throw 'Install time unavailable' }
    $installTime = ($appFiles | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1).LastWriteTimeUtc
    $newest = Get-ChildItem -LiteralPath $scripts -File -Recurse -Force | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
    $observed = 'scripts=missing'
    if ($newest) { $observed = 'scripts=' + $newest.LastWriteTimeUtc.ToString('yyyy-MM-ddTHH:mm:ss.fffffffZ') }
    # Symbolic folder identifies the source without printing any environment value.
    $observed += '; install=' + $installTime.ToString('yyyy-MM-ddTHH:mm:ss.fffffffZ') + ' (newest file in %LOCALAPPDATA%/Sidenote/current)'
    Add-Check 'Scripts newer than install' ($null -ne $newest -and $newest.LastWriteTimeUtc -gt $installTime) $observed
} catch { Add-Check 'Scripts newer than install' $false 'folder or timestamp unavailable; install folder=%LOCALAPPDATA%/Sidenote/current' }

try {
    if (-not $installTime -or -not $local) { throw 'Install time unavailable' }
    $log = Join-Path $local 'hex-meeting-watch.log'
    $count = 0
    $undated = 0
    $kinds = [Collections.Generic.HashSet[string]]::new()
    # Continuations inherit the preceding entry's time; leading undated text is old.
    $stamp = [datetimeoffset]::MinValue
    # Streaming scan of this diagnostic log only; no meeting files are opened.
    foreach ($line in [IO.File]::ReadLines($log)) {
        $dated = $false
        $parsed = [datetimeoffset]::MinValue
        if ($line -match '^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})?)(?=\s|$)' -and
            [datetimeoffset]::TryParse($Matches[1], [ref]$parsed)) {
            $stamp = $parsed
            $dated = $true
        }
        if ($line -notmatch '(?i)\b(error|failed|failure|exception|timeout|timed out)\b') { continue }
        if ($stamp.UtcDateTime -lt $installTime) { continue }
        $count++
        if (-not $dated) { $undated++ }
        # Only fixed categories can leave this process, never raw log tokens.
        $kind = 'error'
        if ($line -match '\bpipeline-failure\b') {
            $kind = 'pipeline-failure'
            if ($line -match '\bcategory=(timeout|exit|missing)(?=\s|$)') { $kind += '/' + $Matches[1].ToLowerInvariant() }
        } elseif ($line -match '(?i)\b(timeout|timed out)\b') { $kind = 'timeout' }
        elseif ($line -match '(?i)\bexception\b') { $kind = 'exception' }
        elseif ($line -match '(?i)\b(failed|failure)\b') { $kind = 'failure' }
        [void]$kinds.Add($kind)
    }
    $names = if ($kinds.Count) { ($kinds | Sort-Object) -join ', ' } else { 'none' }
    Add-Check 'Errors since install' ($count -eq 0) "count=$count; kinds=$names; undated-error-lines=$undated"
} catch { Add-Check 'Errors since install' $false 'count=unknown; kinds=unavailable' }

$rows | Format-Table -AutoSize | Out-String -Width 320 | Write-Output
if (@($rows | Where-Object Result -eq 'FAIL').Count) { exit 1 }
exit 0
