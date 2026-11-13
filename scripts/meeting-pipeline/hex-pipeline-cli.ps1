$script:PipelineStages = @('pipeline', 'prepare', 'transcribe', 'translate', 'notes', 'render', 'publish', 'tasks', 'compare', 'title')

function Format-PipelineFailure([string]$Stage, [string]$Category, [int]$DeadlineSeconds, [double]$ElapsedSeconds) {
    if ($Stage -cnotin $script:PipelineStages) { $Stage = 'pipeline' }
    if ($Category -cnotin @('timeout', 'exit', 'missing')) { $Category = 'exit' }
    $DeadlineSeconds = [Math]::Min(3600, [Math]::Max(0, $DeadlineSeconds))
    if ([double]::IsNaN($ElapsedSeconds) -or [double]::IsInfinity($ElapsedSeconds)) { $ElapsedSeconds = 0 }
    $elapsed = [Math]::Min(86400, [Math]::Max(0, $ElapsedSeconds)).ToString('0.000', [Globalization.CultureInfo]::InvariantCulture)
    "pipeline-failure stage=$Stage category=$Category deadline_seconds=$DeadlineSeconds elapsed_seconds=$elapsed"
}

function Get-PipelineFailure($Record, [string]$Stage = 'pipeline', [int]$DeadlineSeconds = 0, [double]$ElapsedSeconds = 0) {
    $message = [string]$Record.Exception.Message
    $pattern = '\Apipeline-failure stage=(pipeline|prepare|transcribe|translate|notes|render|publish|tasks|compare|title) category=(timeout|exit|missing) deadline_seconds=([0-9]{1,4}) elapsed_seconds=([0-9]{1,5}\.[0-9]{3})\z'
    if ($message -cmatch $pattern) {
        return Format-PipelineFailure $Matches[1] $Matches[2] ([int]$Matches[3]) ([double]::Parse($Matches[4], [Globalization.CultureInfo]::InvariantCulture))
    }
    $category = 'exit'; $exception = $Record.Exception
    while ($exception) {
        if ($exception -is [IO.FileNotFoundException] -or $exception -is [IO.DirectoryNotFoundException] -or
            $exception -is [Management.Automation.CommandNotFoundException] -or $exception -is [Management.Automation.ItemNotFoundException]) {
            $category = 'missing'; break
        }
        $exception = $exception.InnerException
    }
    Format-PipelineFailure $Stage $category $DeadlineSeconds $ElapsedSeconds
}

function Invoke-HexCli {
    param([ValidateSet('native','translate','claude')][string]$Kind = 'native', [string[]]$CliArgs, [string]$InputFile,
          [string]$OutputFile, [string]$Model, [ValidateRange(1,3600)][int]$TimeoutSeconds = 240, [string]$WorkingDirectory,
          [ValidateSet('pipeline','prepare','transcribe','translate','notes','render','publish','tasks','compare','title')][string]$Stage = 'pipeline')
    if ($Stage -eq 'pipeline') {
        if ($Kind -eq 'translate') { $Stage = 'translate' }
        elseif ($Kind -eq 'claude') { $Stage = 'notes' }
    }
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $requestDir = Join-Path ([IO.Path]::GetTempPath()) ('hex-cli-' + [guid]::NewGuid().ToString('N'))
    try {
        New-Item -ItemType Directory -Path $requestDir | Out-Null
        $request = Join-Path $requestDir 'request.json'
        $diagnostic = Join-Path $requestDir 'failure.json'
        $body = @{kind=$Kind; stage=$Stage; args=@($CliArgs); input_file=$InputFile; output_file=$OutputFile;
                  model=$Model; timeout=$TimeoutSeconds; cwd=$WorkingDirectory} | ConvertTo-Json -Depth 5
        [IO.File]::WriteAllText($request, $body, (New-Object System.Text.UTF8Encoding($false)))
        $python = Get-Command python -CommandType Application -ErrorAction Stop | Select-Object -First 1
        $runner = Join-Path $PSScriptRoot 'hex-pipeline-run.py'
        if (-not (Test-Path -LiteralPath $runner -PathType Leaf)) { throw [IO.FileNotFoundException]::new('Pipeline runner missing') }
        $old = $ErrorActionPreference
        try {
            $ErrorActionPreference = 'Continue'
            & $python.Source -I $runner --request $request --diagnostic $diagnostic 2>&1 | Out-Null
            $code = $LASTEXITCODE
        } finally { $ErrorActionPreference = $old }
        if ($code -ne 0) {
            $safe = Format-PipelineFailure $Stage 'exit' $TimeoutSeconds $clock.Elapsed.TotalSeconds
            if ((Test-Path -LiteralPath $diagnostic -PathType Leaf) -and (Get-Item -LiteralPath $diagnostic).Length -le 1024) {
                try {
                    $data = [IO.File]::ReadAllText($diagnostic) | ConvertFrom-Json
                    if (@($data.PSObject.Properties).Count -eq 4 -and $data.stage -ceq $Stage -and
                        $data.category -cin @('timeout','exit','missing') -and $data.deadline_seconds -eq $TimeoutSeconds -and
                        $data.elapsed_seconds -is [ValueType] -and $data.elapsed_seconds -ge 0 -and $data.elapsed_seconds -le 86400) {
                        $safe = Format-PipelineFailure $Stage $data.category $TimeoutSeconds ([double]$data.elapsed_seconds)
                    }
                } catch {} # Malformed diagnostics become fixed wrapper metadata, never raw text.
            }
            throw $safe
        }
    } catch {
        throw (Get-PipelineFailure $_ $Stage $TimeoutSeconds $clock.Elapsed.TotalSeconds)
    } finally {
        if (Test-Path -LiteralPath $requestDir) { Remove-Item -LiteralPath $requestDir -Recurse -Force -ErrorAction SilentlyContinue }
    }
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
