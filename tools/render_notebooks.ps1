param(
 [string]$Exe="$PSScriptRoot/../target/x86_64-pc-windows-gnu/debug/sidenote.exe",
 [string]$Out="$PSScriptRoot/../artifacts/notebooks"
)
$ErrorActionPreference='Stop'
$only=foreach($scene in @('notebooks','notebook','notebook-note','share-confirmation')){foreach($width in @(1280,640)){"${scene}-${width}"}}
& "$PSScriptRoot/render_ui.ps1" -Exe $Exe -Out $Out -Only $only -Themes light,dark -Trees -Throttle 2 -StateRoot "$PSScriptRoot/../target/nbr"
