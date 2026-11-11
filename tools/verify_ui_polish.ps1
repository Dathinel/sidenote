$ErrorActionPreference = 'Stop'
$env:PATH = 'E:\Personal-Projects\tools\hex-meetings-gnu\w64devkit\bin;' + $env:PATH
$env:CARGO_TARGET_DIR = Join-Path $PSScriptRoot '../target'
$out = Join-Path $PSScriptRoot '../artifacts/ui-polish/gates'
New-Item -ItemType Directory -Force $out | Out-Null
$gates = @(
    @{ Name = 'fmt'; Args = @('fmt','--all','--check') },
    @{ Name = 'clippy'; Args = @('clippy','--workspace','--locked','--all-targets','--','-D','warnings') },
    @{ Name = 'clippy-asr'; Args = @('clippy','--workspace','--locked','--all-targets','--features','asr-runtime','--','-D','warnings') },
    @{ Name = 'test'; Args = @('test','--workspace','--locked') },
    @{ Name = 'test-asr'; Args = @('test','--workspace','--locked','--features','asr-runtime') },
    @{ Name = 'build'; Args = @('build','--locked') }
)
$results = @()
foreach ($gate in $gates) {
    $began = Get-Date
    $arguments = $gate.Args
    $process = Start-Process -FilePath (Get-Command cargo).Source -ArgumentList $arguments -WindowStyle Hidden -RedirectStandardOutput (Join-Path $out ($gate.Name + '.stdout.log')) -RedirectStandardError (Join-Path $out ($gate.Name + '.log')) -PassThru
    $null = $process.Handle
    $process.WaitForExit()
    $code = $process.ExitCode
    $results += @{ name = $gate.Name; exit_code = $code; seconds = [math]::Round(((Get-Date)-$began).TotalSeconds,1) }
    $results | ConvertTo-Json | Set-Content (Join-Path $out 'results.json')
    Write-Output ($gate.Name + ': exit ' + $code)
    if ($code -ne 0) { exit $code }
}
$process = Start-Process -FilePath (Get-Command python).Source -ArgumentList @('tools/mcp-smoke.py','target/x86_64-pc-windows-gnu/debug/sidenote.exe') -WindowStyle Hidden -RedirectStandardOutput (Join-Path $out 'mcp-smoke.log') -RedirectStandardError (Join-Path $out 'mcp-smoke.stderr.log') -PassThru
$null = $process.Handle
$process.WaitForExit()
$code = $process.ExitCode
$results += @{ name = 'mcp-smoke'; exit_code = $code }
$results | ConvertTo-Json | Set-Content (Join-Path $out 'results.json')
Write-Output ('mcp-smoke: exit ' + $code)
exit $code
