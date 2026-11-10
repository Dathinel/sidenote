param(
    [string]$Exe = "$PSScriptRoot/../target/x86_64-pc-windows-gnu/debug/sidenote.exe",
    [string]$Out = "$PSScriptRoot/../artifacts/annotations",
    [string[]]$Themes = @('light', 'dark')
)
$ErrorActionPreference = 'Stop'
$only = foreach ($scene in @('annotations-list', 'annotation-capture', 'annotations-empty', 'annotation-transcript','annotation-review','annotation-drafting','annotation-note')) {
    foreach ($width in @(1280, 640)) { "${scene}-${width}" }
}
& "$PSScriptRoot/render_ui.ps1" -Exe $Exe -Out $Out -Only $only -Themes $Themes -Trees -Throttle 2 -StateRoot "$PSScriptRoot/../target/annotations-render-state"
