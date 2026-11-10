param(
    [string]$Feed = (Join-Path $PSScriptRoot '../artifacts/releases'),
    [string]$DotnetRoot = $env:DOTNET_ROOT,
    [string]$GnuBin = $env:RUST_GNU_BIN,
    [string]$ReleaseNotes = ''
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot/..").Path
$feedPath = [IO.Path]::GetFullPath($Feed)
if ($Feed -match '^[a-z]+://' -or -not [IO.Path]::IsPathRooted($Feed)) { throw 'Feed must be an absolute local folder or file share.' }
if ($DotnetRoot) {
    $env:DOTNET_ROOT = $DotnetRoot
    $env:PATH = "$DotnetRoot;$DotnetRoot/tools;$env:PATH"
}
$env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
$env:PATH = "$env:USERPROFILE/.cargo/bin;$env:PATH"
if ($GnuBin) { $env:PATH = "$GnuBin;$env:PATH" }
if (-not (Get-Command as.exe -ErrorAction SilentlyContinue)) { throw 'Set RUST_GNU_BIN to a full MinGW GNU bin folder, or put it on PATH.' }
(Get-Process -Id $PID).PriorityClass = 'BelowNormal'
Push-Location -LiteralPath $root
try {
    $version = [regex]::Match((Get-Content Cargo.toml -Raw), '(?m)^version = "([^"]+)"').Groups[1].Value
    if (-not $version) { throw 'Cargo.toml has no package version.' }
    $metadataJson = & cargo metadata --offline --locked --filter-platform x86_64-pc-windows-gnu --format-version 1
    if ($LASTEXITCODE) { throw 'Could not resolve locked dependency notices from the Cargo cache.' }
    $metadata = ($metadataJson -join "`n") | ConvertFrom-Json
    $stage = "$root/artifacts/updates-core/package-$version-$([guid]::NewGuid().ToString('N'))"
    New-Item -ItemType Directory -Force -Path $stage | Out-Null
    . (Join-Path $PSScriptRoot 'dependency_notices.ps1')
    $noticesResult = Write-DependencyNotices -Metadata $metadata -Destination $stage
    if ($noticesResult.uncovered.Count) {
        $uncovered = @($noticesResult.uncovered | ForEach-Object { "$($_.name) $($_.version): $($_.license) ($($_.reason))" })
        throw "Dependency notices are incomplete: $($uncovered -join '; '). Coverage: $stage/DEPENDENCY_NOTICE_COVERAGE.json"
    }
    & cargo build --release --locked --bin sidenote
    if ($LASTEXITCODE) { throw 'Release build failed.' }
    New-Item -ItemType Directory -Force -Path $feedPath | Out-Null
    Copy-Item -LiteralPath "$($metadata.target_directory)/x86_64-pc-windows-gnu/release/sidenote.exe" -Destination "$stage/sidenote.exe" -Force
    $notices = @('LICENSE', 'THIRD_PARTY.md', 'assets/fonts/Inter-LICENSE.txt',
        'assets/licenses/webpki-roots-CDLA-Permissive-2.0.txt', 'assets/licenses/lucide-LICENSE.txt')
    foreach ($notice in $notices) {
        $destination = Join-Path $stage $notice
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $destination) | Out-Null
        Copy-Item -LiteralPath (Join-Path $root $notice) -Destination $destination -Force
    }
    $packArgs = @('pack', '--packId', 'sidenote', '--packVersion', $version, '--packDir', $stage,
        '--mainExe', 'sidenote.exe', '--icon', "$root/assets/icon/meetings.ico", '--packTitle', 'Sidenote', '--outputDir', $feedPath, '--shortcuts', 'StartMenuRoot')
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
