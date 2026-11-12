param(
    [string]$MeetingDir,
    [string]$Model = 'openai/gpt-6.1-terra-fast',
    [string]$NotesFile,
    [ValidateScript({ $_ -in @('auto', 'other', $env:HEX_MEETING_PROJECT) })][string]$Project = 'auto',
    [string]$NotesModel = 'claude-sonnet-5-5',
    [int]$LlmTimeoutSeconds = 240,
    # Rebuild only the summary from the existing english-transcript.md (no audio, no translation).
    [switch]$NotesOnly
)
$promptDir = $PSScriptRoot
. (Join-Path $PSScriptRoot 'hex-pipeline-cli.ps1')
$glossaryFile = Join-Path $env:USERPROFILE '.config\hex\meeting-glossary.txt'

$ErrorActionPreference = 'Stop'
$OutputEncoding = [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false
$total = [Diagnostics.Stopwatch]::StartNew()
$utf8 = New-Object System.Text.UTF8Encoding $false
if (-not $MeetingDir) {
    $root = Join-Path $env:APPDATA 'hex\meetings'
    $latest = Get-ChildItem -LiteralPath $root -Directory | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if (-not $latest) { throw "No meeting folders found in $root" }
    $MeetingDir = $latest.FullName
}
$dir = Get-Item -LiteralPath $MeetingDir
if (-not $dir.PSIsContainer) { throw "MeetingDir is not a folder: $MeetingDir" }
# Quick title (a cheap, fast model, a few seconds) before translation, so the app can name the meeting
# early. hex-meeting-watch.ps1 usually has done this already, and the script then skips. It never
# fails the notes: any problem is one log line.
try { & (Join-Path $PSScriptRoot 'hex-meeting-quick-title.ps1') -MeetingDir $dir.FullName } catch { Write-Host "quick-title failed; detail bytes=$([Text.Encoding]::UTF8.GetByteCount([string]$_))" }
$version = (Get-Content -LiteralPath (Join-Path $env:APPDATA 'hex\current-version') -Raw).Trim()
$hex = Join-Path $env:APPDATA "hex\versions\$version\hex.exe"
if (-not (Test-Path -LiteralPath $hex -PathType Leaf)) { throw "Hex executable not found: $hex" }
$ffmpeg = Get-Command ffmpeg -ErrorAction SilentlyContinue | Select-Object -First 1
$uv = Get-Command uv -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $uv) { throw 'Install uv to prepare speech segments.' }
$tracks = @(@{File='system.wav'; Speaker='Them'}, @{File='microphone.wav'; Speaker='You'}) | Where-Object { Test-Path -LiteralPath (Join-Path $dir.FullName $_.File) -PathType Leaf }
if (-not $tracks) { throw "No system.wav or microphone.wav found in $($dir.FullName)" }
$temp = Join-Path ([IO.Path]::GetTempPath()) ('hex-meeting-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $temp | Out-Null
$errFile = Join-Path $temp 'stderr.txt'
function Invoke-Native([string]$exe, [string[]]$arguments, [string]$outputFile) {
    Invoke-HexCli -CliArgs (@($exe) + $arguments) -OutputFile $outputFile -TimeoutSeconds 1800
}
function Run-LLM([string]$instruction, [string]$contents, [string]$tag) {
    $inputFile = Join-Path $temp "$tag-input.txt"
    $outputFile = Join-Path $temp "$tag-output.txt"
    [IO.File]::WriteAllText($inputFile, "# Instructions`n`n$instruction`n`n# Input`n`n$contents", $utf8)
    Invoke-HexCli -Kind translate -InputFile $inputFile -OutputFile $outputFile -Model $Model -TimeoutSeconds $LlmTimeoutSeconds
    return [IO.File]::ReadAllText($outputFile, $utf8).Trim()
}

try {
    $step = [Diagnostics.Stopwatch]::StartNew()
    if (-not $NotesOnly) {
    $audioHelper = Join-Path $promptDir 'meeting-audio.py'
    $extraModels = Join-Path $env:USERPROFILE '.config\hex\models-extra'
    if (-not $uv) { throw 'Install uv to run local diarization and the warm Hex API client.' }
    $uvArgs = @('run','--quiet','--with','sherpa-onnx==2.0.1','--with','soundfile','--with','scipy','python',$audioHelper)
    $prepareArgs = @('prepare','--meeting',$dir.FullName,'--temp',$temp,'--models',$extraModels)
    if ($ffmpeg) { $prepareArgs += @('--ffmpeg',$ffmpeg.Source) }
    Invoke-Native $uv.Source ($uvArgs + $prepareArgs) (Join-Path $temp 'prepare-log.txt')
    $audioInfo = [IO.File]::ReadAllText((Join-Path $temp 'audio-analysis.json'), $utf8) | ConvertFrom-Json
    $prepareSeconds = $step.Elapsed.TotalSeconds
    Write-Host ('Audio preparation: {0:N1}s; {1} speech segments; {2} remote speakers' -f $prepareSeconds,$audioInfo.segment_count,$audioInfo.remote_speakers)
    $step.Restart()
    Invoke-Native $uv.Source ($uvArgs + @('transcribe','--temp',$temp)) (Join-Path $temp 'asr-log.txt')
    $asrInfo = [IO.File]::ReadAllText((Join-Path $temp 'asr-timings.json'), $utf8) | ConvertFrom-Json
    foreach ($lang in 'te','hi') { Write-Host ('{0} transcription: {1:N1}s' -f $lang,$asrInfo.$lang.seconds) }
    $chunks = [IO.File]::ReadAllText((Join-Path $temp 'chunks.json'), $utf8) | ConvertFrom-Json
    if (@($chunks).Count -ne $audioInfo.segment_count) { throw 'Segment count changed while reading transcription results.' }
    # Durable evidence omits temporary WAV paths, which are removed after the run.
    $evidence = [ordered]@{
        preparationSeconds=$prepareSeconds
        analysis=$audioInfo
        transcription=$asrInfo
        segments=@($chunks | Select-Object Id,Start,End,AudioStart,AudioEnd,Speaker,Track,Telugu,Hindi)
    }
    $evidencePath = Join-Path $dir.FullName 'audio-evidence.json'
    if (Test-Path -LiteralPath $evidencePath) { Copy-Item -LiteralPath $evidencePath -Destination "$evidencePath.$([DateTime]::Now.ToString('yyyyMMddHHmmssfff')).bak" }
    [IO.File]::WriteAllText($evidencePath, ($evidence | ConvertTo-Json -Depth 15), $utf8)
    $step.Restart()
    # Hex's own English transcript, from before hex-meeting-publish.py replaced it. Each Hex line goes to
    # the segment on the same track that overlaps it most, so no line is read twice.
    $hexReading = @{}
    $hexFile = @((Join-Path $dir.FullName 'hex-original\transcript.ndjson'), (Join-Path $dir.FullName 'transcript.ndjson')) | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    if ($hexFile) {
        foreach ($line in [IO.File]::ReadAllLines($hexFile, $utf8)) {
            if (-not $line.Trim()) { continue }
            $h = $line | ConvertFrom-Json
            $track = if ($h.source -eq 'microphone') { 'microphone.wav' } else { 'system.wav' }
            $a = $h.start_ms / 1000; $b = [Math]::Max($h.end_ms / 1000, $a + 0.2)
            $best = $null; $bestOverlap = 0
            foreach ($chunk in $chunks) {
                if ($chunk.Track -ne $track) { continue }
                $overlap = [Math]::Min($b, $chunk.End + 0.5) - [Math]::Max($a, $chunk.Start - 0.5)
                if ($overlap -gt $bestOverlap) { $best = $chunk; $bestOverlap = $overlap }
            }
            if ($best) { $hexReading[$best.Id] = (($hexReading[$best.Id], $h.text) | Where-Object { $_ }) -join ' ' }
        }
    }
    $entries = foreach ($chunk in ($chunks | Sort-Object Start,Track,Id)) {
        $startStamp = [TimeSpan]::FromSeconds($chunk.Start).ToString('hh\:mm\:ss\.fff')
        $endStamp = [TimeSpan]::FromSeconds($chunk.End).ToString('hh\:mm\:ss\.fff')
        $english = if ($hexReading[$chunk.Id]) { $hexReading[$chunk.Id] } else { '(none)' }
        "[$startStamp-$endStamp] $($chunk.Speaker) | id=$($chunk.Id)`nTelugu-model reading: $($chunk.Telugu)`nHindi/English-model reading: $($chunk.Hindi)`nHex English-model reading: $english"
    }
    $batches = New-Object System.Collections.Generic.List[string]
    $batch = ''
    foreach ($entry in $entries) {
        if ($batch.Length -gt 0 -and ($batch.Length + $entry.Length) -gt 18000) { $batches.Add($batch); $batch = '' }
        $batch += $entry + "`n`n"
    }
    if ($batch) { $batches.Add($batch) }
    }
    # Glossary: Hex vocabulary terms plus optional one-term-per-line file.
    $terms = New-Object System.Collections.Generic.List[string]
    try {
        $vocab = (& $hex vocabulary list --json 2>$null | Out-String | ConvertFrom-Json)
        foreach ($v in @($vocab.entries) + @($vocab.terms)) { if ($v.term) { $terms.Add([string]$v.term) } }
    } catch {}
    if (Test-Path -LiteralPath $glossaryFile) {
        foreach ($line in [IO.File]::ReadAllLines($glossaryFile, $utf8)) { if ($line.Trim()) { $terms.Add($line.Trim()) } }
    }
    $glossary = if ($terms.Count) { ($terms | Select-Object -Unique | ForEach-Object { "- $_" }) -join "`n" } else { '(none)' }
    # Context: title/time from Hex's manifest when present, optional attendees.txt beside the audio.
    $title = ''; $started = $dir.CreationTime
    $manifestPath = Join-Path $dir.FullName 'manifest.json'
    if (Test-Path -LiteralPath $manifestPath) {
        try {
            $mf = [IO.File]::ReadAllText($manifestPath, $utf8) | ConvertFrom-Json
            if ($mf.title) { $title = [string]$mf.title }
            foreach ($k in 'started_at_ms','created_at_ms') { if ($mf.$k) { $started = [DateTimeOffset]::FromUnixTimeMilliseconds([int64]$mf.$k).LocalDateTime; break } }
        } catch {}
    }
    $attendeesPath = Join-Path $dir.FullName 'attendees.txt'
    $attendees = if (Test-Path -LiteralPath $attendeesPath) { ([IO.File]::ReadAllLines($attendeesPath, $utf8) | Where-Object { $_.Trim() }) -join ', ' } else { 'not provided' }
    $context = "Title: $(if ($title) { $title } else { 'not provided' })`nDate and time: $($started.ToString('dddd d MMMM yyyy, HH:mm')) (IST)`nAttendees: $attendees"
    # Rough notes typed during the meeting (Granola-style): -NotesFile, else notes.md beside the audio.
    if (-not $NotesFile) { $NotesFile = Join-Path $dir.FullName 'notes.md' }
    $myNotes = if (Test-Path -LiteralPath $NotesFile) { [IO.File]::ReadAllText($NotesFile, $utf8).Trim() } else { '' }
    if (-not $myNotes) { $myNotes = '(none)' }
    $rule = (Read-HexPrompt (Join-Path $promptDir 'meeting-translate-prompt.md')) + "`n## Glossary`n$glossary`n`n## Context`n$context`n`nThe meeting entries are in the Input section."
    if ($NotesOnly) {
        $transcript = ([IO.File]::ReadAllText((Join-Path $dir.FullName 'english-transcript.md'), $utf8) -replace '^\s*# Full English transcript\s*', '').Trim()
    } else {
    $transcripts = New-Object System.Collections.Generic.List[string]
    for ($i=0; $i -lt $batches.Count; $i++) {
        Write-Host "Translating batch $($i+1)/$($batches.Count)"
        $transcripts.Add((Run-LLM $rule $batches[$i] "batch-$i"))
    }
    $transcript = $transcripts -join "`n`n"
    }
    Write-Host 'Creating notes'
    # The quick title is only a hint, never required: the notes write the final title from the full meeting.
    $workingTitle = ''
    try {
        $qt = Join-Path $dir.FullName 'quick-title.json'
        if (Test-Path -LiteralPath $qt) { $workingTitle = ([string]([IO.File]::ReadAllText($qt, $utf8) | ConvertFrom-Json).title).Trim() }
    } catch {}
    $workingTitleSection = if ($workingTitle) { "## Working title`n$workingTitle`n`n" } else { '' }
    $workingTitleRule = if ($workingTitle) { "`nThe Input may include a Working title: a quick guess from the start and end of the transcript. Treat it only as a hint, and write the title from the whole meeting." } else { '' }
    $notesRule = (Read-HexPrompt (Join-Path $promptDir 'meeting-notes-prompt.md')) + "`n## Glossary`n$glossary`n`nThe Context, My notes and Transcript are in the Input section.$workingTitleRule"
    # Notes run in Claude Code from the meeting's own context; see hex-meeting-notes.ps1.
    $notesIn = Join-Path $temp 'notes-input.txt'
    $notesOut = Join-Path $temp 'notes-output.json'
    [IO.File]::WriteAllText($notesIn, "# Instructions`n`n$notesRule`n`n# Input`n`n## Context`n$context`n`n$workingTitleSection## My notes`n$myNotes`n`n## Transcript`n$transcript", $utf8)
    & (Join-Path $promptDir 'hex-meeting-notes.ps1') -InputFile $notesIn -OutFile $notesOut -Project $Project -Model $NotesModel
    $notes = [IO.File]::ReadAllText($notesOut, $utf8).Trim()
    Write-Host ('Translation and notes: {0:N1}s' -f $step.Elapsed.TotalSeconds)
    # Two files: the transcript is the record of what was said; the summary is written from it.
    # The meeting folder's notes.md is the user's own typed notes, so ours are english-summary.md and english-transcript.md (Hex itself writes transcript.md).
    # A real Hex meeting gets the English versions put into Hex itself (hex-meeting-publish.py), so the
    # Documents copies are only for folders Hex does not know about.
    $isHex = Test-Path -LiteralPath (Join-Path $dir.FullName 'manifest.json')
    $copyDir = Join-Path $env:USERPROFILE 'Documents\Hex Meetings'
    $stamp = $dir.CreationTime.ToString('yyyy-MM-dd HHmm')
    $suffix = ''
    if ((Test-Path -LiteralPath (Join-Path $copyDir "$stamp English Summary.md")) -or (Test-Path -LiteralPath (Join-Path $copyDir "$stamp English Transcript.md"))) {
        $suffix = ' ' + [guid]::NewGuid().ToString('N').Substring(0,8)
    }
    $transcriptName = if ($isHex) { 'english-transcript.md' } else { "$stamp English Transcript$suffix.md" }
    $outputs = @(
        @{ Skip = [bool]$NotesOnly; Local = 'english-transcript.md'; Copy = "$stamp English Transcript$suffix.md"; Text = "# Full English transcript`n`n$transcript`n" },
        @{ Skip = $false; Local = 'english-summary.json'; Copy = $null; Text = "$notes`n" }
    )
    if (-not $isHex) { New-Item -ItemType Directory -Force -Path $copyDir | Out-Null }
    foreach ($o in $outputs) {
        if ($o.Skip) { continue }
        $target = Join-Path $dir.FullName $o.Local
        if (Test-Path -LiteralPath $target) {
            $backup = "$target.$([DateTime]::Now.ToString('yyyyMMddHHmmssfff')).bak"
            Copy-Item -LiteralPath $target -Destination $backup
            Write-Host "Previous output backed up: $backup"
        }
        [IO.File]::WriteAllText($target, $o.Text, $utf8)
        Write-Host "Meeting output: $target"
        if (-not $isHex -and $o.Copy) {
            [IO.File]::WriteAllText((Join-Path $copyDir $o.Copy), $o.Text, $utf8)
            Write-Host "Documents copy: $(Join-Path $copyDir $o.Copy)"
        }
    }
    # The summary is JSON; hex-meeting-render.py writes english-summary.md and the meeting page from it
    # (the publish step runs it for Hex meetings).
    if (-not $isHex) {
        Invoke-HexCli -CliArgs @('uv','run','--quiet','--no-project','python','-I',(Join-Path $promptDir 'hex-meeting-render.py'),$dir.FullName) -OutputFile (Join-Path $temp 'render-status.txt') -TimeoutSeconds 120
        Copy-Item -LiteralPath (Join-Path $dir.FullName 'english-summary.md') -Destination (Join-Path $copyDir "$stamp English Summary$suffix.md")
    }
    if ($isHex) {
        Invoke-HexCli -CliArgs @('uv','run','--quiet','--no-project','python','-I',(Join-Path $promptDir 'hex-meeting-publish.py'),$dir.FullName) -OutputFile (Join-Path $temp 'publish-status.txt') -TimeoutSeconds 120
    }
    # Projects for the to-dos (tasks.json), read by the Meetings app's Tasks view. A failure here
    # only leaves the tasks unsorted, so it does not fail the meeting.
    try {
        Invoke-HexCli -CliArgs @('uv','run','--quiet','--no-project','python','-I',(Join-Path $promptDir 'hex-meeting-tasks.py'),'--force',$dir.FullName) -OutputFile (Join-Path $temp 'tasks-status.txt') -TimeoutSeconds 360
    } catch { Write-Host "Task classification failed; diagnostic bytes=$([Text.Encoding]::UTF8.GetByteCount([string]$_))" }
    # Compare with Granola's notes for the same call, 5 minutes later so late Granola edits are included
    # (owner, 2026-11-07). Wait in the foreground; it never fails the meeting.
    if ($isHex) {
        & (Join-Path $promptDir 'hex-meeting-compare-delayed.ps1') -MeetingDir $dir.FullName
    }
    Write-Host ('Total: {0:N1}s' -f $total.Elapsed.TotalSeconds)
} catch {
    throw "Meeting pipeline failed; diagnostic bytes=$([Text.Encoding]::UTF8.GetByteCount([string]$_))"
} finally {
    if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Recurse -Force }
}
