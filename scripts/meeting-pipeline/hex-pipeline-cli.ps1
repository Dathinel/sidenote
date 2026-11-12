function Invoke-HexCli {
    param([string]$Kind = 'native', [string[]]$CliArgs, [string]$InputFile,
          [string]$OutputFile, [string]$Model, [int]$TimeoutSeconds = 240, [string]$WorkingDirectory)
    $requestDir = Join-Path ([IO.Path]::GetTempPath()) ('hex-cli-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $requestDir | Out-Null
    try {
        $request = Join-Path $requestDir 'request.json'
        $body = @{kind=$Kind; args=@($CliArgs); input_file=$InputFile; output_file=$OutputFile;
                  model=$Model; timeout=$TimeoutSeconds; cwd=$WorkingDirectory} | ConvertTo-Json -Depth 5
        [IO.File]::WriteAllText($request, $body, (New-Object System.Text.UTF8Encoding($false)))
        $old = $ErrorActionPreference
        try {
            $ErrorActionPreference = 'Continue'
            $diagnostic = & python -I (Join-Path $PSScriptRoot 'hex-pipeline-run.py') --request $request 2>&1
            $code = $LASTEXITCODE
        } finally { $ErrorActionPreference = $old }
        if ($code -ne 0) { throw "Pipeline CLI failed (exit $code); diagnostic bytes=$([Text.Encoding]::UTF8.GetByteCount(($diagnostic -join "`n")))" }
    } finally { Remove-Item -LiteralPath $requestDir -Recurse -Force }
}

function Read-HexPrompt([string]$Path) {
    $text = [IO.File]::ReadAllText($Path, (New-Object Text.UTF8Encoding($false)))
    foreach ($key in @('OWNER_NAME', 'COMPANY_NAME', 'PRODUCT_NAME')) {
        $value = [Environment]::GetEnvironmentVariable("HEX_MEETING_$key")
        if (-not $value) { $value = if ($key -eq 'OWNER_NAME') { 'You' } else { 'the configured organization' } }
        $text = $text.Replace("{{$key}}", $value)
    }
    return $text
}
