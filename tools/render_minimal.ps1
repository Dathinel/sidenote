param([string]$Exe = "$PSScriptRoot/../target/minimal-clean/x86_64-pc-windows-gnu/debug/sidenote.exe")
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot/..").Path
$artifacts = "$root/artifacts"
New-Item -ItemType Directory -Force -Path $artifacts | Out-Null
foreach ($theme in @('light','dark')) {
    $arguments = @('--snap',"$artifacts/minimal-$theme.png",'--tree',"$artifacts/minimal-$theme.json",'--theme',$theme,'--size','640x400','--scale','1')
    $process = Start-Process -FilePath $Exe -ArgumentList $arguments -WorkingDirectory $root -WindowStyle Hidden -RedirectStandardOutput "$artifacts/minimal-$theme.out.log" -RedirectStandardError "$artifacts/minimal-$theme.err.log" -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "Minimal render $theme failed with exit $($process.ExitCode)" }
    $tree = Get-Content -LiteralPath "$artifacts/minimal-$theme.json" -Raw -Encoding utf8 | ConvertFrom-Json
    $texts = @($tree.elements | Where-Object role -eq 'text')
    if ($texts.Count -ne 1 -or $texts[0].text -ne 'Meetings') { throw 'The proof window must have exactly one text line' }
    Write-Output "Verified minimal-$theme.png, exactly one text line"
}
