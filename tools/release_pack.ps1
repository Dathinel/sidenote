<# Run via local_build.ps1 -Pack, from the release worktree root. #>
param(
    [Parameter(Mandatory = $true)] [string]$ExpectedBranch,
    [Parameter(Mandatory = $true)] [string]$ExpectedCommit,
    [Parameter(Mandatory = $true)] [string]$Version,
    [Parameter(Mandatory = $true)] [string]$Feed,
    [Parameter(Mandatory = $true)] [string]$DotnetRoot,
    [Parameter(Mandatory = $true)] [string]$GnuBin,
    [Parameter(Mandatory = $true)] [string]$ReleaseNotes,
    [Parameter(Mandatory = $true)] [string]$Log
)
$ErrorActionPreference = 'Stop'
try {
    $root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
    $top = & git rev-parse --show-toplevel 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $top -or (Resolve-Path $top).Path -ne (Get-Location).Path -or $root -ne (Get-Location).Path) { throw 'Run from the release worktree root.' }
    if (-not (Test-Path 'Cargo.toml') -or -not (Test-Path 'relay' -PathType Container)) { throw 'Cargo.toml or relay directory is missing.' }
    $status = & git status --porcelain --untracked-files=all
    if ($LASTEXITCODE -ne 0 -or $status) { throw 'The worktree must be clean.' }
    $branch = & git symbolic-ref --quiet --short HEAD
    if ($LASTEXITCODE -ne 0 -or $branch -cne $ExpectedBranch) { throw 'The branch does not match ExpectedBranch.' }
    $head = & git rev-parse HEAD
    if ($LASTEXITCODE -ne 0 -or $ExpectedCommit -notmatch '^[0-9a-fA-F]{7,40}$' -or -not $head.StartsWith($ExpectedCommit, [StringComparison]::OrdinalIgnoreCase)) { throw 'The commit does not match ExpectedCommit.' }
    $manifest = [IO.File]::ReadAllText((Join-Path $root 'Cargo.toml'))
    if ([regex]::Match($manifest, '(?m)^version\s*=\s*"([^\"]+)"').Groups[1].Value -cne $Version) { throw 'The manifest version does not match Version.' }
    if (-not [IO.Path]::IsPathRooted($Feed) -or $Feed -match '^[a-z]+://') { throw 'Feed must be an absolute local path or file share.' }
    if (-not [IO.Path]::IsPathRooted($Log)) { throw 'Log must be an absolute path.' }
    foreach ($path in @($DotnetRoot, $GnuBin, $ReleaseNotes)) { if (-not (Test-Path -LiteralPath $path)) { throw 'A toolchain path or release notes file is missing.' } }
    $env:DOTNET_ROOT = $DotnetRoot
    $env:PATH = "$GnuBin;$DotnetRoot;$DotnetRoot\tools;$env:USERPROFILE\.cargo\bin;$env:PATH"
    foreach ($tool in @('cargo.exe', 'rustup.exe', 'as.exe', 'dlltool.exe', 'dotnet.exe', 'vpk.exe')) {
        if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { throw "Required tool is missing: $tool." }
    }
    $toolchains = & rustup toolchain list 2>$null
    if ($LASTEXITCODE -ne 0 -or -not ($toolchains -match '^stable-x86_64-pc-windows-gnu\b')) { throw 'The stable Windows GNU Rust toolchain is missing.' }
    (Get-Process -Id $PID).PriorityClass = 'BelowNormal'
    $env:CARGO_TARGET_DIR = Join-Path $root 'target'
    $env:CARGO_BUILD_JOBS = if ($env:CARGO_BUILD_JOBS) { $env:CARGO_BUILD_JOBS } else { '1' }
    $parent = Split-Path -Parent $Log
    if (-not (Test-Path -LiteralPath $parent -PathType Container)) { throw 'The log directory does not exist.' }
    $ErrorActionPreference = 'Continue'
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'pack_release.ps1') -Feed $Feed -DotnetRoot $DotnetRoot -GnuBin $GnuBin -ReleaseNotes $ReleaseNotes 2>&1 | Out-File -LiteralPath $Log -Encoding utf8
    $packExit = $LASTEXITCODE
    $ErrorActionPreference = 'Stop'
    if ($packExit -ne 0) { throw "Pack failed; see $Log." }
    Write-Output "release_pack: packed $Version; log: $Log"
    exit 0
} catch {
    [Console]::Error.WriteLine(('release_pack: ' + $_.Exception.Message -replace '[\r\n]+', ' '))
    exit 1
}
