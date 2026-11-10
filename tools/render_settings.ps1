param(
    [string]$Exe = "$PSScriptRoot/../target/x86_64-pc-windows-gnu/debug/sidenote.exe",
    [string]$Out = "$PSScriptRoot/../artifacts/settings",
    [string[]]$Themes = @('light', 'dark')
)
$ErrorActionPreference = 'Stop'
$scenes = @('general','recording','summaries','tasks','prompts','people','hex','helpers','speech','helper-advanced','appearance','sync','updates','search','sidebar-open','sidebar-folded','recording-expanded','summaries-expanded','tasks-expanded','hex-expanded','helper-advanced-expanded','sync-expanded','updates-expanded','helpers-expanded')
$only = foreach ($scene in $scenes) { foreach ($width in @(1280,640)) { "settings-${scene}-${width}" } }
$only += @('settings-review-sidebar-slash','settings-review-settings-slash')
& "$PSScriptRoot/render_ui.ps1" -Exe $Exe -Out $Out -Only $only -Themes $Themes -Trees -Throttle 2 -StateRoot "$PSScriptRoot/../target/settings-render-state"
