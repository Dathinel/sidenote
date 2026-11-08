param(
    [string]$Exe = "$PSScriptRoot/../target/x86_64-pc-windows-gnu/debug/sidenote.exe",
    [string]$Out = "$PSScriptRoot/../artifacts/ask-motion",
    [string]$Theme = 'dark',
    [string]$Size = '1320x800'
)
# Records the Ask bar's motion headlessly: the bar gaining focus (chips fade and slide up), then
# Esc folding it back. Each clip is dumped as PNG frames, tiled into a contact sheet and, when
# ffmpeg is on PATH, joined into an MP4 and a GIF. Frames are cropped to the bar's corner of the
# window. Rune dumps frames on its own virtual clock, so the timing is the same on every run.
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot/..").Path
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$crop = '420,590,740,210'
$clips = @(
    # name, scene, script, first frame time, frame count
    @('focus',    'ask-idle',    'ask-focus.json',    0.20, 24),
    @('collapse', 'ask-focused', 'ask-collapse.json', 0.20, 24)
)
foreach ($clip in $clips) {
    $name, $scene, $script, $start, $frames = $clip
    $state = Join-Path ([IO.Path]::GetTempPath()) "meetings-ui-state-motion-$name"
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $state
    New-Item -ItemType Directory -Force -Path $state | Out-Null
    Copy-Item "$root/tools/ui-fixtures/hex.json" "$state/hex-fixture.json"
    $env:SIDENOTE_STATE_DIR = $state
    $dir = "$Out/$name-$Theme"
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $dir
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $common = @('--meetings-dir', "$root/tools/ui-fixtures/meetings", '--data-dir', $state, '--scene', $scene, '--theme', $Theme, '--size', $Size, '--scale', '1', '--script', "$root/tools/ui-scripts/$script", '--start', $start, '--frames', $frames, '--fps', 30, '--crop', $crop)
    $log = "$Out/render-motion.log"
    $run = { param($a) $p = Start-Process -FilePath $Exe -ArgumentList $a -WorkingDirectory $root -WindowStyle Hidden -RedirectStandardOutput "$log.out" -RedirectStandardError $log -Wait -PassThru; if ($p.ExitCode -ne 0) { throw "Motion render $name failed with exit $($p.ExitCode), see $log" } }
    & $run (@('--out', $dir) + $common)
    & $run (@('--strip', "$Out/$name-$Theme-strip.png", '--cols', 4, '--every', 3) + $common)
    if (Get-Command ffmpeg -ErrorAction SilentlyContinue) {
        $pattern = (Get-ChildItem $dir -Filter *.png | Sort-Object Name | Select-Object -First 1).Name -replace '\d+', '%0*d'
        $digits = ((Get-ChildItem $dir -Filter *.png | Select-Object -First 1).BaseName -replace '\D', '').Length
        $pattern = ((Get-ChildItem $dir -Filter *.png | Select-Object -First 1).Name -replace '\d+', "%0${digits}d")
        & ffmpeg -y -loglevel error -framerate 30 -i "$dir/$pattern" -vf 'scale=trunc(iw/2)*2:trunc(ih/2)*2' -pix_fmt yuv420p "$Out/$name-$Theme.mp4"
        & ffmpeg -y -loglevel error -framerate 30 -i "$dir/$pattern" -vf 'split[a][b];[a]palettegen[p];[b][p]paletteuse' "$Out/$name-$Theme.gif"
    }
    Write-Output "$Out/$name-$Theme-strip.png"
}
Remove-Item Env:SIDENOTE_STATE_DIR -ErrorAction SilentlyContinue
