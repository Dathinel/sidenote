param([Parameter(Mandatory=$true)][string]$Exe, [string]$Out = 'E:/Personal-Projects/reviews/project-merge-proof')
$ErrorActionPreference = 'Stop'
(Get-Process -Id $PID).PriorityClass = 'BelowNormal'
$allowed = [IO.Path]::GetFullPath('E:/Personal-Projects/reviews/project-merge-proof').TrimEnd('\')
$out = [IO.Path]::GetFullPath($Out).TrimEnd('\')
if ($out -ne $allowed -and -not $out.StartsWith($allowed + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Proof output must stay in the isolated proof root' }
$run = Join-Path $out ('run-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Path $run | Out-Null
$log = Join-Path $run 'proof.txt'
function Invoke-Proof([string]$Name, [string]$Step, [int]$Expected = 0) {
    $base = Join-Path $run $Name
    $result = & $Exe $Step --data-dir "$base/data" --meetings-dir "$base/meetings" --peers-dir "$base/peers" 2>&1
    $code = $LASTEXITCODE
    $result | Add-Content -LiteralPath $log
    if ($code -ne $Expected) { throw "$Name $Step exited $code instead of $Expected. See $log" }
    "$Name $Step exit=$code" | Tee-Object -FilePath $log -Append
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
"PASS: separate-process merge, alias suggestion, peer edit, crash, retry and restart undo. $run" | Tee-Object -FilePath $log -Append
