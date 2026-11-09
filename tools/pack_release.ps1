param(
    [string]$Feed = 'E:\Personal-Projects\reviews\updates-proof\feed',
    [string]$DotnetRoot = 'E:\Personal-Projects\tools\dotnet',
    [string]$ReleaseNotes = ''
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot/..").Path
$feedPath = [IO.Path]::GetFullPath($Feed)
if ($Feed -match '^[a-z]+://' -or -not [IO.Path]::IsPathRooted($Feed)) { throw 'Feed must be an absolute local folder or file share.' }
$env:DOTNET_ROOT = $DotnetRoot
$env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
$env:PATH = "$DotnetRoot;$DotnetRoot/tools;$env:USERPROFILE/.cargo/bin;E:/Personal-Projects/tools/hex-meetings-gnu/w64devkit/bin;$env:PATH"
$env:CARGO_TARGET_DIR = 'E:\Personal-Projects\hex-meetings-shared-target'
(Get-Process -Id $PID).PriorityClass = 'BelowNormal'
Push-Location -LiteralPath $root
try {
    $version = [regex]::Match((Get-Content Cargo.toml -Raw), '(?m)^version = "([^"]+)"').Groups[1].Value
    if (-not $version) { throw 'Cargo.toml has no package version.' }
    (Get-Item src/main.rs).LastWriteTime = Get-Date
    & cargo build --release --locked --bin sidenote
    if ($LASTEXITCODE) { throw 'Release build failed.' }
    $stage = "$root/artifacts/updates-core/package-$version"
    New-Item -ItemType Directory -Force -Path $stage, $feedPath | Out-Null
    Copy-Item -LiteralPath "$env:CARGO_TARGET_DIR/x86_64-pc-windows-gnu/release/sidenote.exe" -Destination "$stage/sidenote.exe" -Force
    $packArgs = @('pack', '--packId', 'sidenote', '--packVersion', $version, '--packDir', $stage,
        '--mainExe', 'sidenote.exe', '--packTitle', 'Sidenote', '--outputDir', $feedPath, '--shortcuts', 'None')
    if ($ReleaseNotes) { $packArgs += @('--releaseNotes', (Resolve-Path -LiteralPath $ReleaseNotes).Path) }
    & vpk --skip-updates --yes @packArgs
    if ($LASTEXITCODE) { throw 'Velopack pack failed.' }
    $index = Join-Path $feedPath 'releases.win.json'
    $releases = Get-Content -LiteralPath $index -Raw | ConvertFrom-Json
    $full = @($releases.Assets | Where-Object Type -eq 'Full')
    $ordered = @($full | Sort-Object { (Get-Item -LiteralPath (Join-Path $feedPath $_.FileName)).LastWriteTimeUtc } -Descending)
    $keep = @($ordered | Select-Object -First 3 | ForEach-Object Version)
    $remove = @($releases.Assets | Where-Object { $_.Version -notin $keep })
    foreach ($asset in $remove) {
        if ($asset.FileName -notmatch '^[a-zA-Z0-9._-]+\.nupkg$') { throw 'Invalid package filename in feed.' }
    }
    $releases.Assets = @($releases.Assets | Where-Object { $_.Version -in $keep })
    [IO.File]::WriteAllText("$index.tmp", ($releases | ConvertTo-Json -Depth 20))
    Move-Item -LiteralPath "$index.tmp" -Destination $index -Force
    foreach ($asset in $remove) { Remove-Item -LiteralPath (Join-Path $feedPath $asset.FileName) -ErrorAction SilentlyContinue }
    $legacy = Join-Path $feedPath 'RELEASES'
    if (Test-Path -LiteralPath $legacy) {
        $names = @($releases.Assets | ForEach-Object FileName)
        $lines = @(Get-Content -LiteralPath $legacy | Where-Object { ($_ -split ' ')[1] -in $names })
        $lines | Set-Content -LiteralPath $legacy -Encoding ascii
    }
    Write-Output "Packed unsigned Sidenote $version into $feedPath. Kept $($keep.Count) full packages."
} finally { Pop-Location }
