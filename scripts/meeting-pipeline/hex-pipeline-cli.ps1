$script:JobContract = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'job-contract.json')) | ConvertFrom-Json
if ($script:JobContract.version -ne 1) { throw 'Unsupported meeting job contract' }
$script:PipelineStages = @($script:JobContract.stages)

function Format-PipelineFailure([string]$Stage, [string]$Category, [int]$DeadlineSeconds, [double]$ElapsedSeconds, [string]$Kind = 'other', $ExitCode = $null) {
    if ($Stage -cnotin $script:PipelineStages) { $Stage = 'pipeline' }
    if ($Category -cnotin $script:JobContract.categories) { $Category = 'other' }
    if ($Kind -cnotin @('git','claude','codex','python','opencode','other')) { $Kind = 'other' }
    $exitText = if ($ExitCode -is [int] -or $ExitCode -is [long]) { [string]$ExitCode } else { 'none' }
    $DeadlineSeconds = [Math]::Min(3600, [Math]::Max(0, $DeadlineSeconds))
    if ([double]::IsNaN($ElapsedSeconds) -or [double]::IsInfinity($ElapsedSeconds)) { $ElapsedSeconds = 0 }
    $elapsed = [Math]::Min(86400, [Math]::Max(0, $ElapsedSeconds)).ToString('0.000', [Globalization.CultureInfo]::InvariantCulture)
    "pipeline-failure stage=$Stage category=$Category kind=$Kind exit_code=$exitText deadline_seconds=$DeadlineSeconds elapsed_seconds=$elapsed"
}

function Get-PipelineFailure($Record, [string]$Stage = 'pipeline', [int]$DeadlineSeconds = 0, [double]$ElapsedSeconds = 0) {
    $message = [string]$Record.Exception.Message
    $stages = ($script:PipelineStages | ForEach-Object { [regex]::Escape($_) }) -join '|'
    $categories = ($script:JobContract.categories | ForEach-Object { [regex]::Escape($_) }) -join '|'
    $pattern = '\Apipeline-failure stage=(' + $stages + ') category=(' + $categories + ') kind=(git|claude|codex|python|opencode|other) exit_code=(none|-?[0-9]{1,10}) deadline_seconds=([0-9]{1,4}) elapsed_seconds=([0-9]{1,5}\.[0-9]{3})\z'
    if ($message -cmatch $pattern) {
        $exitCode = if ($Matches[4] -ceq 'none') { $null } else { [long]$Matches[4] }
        return Format-PipelineFailure $Matches[1] $Matches[2] ([int]$Matches[5]) ([double]::Parse($Matches[6], [Globalization.CultureInfo]::InvariantCulture)) $Matches[3] $exitCode
    }
    Format-PipelineFailure $Stage 'other' $DeadlineSeconds $ElapsedSeconds
}

function Invoke-HexCli {
    param([ValidateSet('native','translate','claude')][string]$Kind = 'native', [string[]]$CliArgs, [string]$InputFile,
          [string]$OutputFile, [string]$Model, [ValidateRange(1,3600)][int]$TimeoutSeconds = 240, [string]$WorkingDirectory,
          [string]$Stage = 'pipeline')
    if ($Stage -cnotin $script:PipelineStages) { throw 'Invalid meeting job stage' }
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
                        $data.category -cin $script:JobContract.categories -and $data.deadline_seconds -eq $TimeoutSeconds -and
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

function Enter-MeetingRecycleClaim([string]$MeetingDir) {
    $id = [IO.Path]::GetFileName([IO.Path]::GetFullPath($MeetingDir))
    if ($id -cnotmatch '\A(?=.*[0-9])[0-9-]{1,100}\z') { return [IO.MemoryStream]::new() }
    $data = if ($env:SIDENOTE_DATA_DIR) { $env:SIDENOTE_DATA_DIR }
            elseif ($env:SIDENOTE_STATE_DIR) { $env:SIDENOTE_STATE_DIR }
            elseif (Test-Path -LiteralPath (Join-Path $env:APPDATA 'sidenote')) { Join-Path $env:APPDATA 'sidenote' }
            else { Join-Path $env:APPDATA 'hex-meetings' }
    $local = Join-Path $data 'local'
    $claims = Join-Path $local 'deleted-meeting-claims'
    $path = Join-Path $claims $id
    $suppression = Join-Path $local 'deleted-meeting-ids.json'
    $result = $null
    try {
        foreach ($candidate in @($data, $local, $claims, $path, $suppression)) {
            $entry = Get-Item -LiteralPath $candidate -Force -ErrorAction SilentlyContinue
            if ($entry -and ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint)) { return $null }
        }
        [void][IO.Directory]::CreateDirectory($claims)
        # Rust writers share read/write. Recycling opens this path with FileShare.None.
        $result = [IO.File]::Open($path, 'OpenOrCreate', 'ReadWrite', 'ReadWrite')
        if (Test-Path -LiteralPath $suppression) {
            $ids = [IO.File]::ReadAllText($suppression) | ConvertFrom-Json -ErrorAction Stop
            if ($ids -isnot [PSCustomObject] -or $ids.PSObject.Properties[$id]) {
                $result.Dispose()
                return $null
            }
        }
        return $result
    } catch {
        if ($null -ne $result) { $result.Dispose() }
        return $null
    }
}

function Enter-NotesEngine([string]$MeetingDir) {
    # FileShare.None matches store_io::lock. The OS expires ownership on process death.
    try {
        return [IO.File]::Open((Join-Path $MeetingDir $script:JobContract.lock_file),
            [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    } catch [IO.IOException] {
        if (($_.Exception.HResult -band 0xffff) -in @(32,33)) { return $null }
        throw
    }
}

function Get-ScriptsNotesEngine {
    $child = $null
    try {
        $exe = if ($env:SIDENOTE_NOTES_QUERY_EXE) { $env:SIDENOTE_NOTES_QUERY_EXE }
               else { Join-Path $env:LOCALAPPDATA 'Sidenote\current\sidenote.exe' }
        $info = New-Object Diagnostics.ProcessStartInfo
        $info.FileName = $exe; $info.Arguments = '--notes-engine'
        $info.UseShellExecute = $false; $info.CreateNoWindow = $true
        $info.RedirectStandardOutput = $true; $info.RedirectStandardError = $true
        $child = [Diagnostics.Process]::Start($info)
        $output = $child.StandardOutput.ReadToEndAsync()
        $errors = $child.StandardError.ReadToEndAsync()
        if (-not $child.WaitForExit(15000)) { $child.Kill(); $child.WaitForExit(); return 'query_unavailable' }
        $value = $output.Result.Trim()
        if ($child.ExitCode -eq 0 -and -not $errors.Result -and
            $value -cin @('app','scripts','off','settings_invalid','settings_unavailable')) { return $value }
    } catch {} # No raw subprocess or configuration diagnostics escape the policy protocol.
    finally { if ($null -ne $child) { $child.Dispose() } }
    return 'query_unavailable'
}

function Format-NotesSkip([string]$Policy) {
    if ($Policy -cin @('app','off')) { return "notes skipped engine=$Policy" }
    if ($Policy -cnotin @('settings_invalid','settings_unavailable')) { $Policy = 'query_unavailable' }
    return "notes skipped category=$Policy"
}
