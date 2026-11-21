function Invoke-LinuxBoundaryPreflight([string]$Root) {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    $python = Get-Command python.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $python) { throw 'Linux boundary preflight failed: python.exe is unavailable.' }

    $ErrorActionPreference = 'Continue'
    $result = & $python.Source -I (Join-Path $PSScriptRoot 'prove_linux_boundaries.py') $Root 2>$null
    if ($LASTEXITCODE -ne 0) {
        if (-not $result) { $result = 'boundary verifier exited without a result.' }
        throw "Linux boundary preflight failed: $result"
    }
    return ('Linux boundary preflight passed ({0:F2}s).' -f $timer.Elapsed.TotalSeconds)
}
