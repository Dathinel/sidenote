<#
Self-test for prune_merged_targets.ps1. Builds a throwaway git repo under %TEMP%,
runs the script as a child process and checks which target folders are left.
Touches nothing outside that temp folder. Exit 0 = all checks passed.
#>
$ErrorActionPreference = 'Continue'  # native stderr must not abort; git failures throw explicitly
$script = Join-Path $PSScriptRoot 'prune_merged_targets.ps1'
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("prune-selftest-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
$repo = Join-Path $tmp 'repo'
$wt = Join-Path $tmp 'wt'
$fails = 0
function Invoke-Git { & git -C $repo -c user.name=t -c user.email=t@example.invalid @args *> $null; if ($LASTEXITCODE -ne 0) { throw "git $args failed" } }
function Check([string]$Name, [bool]$Ok) {
    if ($Ok) { Write-Host "PASS  $Name" } else { Write-Host "FAIL  $Name"; $script:fails++ }
}
function Has([string]$Name) { Test-Path -LiteralPath (Join-Path (Join-Path $wt $Name) 'target') }
function Add-Wt([string]$Name, [string]$Branch, [string]$Start) {
    Invoke-Git worktree add -b $Branch (Join-Path $wt $Name) $Start
    $t = Join-Path (Join-Path $wt $Name) 'target'
    New-Item -ItemType Directory -Force -Path $t | Out-Null
    Set-Content -LiteralPath (Join-Path $t 'a.bin') 'x'
}
function Run-Prune([string[]]$Extra) {
    $out = & powershell -NoProfile -ExecutionPolicy Bypass -File $script -Repo $repo -Release release/x @Extra
    $out | ForEach-Object { Write-Host "  | $_" }
    return $LASTEXITCODE
}

try {
    New-Item -ItemType Directory -Force -Path $repo, $wt | Out-Null
    & git -C $repo init -q -b main
    Set-Content -LiteralPath (Join-Path $repo '.gitignore') 'target/'
    Set-Content -LiteralPath (Join-Path $repo 'f.txt') 'one'
    Invoke-Git add -A; Invoke-Git commit -q -m one
    Invoke-Git branch release/x
    Add-Wt 'merged'    'lane/merged'    'main'
    Add-Wt 'dirty'     'lane/dirty'     'main'
    Add-Wt 'keep'      'lane/keep'      'main'
    Add-Wt 'relwt'     'release/x-wt'   'main'   # merged and clean (not the release branch), so it is pruned
    Add-Wt 'unmerged'  'lane/unmerged'  'main'
    Set-Content -LiteralPath (Join-Path (Join-Path $wt 'unmerged') 'g.txt') 'two'
    & git -C (Join-Path $wt 'unmerged') add g.txt
    & git -C (Join-Path $wt 'unmerged') -c user.name=t -c user.email=t@example.invalid commit -q -m two
    Set-Content -LiteralPath (Join-Path (Join-Path $wt 'dirty') 'f.txt') 'edited'
    # path longer than 260 chars inside the clean merged target
    $deep = Join-Path (Join-Path $wt 'merged') 'target'
    1..12 | ForEach-Object { $deep = Join-Path $deep ('d' * 24) }
    [void][IO.Directory]::CreateDirectory('\\?\' + $deep)
    [IO.File]::WriteAllText('\\?\' + (Join-Path $deep 'deep.bin'), 'x')
    Check "setup: deep path is over 260 chars ($($deep.Length))" ($deep.Length -gt 260)

    Write-Host '== -WhatIf'
    $rc = Run-Prune @('-Keep', (Join-Path $wt 'keep'), '-WhatIf')
    Check 'whatif exit 0' ($rc -eq 0)
    Check 'whatif deleted nothing' ((Has 'merged') -and (Has 'dirty') -and (Has 'keep') -and (Has 'unmerged'))

    Write-Host '== real run'
    $rc = Run-Prune @('-Keep', (Join-Path $wt 'keep'))
    Check 'real exit 0' ($rc -eq 0)
    Check 'merged+clean target removed (incl. deep path)' (-not (Has 'merged'))
    Check 'relwt (merged, clean, other branch) target removed' (-not (Has 'relwt'))
    Check 'dirty target kept' (Has 'dirty')
    Check 'unmerged target kept' (Has 'unmerged')
    Check '-Keep target kept' (Has 'keep')
    Check 'worktree itself still there' (Test-Path -LiteralPath (Join-Path $wt 'merged\f.txt'))

    Write-Host '== release branch checked out, running process, locked file'
    Invoke-Git worktree add (Join-Path $wt 'onrelease') release/x
    New-Item -ItemType Directory -Force -Path (Join-Path $wt 'onrelease\target') | Out-Null
    Add-Wt 'running' 'lane/running' 'main'
    $exe = Join-Path $wt 'running\target\hold.exe'
    Copy-Item (Join-Path $env:SystemRoot 'System32\ping.exe') $exe
    $p = Start-Process -FilePath $exe -ArgumentList '-n', '60', '127.0.0.1' -PassThru -WindowStyle Hidden
    Add-Wt 'locked' 'lane/locked' 'main'
    $held = [IO.File]::Open((Join-Path $wt 'locked\target\a.bin'), 'Open', 'Read', 'None')
    try {
        $rc = Run-Prune @('-Keep', (Join-Path $wt 'keep'))
        Check 'locked file: exit 1' ($rc -eq 1)
        Check 'locked target still there' (Has 'locked')
        Check 'release checkout target kept' (Has 'onrelease')
        Check 'running-process target kept' (Has 'running')
    } finally {
        $held.Dispose()
        Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
        $p.WaitForExit()
    }
    $rc = Run-Prune @('-Keep', (Join-Path $wt 'keep'))
    Check 'after release: exit 0' ($rc -eq 0)
    Check 'locked and running now removed' ((-not (Has 'locked')) -and (-not (Has 'running')))
    Check 'release checkout still kept' (Has 'onrelease')
} finally {
    Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith($tmp, [StringComparison]::OrdinalIgnoreCase) } | Stop-Process -Force
    Remove-Item -LiteralPath ('\\?\' + $tmp) -Recurse -Force -ErrorAction SilentlyContinue
}
if ($fails) { Write-Host "$fails check(s) failed"; exit 1 }
Write-Host 'all checks passed'
exit 0
