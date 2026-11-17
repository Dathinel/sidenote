<#
Frees disk by deleting <worktree>\target of lanes that are already merged.
Run it after a successful install. Only the target folder is ever removed.

A worktree's target folder is deleted only when ALL of these hold:
  - it is not the main checkout (first entry of `git worktree list`)
  - it is not listed in -Keep
  - it does not have -Release checked out
  - `git status --short` is empty
  - its HEAD is an ancestor of -Release
  - no running process has an executable under the worktree
Use -WhatIf to list what would be deleted without deleting.
-Keep takes paths; a single comma-joined string (powershell -File) is split too.
Exit 0 when done; exit 1 when a deletion was attempted and the folder is still
there (or the repo/release could not be read).
Limit: the process check sees only processes whose ExecutablePath this user can read.
#>
[CmdletBinding()]
param(
    [string]$Repo = '',
    [Parameter(Mandatory)][string]$Release,
    [string[]]$Keep = @(),
    [switch]$WhatIf
)

function Get-FullPath([string]$Path) {
    $full = [IO.Path]::GetFullPath($Path)
    $root = [IO.Path]::GetPathRoot($full)
    if ($full.Length -gt $root.Length) { $full = $full.TrimEnd('\', '/') }
    return $full
}

function Get-FreeGB([string]$Path) {
    try {
        $drive = New-Object IO.DriveInfo ([IO.Path]::GetPathRoot($Path))
        return [math]::Round($drive.AvailableFreeSpace / 1GB, 1)
    } catch { return $null }
}

function Get-LongPath([string]$Full) {
    if ($Full.StartsWith('\\')) { return '\\?\UNC\' + $Full.Substring(2) }
    return '\\?\' + $Full
}

function Get-WorktreeEntries([string]$RepoPath) {
    $lines = & git -C $RepoPath worktree list --porcelain
    if ($LASTEXITCODE -ne 0) { throw "git worktree list failed for $RepoPath" }
    $entries = [Collections.Generic.List[object]]::new()
    $cur = $null
    foreach ($line in $lines) {
        if ($line -like 'worktree *') {
            $cur = [pscustomobject]@{ Path = $line.Substring(9); Head = ''; Branch = '' }
            $entries.Add($cur)
        } elseif ($cur -and $line -like 'HEAD *') { $cur.Head = $line.Substring(5) }
        elseif ($cur -and $line -like 'branch *') { $cur.Branch = $line.Substring(7) }
    }
    return $entries
}

function Get-ProcessUnder([string]$Full) {
    $prefix = $Full.TrimEnd('\') + '\'
    foreach ($p in Get-CimInstance Win32_Process -Filter 'ExecutablePath IS NOT NULL' -ErrorAction Stop) {
        if ($p.ExecutablePath.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) { return $p }
    }
    return $null
}

if (-not $Repo) { $Repo = Split-Path -Parent $PSScriptRoot }
$Repo = Get-FullPath $Repo
$releaseName = $Release -replace '^refs/heads/', ''
$releaseRef = "refs/heads/$releaseName"
& git -C $Repo rev-parse --verify --quiet "$releaseRef^{commit}" *> $null
if ($LASTEXITCODE -ne 0) { Write-Host "ERROR: release branch not found: $releaseName"; exit 1 }

$keepFull = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
foreach ($k in $Keep) {
    foreach ($part in ($k -split ',')) {
        if ($part.Trim()) { [void]$keepFull.Add((Get-FullPath $part.Trim())) }
    }
}

$entries = Get-WorktreeEntries $Repo
$before = Get-FreeGB $Repo
$failed = 0
$first = $true
foreach ($e in $entries) {
    $isMain = $first
    $first = $false
    $full = Get-FullPath $e.Path
    $name = Split-Path -Leaf $full
    $skip = $null
    if ($isMain) { $skip = 'main checkout' }
    elseif ($keepFull.Contains($full)) { $skip = 'in -Keep' }
    elseif ($e.Branch -eq $releaseRef) { $skip = "has $releaseName checked out" }
    elseif (-not (Test-Path -LiteralPath $full -PathType Container)) { $skip = 'worktree folder missing' }

    $target = if ($full) { Join-Path $full 'target' } else { '' }
    if (-not $skip) {
        if (-not $target -or -not $target.EndsWith('\target', [StringComparison]::OrdinalIgnoreCase)) {
            $skip = 'refused: computed path is not a target folder'
        } elseif (-not (Test-Path -LiteralPath (Get-LongPath $target))) {
            $skip = 'no target folder'
        } elseif ((Get-Item -LiteralPath (Get-LongPath $target) -Force -ErrorAction Stop).Attributes -band [IO.FileAttributes]::ReparsePoint) {
            $skip = 'target is a link, not touched'
        }
    }
    if (-not $skip) {
        $status = & git -C $full status --short
        if ($LASTEXITCODE -ne 0) { $skip = 'git status failed' }
        elseif ($status) { $skip = 'uncommitted changes' }
    }
    if (-not $skip) {
        & git -C $Repo merge-base --is-ancestor $e.Head $releaseRef
        if ($LASTEXITCODE -eq 1) { $skip = "not merged into $releaseName" }
        elseif ($LASTEXITCODE -ne 0) { $skip = 'merge check failed' }
    }
    if (-not $skip) {
        $proc = Get-ProcessUnder $full
        if ($proc) { $skip = "running process $($proc.Name) (pid $($proc.ProcessId))" }
    }
    if (-not $skip) {
        # A lane branched from the release counts as merged before its first commit, and cargo
        # runs from the global toolchain, so recent writes are the only sign it is still building.
        $cutoff = (Get-Date).AddMinutes(-60)
        $longTarget = Get-LongPath $target
        $recent = @(Get-Item -LiteralPath $longTarget -Force) + @(Get-ChildItem -LiteralPath $longTarget -Directory -Force -ErrorAction SilentlyContinue) +
            @(Get-ChildItem -LiteralPath $longTarget -Directory -Force -ErrorAction SilentlyContinue | ForEach-Object { Get-ChildItem -LiteralPath $_.FullName -Directory -Force -ErrorAction SilentlyContinue }) |
            Where-Object { $_.LastWriteTime -gt $cutoff } | Select-Object -First 1
        if ($recent) { $skip = 'target written in the last 60 minutes' }
    }

    if ($skip) { Write-Host ("SKIP   {0}: {1}" -f $name, $skip); continue }
    if ($WhatIf) { Write-Host ("WOULD DELETE {0}\target" -f $name); continue }

    $long = Get-LongPath $target
    try { Remove-Item -LiteralPath $long -Recurse -Force -ErrorAction Stop }
    catch { Write-Host ("  error: {0}" -f $_.Exception.Message) }
    if (Test-Path -LiteralPath $long) {
        $failed++
        Write-Host ("FAILED {0}\target: still present after delete" -f $name)
    } else {
        Write-Host ("DELETED {0}\target" -f $name)
    }
}

$after = Get-FreeGB $Repo
$drive = [IO.Path]::GetPathRoot($Repo)
Write-Host ("Free on {0} before: {1} GB, after: {2} GB" -f $drive, $before, $after)
if ($failed -gt 0) { exit 1 }
exit 0
