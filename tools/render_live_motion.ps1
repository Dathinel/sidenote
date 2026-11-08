param(
    [string]$Exe = "$PSScriptRoot/../target/x86_64-pc-windows-gnu/debug/sidenote.exe",
    [string]$Out = "$PSScriptRoot/../artifacts/live-motion",
    [string]$Theme = 'dark',
    [string]$Size = '1320x800'
)
# Records the live transcript panel opening headlessly: the chevron in the recording pill is
# tapped and the panel rises above the bar. Frames are dumped as PNGs, tiled into a contact sheet
# (4 frames) and, when ffmpeg is on PATH, joined into an MP4 and a GIF. Rune dumps frames on its
# own virtual clock, so the timing is the same on every run.
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot/..").Path
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$crop = '420,330,740,470'
$clips = @(
    # name, scene, script, first frame time, frame count
    ,@('panel-open', 'live-panel-closed', 'live-panel-open.json', 0.20, 24)
)
foreach ($clip in $clips) {
    $name, $scene, $script, $start, $frames = $clip
    $state = Join-Path ([IO.Path]::GetTempPath()) "meetings-ui-state-motion-$name"
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $state
    New-Item -ItemType Directory -Force -Path $state | Out-Null
    Copy-Item "$root/tools/ui-fixtures/hex.json" "$state/hex-fixture.json"
    $meetings = "$root/tools/ui-fixtures/live-notes/meetings"
    $env:SIDENOTE_STATE_DIR = $state
    $dir = "$Out/$name-$Theme"
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $dir
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $common = @('--meetings-dir', $meetings, '--data-dir', $state, '--scene', $scene, '--theme', $Theme, '--size', $Size, '--scale', '1', '--script', "$root/tools/ui-scripts/$script", '--start', $start, '--frames', $frames, '--fps', 30, '--crop', $crop)
    $log = "$Out/render-motion.log"
    $run = { param($a) $p = Start-Process -FilePath $Exe -ArgumentList $a -WorkingDirectory $root -WindowStyle Hidden -RedirectStandardOutput "$log.out" -RedirectStandardError $log -Wait -PassThru; if ($p.ExitCode -ne 0) { throw "Motion render $name failed with exit $($p.ExitCode), see $log" } }
    & $run (@('--out', $dir) + $common)
    & $run (@('--strip', "$Out/$name-$Theme-strip.png", '--cols', 4, '--every', 6) + $common)
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
