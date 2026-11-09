param(
    [string]$Exe = "$PSScriptRoot/../target/x86_64-pc-windows-gnu/debug/sidenote.exe",
    [string]$Out = "$PSScriptRoot/../artifacts/ui/notes",
    [string[]]$Only = @(),
    [string[]]$Themes = @('light', 'dark'),
    [string]$IndexName = 'render-index.csv'
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot/..").Path
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$state = Join-Path ([IO.Path]::GetTempPath()) ("sidenote-notes-render-" + [IO.Path]::GetRandomFileName())
New-Item -ItemType Directory -Force -Path $state, "$state/appdata", "$state/localappdata", "$state/peers" | Out-Null
Copy-Item "$root/tools/ui-fixtures/hex.json" "$state/hex-fixture.json"
$utf8 = New-Object System.Text.UTF8Encoding($false)
[IO.File]::WriteAllText("$state/config.json", '{"version":1,"features":{"tasks":true,"memory":false,"daily_check":false,"projects":true,"other_devices":false}}', $utf8)
[IO.File]::WriteAllText("$state/projects.json", '{"projects":["Northwind Atlas"],"aliases":{}}', $utf8)
$priorAppData = $env:APPDATA
$priorLocalAppData = $env:LOCALAPPDATA
$priorState = $env:SIDENOTE_STATE_DIR
$rows = @()
try {
    $env:APPDATA = "$state/appdata"
    $env:LOCALAPPDATA = "$state/localappdata"
    $env:SIDENOTE_STATE_DIR = $state
    $source = Get-Content "$root/src/ui/mod.rs" -Raw
    $scenes = [regex]::Matches($source, '"(notes-(?:settings|meeting|live|today|list)-[^" ]+)"') | ForEach-Object { $_.Groups[1].Value } | Select-Object -Unique
    $shots = @($scenes | ForEach-Object { [PSCustomObject]@{ Name = $_; Scene = $_; Script = $null } })
    foreach ($interaction in @(
        @('notes-click-dismiss', 'notes-meeting-fallback', 'notes-dismiss.json'),
        @('notes-click-reset', 'notes-settings-edited', 'notes-reset-confirm.json'),
        @('notes-click-type', 'notes-settings-edited', 'notes-type.json'),
        @('notes-click-fixed', 'notes-settings-tasks', 'notes-show-fixed.json'),
        @('notes-click-privacy', 'notes-today-found', 'notes-privacy.json')
    )) { $shots += [PSCustomObject]@{ Name = $interaction[0]; Scene = $interaction[1]; Script = $interaction[2] } }
    foreach ($shot in $shots) {
        $name = $shot.Name
        $scene = $shot.Scene
        if ($Only.Count -gt 0 -and $Only -notcontains $name) { continue }
        foreach ($theme in $Themes) {
            foreach ($width in @(1320, 560)) {
                $height = if ($scene -like 'notes-settings-fixed-*' -or $name -eq 'notes-click-fixed') { 3000 } elseif ($scene -like 'notes-list-*') { 1500 } elseif ($scene -like 'notes-settings-*') { 1400 } else { 1000 }
                $png = Join-Path $Out "$name-$width-$theme.png"
                $log = Join-Path $Out "$name-$width-$theme.log"
                $arguments = @('--data-dir', $state, '--meetings-dir', "$root/tools/ui-fixtures/screens/meetings", '--peers-dir', "$state/peers", '--snap', $png, '--scene', $scene, '--theme', $theme, '--size', "${width}x${height}", '--scale', '1')
                if ($shot.Script) { $arguments += @('--script', "$root/tools/ui-scripts/$($shot.Script)", '--at', 'mark:end') }
                $process = Start-Process -FilePath $Exe -ArgumentList $arguments -WorkingDirectory $root -WindowStyle Hidden -RedirectStandardOutput "$log.out" -RedirectStandardError $log -Wait -PassThru
                if ($process.ExitCode -ne 0) { throw "Render $scene-$width-$theme failed with exit $($process.ExitCode), see $log" }
                $rows += [PSCustomObject]@{ Scene = $name; Width = $width; Theme = $theme; Path = $png }
                Write-Output $png
            }
        }
    }
    $rows | Export-Csv -NoTypeInformation -Encoding UTF8 (Join-Path $Out $IndexName)
} finally {
    $env:APPDATA = $priorAppData
    $env:LOCALAPPDATA = $priorLocalAppData
    $env:SIDENOTE_STATE_DIR = $priorState
    $resolved = [IO.Path]::GetFullPath($state)
    $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    if ($resolved.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -and (Split-Path $resolved -Leaf).StartsWith('sidenote-notes-render-')) {
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}
