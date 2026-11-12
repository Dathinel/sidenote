param([string[]]$MeetingFolders)
$ErrorActionPreference = 'Stop'
$clock = [Diagnostics.Stopwatch]::StartNew()
$category = 'missing'
try {
    $pointerPath = Join-Path $PSScriptRoot '.pipeline-current.json'
    if ((Get-Item -LiteralPath $pointerPath).Length -gt 2048) { throw 'Invalid pointer' }
    $pointer = [IO.File]::ReadAllText($pointerPath) | ConvertFrom-Json
    $selected = $pointer.current
    if ($pointer.schema -cne 'sidenote.pipeline/1' -or
        $selected.generation -cnotmatch '\A(?:generation|legacy)-[0-9a-f]{32}\z' -or
        $selected.manifest_sha256 -cnotmatch '\A[0-9a-f]{64}\z') { throw 'Invalid pointer' }
    # Resolve once; every child uses this immutable generation's script directory.
    $generation = Join-Path (Join-Path $PSScriptRoot '.pipeline-generations') $selected.generation
    $manifestPath = Join-Path $generation '.generation.json'
    if ((Get-Item -LiteralPath $manifestPath).Length -gt 8192) { throw 'Invalid manifest' }
    $category = 'exit'
    if ((Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash -ine $selected.manifest_sha256) { throw 'Hash mismatch' }
    $manifest = [IO.File]::ReadAllText($manifestPath) | ConvertFrom-Json
    if ($manifest.schema -cne 'sidenote.pipeline/1' -or @($manifest.files.PSObject.Properties).Count -ne 14) { throw 'Invalid manifest' }
    foreach ($file in $manifest.files.PSObject.Properties) {
        if ($file.Name -cnotmatch '\A[a-z][a-z0-9-]*\.(?:ps1|py|md)\z' -or $file.Value -cnotmatch '\A[0-9a-f]{64}\z') { throw 'Invalid manifest' }
        $path = Join-Path $generation $file.Name
        $category = 'missing'
        if (-not [IO.File]::Exists($path)) { throw 'Generation incomplete' }
        $category = 'exit'
        if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ine $file.Value) { throw 'Hash mismatch' }
    }
    $entry = Join-Path $generation 'hex-meeting-watch.ps1'
    $category = 'missing'
    if (-not [IO.File]::Exists($entry)) { throw 'Entry missing' }
} catch {
    $elapsed = $clock.Elapsed.TotalSeconds.ToString('0.000', [Globalization.CultureInfo]::InvariantCulture)
    throw "pipeline-failure stage=pipeline category=$category deadline_seconds=0 elapsed_seconds=$elapsed"
}
& $entry @PSBoundParameters @args
