param([Parameter(Mandatory)][string]$MeetingDir)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'hex-pipeline-cli.ps1')
Start-Sleep -Seconds 300
$work = Join-Path ([IO.Path]::GetTempPath()) ('hex-compare-log-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $work | Out-Null
try {
    Invoke-HexCli -CliArgs @('python','-I',(Join-Path $PSScriptRoot 'hex-meeting-compare-granola.py'),$MeetingDir) -OutputFile (Join-Path $work 'status.txt') -TimeoutSeconds 1800
    Write-Host 'Granola comparison finished'
} catch {
    Write-Host "Granola comparison failed; diagnostic bytes=$([Text.Encoding]::UTF8.GetByteCount([string]$_))"
} finally { Remove-Item -LiteralPath $work -Recurse -Force }
