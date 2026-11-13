param([Parameter(Mandatory)][string]$MeetingDir)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'hex-pipeline-cli.ps1')
$clock = [Diagnostics.Stopwatch]::StartNew()
Start-Sleep -Seconds 300
$work = Join-Path ([IO.Path]::GetTempPath()) ('hex-compare-log-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $work | Out-Null
try {
    Invoke-HexCli -CliArgs @('python','-I',(Join-Path $PSScriptRoot 'hex-meeting-compare-granola.py'),$MeetingDir) -TimeoutSeconds 1800 -Stage compare
    Write-Host 'Granola comparison finished'
} catch {
    Write-Host (Get-PipelineFailure $_ 'compare' 1800 $clock.Elapsed.TotalSeconds)
} finally { Remove-Item -LiteralPath $work -Recurse -Force }
