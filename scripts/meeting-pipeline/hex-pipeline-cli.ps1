$script:PipelineStages = @('pipeline', 'prepare', 'transcribe', 'translate', 'notes', 'render', 'publish', 'tasks', 'compare', 'title')

function Format-PipelineFailure([string]$Stage, [string]$Category, [int]$DeadlineSeconds, [double]$ElapsedSeconds, [string]$Kind = 'other', $ExitCode = $null) {
    if ($Stage -cnotin $script:PipelineStages) { $Stage = 'pipeline' }
    if ($Category -cnotin @('HELPER_LIMIT', 'HELPER_AUTH', 'timeout', 'other')) { $Category = 'other' }
    if ($Kind -cnotin @('git','claude','codex','python','opencode','other')) { $Kind = 'other' }
    $exitText = if ($ExitCode -is [int] -or $ExitCode -is [long]) { [string]$ExitCode } else { 'none' }
    $DeadlineSeconds = [Math]::Min(3600, [Math]::Max(0, $DeadlineSeconds))
    if ([double]::IsNaN($ElapsedSeconds) -or [double]::IsInfinity($ElapsedSeconds)) { $ElapsedSeconds = 0 }
    $elapsed = [Math]::Min(86400, [Math]::Max(0, $ElapsedSeconds)).ToString('0.000', [Globalization.CultureInfo]::InvariantCulture)
    "pipeline-failure stage=$Stage category=$Category kind=$Kind exit_code=$exitText deadline_seconds=$DeadlineSeconds elapsed_seconds=$elapsed"
}

function Get-PipelineFailure($Record, [string]$Stage = 'pipeline', [int]$DeadlineSeconds = 0, [double]$ElapsedSeconds = 0) {
    $message = [string]$Record.Exception.Message
    $pattern = '\Apipeline-failure stage=(pipeline|prepare|transcribe|translate|notes|render|publish|tasks|compare|title) category=(HELPER_LIMIT|HELPER_AUTH|timeout|other) kind=(git|claude|codex|python|opencode|other) exit_code=(none|-?[0-9]{1,10}) deadline_seconds=([0-9]{1,4}) elapsed_seconds=([0-9]{1,5}\.[0-9]{3})\z'
    if ($message -cmatch $pattern) {
        $exitCode = if ($Matches[4] -ceq 'none') { $null } else { [long]$Matches[4] }
        return Format-PipelineFailure $Matches[1] $Matches[2] ([int]$Matches[5]) ([double]::Parse($Matches[6], [Globalization.CultureInfo]::InvariantCulture)) $Matches[3] $exitCode
    }
    Format-PipelineFailure $Stage 'other' $DeadlineSeconds $ElapsedSeconds
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
            $safe = Format-PipelineFailure $Stage 'other' $TimeoutSeconds $clock.Elapsed.TotalSeconds
            if ((Test-Path -LiteralPath $diagnostic -PathType Leaf) -and (Get-Item -LiteralPath $diagnostic).Length -le 1024) {
                try {
                    $data = [IO.File]::ReadAllText($diagnostic) | ConvertFrom-Json
                    if (@($data.PSObject.Properties).Count -eq 6 -and $data.stage -ceq $Stage -and
                        $data.kind -cin @('git','claude','codex','python','opencode','other') -and
                        ($null -eq $data.exit_code -or $data.exit_code -is [int] -or $data.exit_code -is [long]) -and
                        $data.category -cin @('HELPER_LIMIT','HELPER_AUTH','timeout','other') -and $data.deadline_seconds -eq $TimeoutSeconds -and
                        $data.elapsed_seconds -is [ValueType] -and $data.elapsed_seconds -ge 0 -and $data.elapsed_seconds -le 86400) {
                        $safe = Format-PipelineFailure $Stage $data.category $TimeoutSeconds ([double]$data.elapsed_seconds) $data.kind $data.exit_code
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

function Enter-NotesEngine([string]$MeetingDir) {
    # FileShare.None matches store_io::lock. The OS expires ownership on process death.
    try {
        return [IO.File]::Open((Join-Path $MeetingDir '.notes-engine.lock'),
            [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    } catch [IO.IOException] {
        if (($_.Exception.HResult -band 0xffff) -in @(32,33)) { return $null }
        throw
    }
}

function Get-ScriptsNotesEngine {
    try {
        $state = if ($env:SIDENOTE_DATA_DIR) { $env:SIDENOTE_DATA_DIR }
                 elseif ($env:SIDENOTE_STATE_DIR) { $env:SIDENOTE_STATE_DIR }
                 else { Join-Path $env:APPDATA 'hex-meetings' }
        $config = [IO.File]::ReadAllText((Join-Path $state 'config.json')) | ConvertFrom-Json
        if ($config.notes.engine -cin @('app','scripts','off')) { return [string]$config.notes.engine }
    } catch {} # Missing or unreadable settings never authorize scripts.
    return 'off'
}
