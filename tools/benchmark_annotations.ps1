param(
    [string]$Exe = "$PSScriptRoot/../target/x86_64-pc-windows-gnu/debug/sidenote.exe",
    [string]$Out = "$PSScriptRoot/../artifacts/annotations-benchmark",
    [ValidateSet('baseline','direct','codex','draft1024','parallel3')]
    [string[]]$Variants = @('baseline','direct','codex','draft1024','parallel3')
)
$ErrorActionPreference = 'Stop'
$env:PATH = 'E:/Personal-Projects/tools/hex-meetings-gnu/w64devkit/bin;' + $env:PATH
$root = (Resolve-Path "$PSScriptRoot/..").Path
$Exe = (Resolve-Path $Exe).Path
$Out = [IO.Path]::GetFullPath($Out)
# Another lane can replace the shared target executable. Launch the worktree copy only.
if ($Exe -like '*\.worktrees\settings\target\*') { throw 'Copy the verified build into this worktree before benchmarking.' }
& rg -a -q -F -- '--annotation-benchmark' $Exe
if ($LASTEXITCODE -ne 0) { throw 'This executable does not contain the annotation benchmark entry point.' }
New-Item -ItemType Directory -Force -Path $Out | Out-Null
foreach ($variant in $Variants) {
    $process = Start-Process -FilePath $Exe -ArgumentList @('--annotation-benchmark', $variant, '--annotation-proof-output', ('"' + $Out + '"')) -WorkingDirectory $root -WindowStyle Hidden -Wait -PassThru -RedirectStandardOutput "$Out/$variant.out.log" -RedirectStandardError "$Out/$variant.err.log"
    Get-Content "$Out/$variant.out.log"
    if ($process.ExitCode -ne 0) { throw "$variant failed. Read $Out/$variant.err.log. The durable ledger still counts failed attempts toward 16." }
}
