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
    & cargo build --release --locked --bin sidenote
    if ($LASTEXITCODE) { throw 'Release build failed.' }
    $stage = "$root/artifacts/updates-core/package-$version"
    New-Item -ItemType Directory -Force -Path $stage, $feedPath | Out-Null
    Copy-Item -LiteralPath "$env:CARGO_TARGET_DIR/x86_64-pc-windows-gnu/release/sidenote.exe" -Destination "$stage/sidenote.exe" -Force
    $notices = @('LICENSE', 'THIRD_PARTY.md', 'assets/fonts/Inter-LICENSE.txt',
        'assets/licenses/webpki-roots-CDLA-Permissive-2.0.txt')
    foreach ($notice in $notices) {
        $destination = Join-Path $stage $notice
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $destination) | Out-Null
        Copy-Item -LiteralPath (Join-Path $root $notice) -Destination $destination -Force
    }
    # Include cached upstream notices, not just the licence-name inventory.
    $metadataJson = & cargo metadata --offline --locked --all-features --format-version 1
    if ($LASTEXITCODE) { throw 'Could not resolve locked dependency notices from the Cargo cache.' }
    $metadata = ($metadataJson -join "`n") | ConvertFrom-Json
    $inventory = @('# Dependency notices', '', 'Upstream notices for the locked Cargo dependency inventory.', '')
    foreach ($package in $metadata.packages | Sort-Object name, version) {
        if ($package.id -in $metadata.workspace_members) { continue }
        $packageRoot = Split-Path -Parent $package.manifest_path
        $sourceRoot = $packageRoot
        if ($package.source -like 'git+*') {
            $checkout = & git -C $packageRoot rev-parse --show-toplevel
            if ($LASTEXITCODE) { throw "Could not locate upstream Git notices for $($package.name)." }
            $sourceRoot = [IO.Path]::GetFullPath($checkout)
        }
        $noticeRoot = Join-Path $stage "dependency-notices/$($package.name)-$($package.version)"
        $files = @(Get-ChildItem -LiteralPath $sourceRoot -Recurse -File | Where-Object {
            $_.FullName.Substring($sourceRoot.Length) -match '(?i)[\\/](licen[cs]es?|copying|copyright|notices?)([^\\/]*$|[\\/])'
        })
        if ($package.license_file) {
            $declared = if ([IO.Path]::IsPathRooted($package.license_file)) {
                [IO.Path]::GetFullPath($package.license_file)
            } else {
                [IO.Path]::GetFullPath((Join-Path $packageRoot $package.license_file))
            }
            if (-not $declared.StartsWith($sourceRoot.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) {
                throw "Licence file is outside the dependency source: $($package.name)"
            }
            $files += Get-Item -LiteralPath $declared
        }
        foreach ($file in $files | Sort-Object FullName -Unique) {
            $relative = $file.FullName.Substring($sourceRoot.Length).TrimStart('\', '/')
            $destination = Join-Path $noticeRoot $relative
            New-Item -ItemType Directory -Force -Path (Split-Path -Parent $destination) | Out-Null
            Copy-Item -LiteralPath $file.FullName -Destination $destination -Force
        }
        $inventory += "- $($package.name) $($package.version): $($package.license); $($files.Count) notice file(s)."
        if ($files.Count -eq 0) { throw "No cached upstream notices found for $($package.name) $($package.version)." }
    }
    $inventory | Set-Content -LiteralPath (Join-Path $stage 'DEPENDENCY_NOTICES.md') -Encoding utf8
    $packArgs = @('pack', '--packId', 'sidenote', '--packVersion', $version, '--packDir', $stage,
        '--mainExe', 'sidenote.exe', '--icon', "$root/assets/icon/meetings.ico", '--packTitle', 'Sidenote', '--outputDir', $feedPath, '--shortcuts', 'None')
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
