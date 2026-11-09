param([Parameter(Mandatory=$true)][string]$Exe, [string]$Out = (Join-Path $PSScriptRoot '../artifacts/project-merge-proof'))
$ErrorActionPreference = 'Stop'
(Get-Process -Id $PID).PriorityClass = 'BelowNormal'
$allowed = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../artifacts/project-merge-proof')).TrimEnd('\')
$out = [IO.Path]::GetFullPath($Out).TrimEnd('\')
if ($out -ne $allowed -and -not $out.StartsWith($allowed + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Proof output must stay in the isolated proof root' }
$run = Join-Path $out ('run-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Path $run | Out-Null
$log = Join-Path $run 'proof.txt'
function Invoke-Proof([string]$Name, [string]$Step, [int]$Expected = 0) {
    $base = Join-Path $run $Name
    New-Item -ItemType Directory -Force -Path $base | Out-Null
    $arguments = @($Step, '--data-dir', "`"$base/data`"", '--meetings-dir', "`"$base/meetings`"", '--peers-dir', "`"$base/peers`"")
    $process = Start-Process -FilePath $Exe -ArgumentList $arguments -WindowStyle Hidden -Wait -PassThru -RedirectStandardOutput "$base/$Step.out" -RedirectStandardError "$base/$Step.err"
    $code = $process.ExitCode
    Get-Content -LiteralPath "$base/$Step.out", "$base/$Step.err" | Add-Content -LiteralPath $log -Encoding utf8
    if ($code -ne $Expected) { throw "$Name $Step exited $code instead of $Expected. See $log" }
    $line = "$Name $Step exit=$code"
    Add-Content -LiteralPath $log -Value $line -Encoding utf8
    Write-Output $line
}
Invoke-Proof normal prepare
Invoke-Proof normal merge
Invoke-Proof normal merged
Invoke-Proof normal incoming
Invoke-Proof normal backup-guard
Invoke-Proof normal peer
Invoke-Proof normal undo
Invoke-Proof normal old
Invoke-Proof normal peer-kept
Invoke-Proof moved prepare
Invoke-Proof moved merge
Invoke-Proof moved move-to-survivor
Invoke-Proof moved undo-refused
Invoke-Proof unchanged prepare
Invoke-Proof unchanged merge
Invoke-Proof unchanged undo
Invoke-Proof unchanged old
Invoke-Proof crash prepare
$env:SIDENOTE_MERGE_CRASH_AFTER_BACKUP = '1'
try { Invoke-Proof crash merge 86 } finally { Remove-Item Env:SIDENOTE_MERGE_CRASH_AFTER_BACKUP }
Invoke-Proof crash old
Invoke-Proof crash merge
Invoke-Proof crash merged
Invoke-Proof crash undo
Invoke-Proof crash old
Invoke-Proof stale prepare
Invoke-Proof stale stale
Invoke-Proof stale peer-kept
$caseIndex = 0
foreach ($scenario in @('boundary-merge', 'boundary-same-value', 'boundary-task', 'boundary-memory', 'undo-boundary-merge', 'undo-boundary-task', 'undo-boundary-memory', 'peer-rename')) {
    $caseIndex++
    Invoke-Proof "r$caseIndex" prepare
    Invoke-Proof "r$caseIndex" $scenario
}
$line = "PASS: separate-process merge, alias suggestion, peer edit, crash, retry and restart undo. $run"
Add-Content -LiteralPath $log -Value $line -Encoding utf8
Write-Output $line
