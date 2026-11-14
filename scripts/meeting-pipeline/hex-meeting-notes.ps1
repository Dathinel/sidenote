# Writes the notes for one meeting from the meeting's own context only
# (transcript, the recorder's notes, invite, glossary; configured project people and roles for configured project meetings).
# No mail, Granola or knowledge-base lookups here: that cross-check is hex-meeting-check.ps1, run on request.
param(
    [Parameter(Mandatory)][string]$InputFile,   # "# Instructions ... # Input ..." file built by hex-meeting-english.ps1
    [Parameter(Mandatory)][string]$OutFile,
    [ValidateScript({ $_ -in @('auto', 'other', $env:HEX_MEETING_PROJECT) })][string]$Project = 'auto',
    [string]$Model = 'claude-sonnet-5-5',
    [string]$RepoDir = $env:HEX_MEETING_KB_REPO,
    [ValidateRange(1,3600)][int]$TimeoutSeconds = 600
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'hex-pipeline-cli.ps1')
$notesEngine = Get-ScriptsNotesEngine
if ($notesEngine -cne 'scripts') { Write-Output "notes skipped engine=$notesEngine"; return }

$utf8 = New-Object System.Text.UTF8Encoding($false)
$text = [IO.File]::ReadAllText($InputFile, $utf8)

# Decide from Context and Transcript only: the Glossary always names configured project terms.
function Get-Section([string]$name) {
    $m = [regex]::Match($text, ('(?ms)^## ' + $name + '\s*$(.*?)(?=^## |\z)'))
    if ($m.Success) { $m.Groups[1].Value } else { '' }
}
if ($Project -eq 'auto') {
    $pattern = $env:HEX_MEETING_PROJECT_PATTERN
    $hits = if ($pattern) { [regex]::Matches((Get-Section 'Transcript'), $pattern).Count } else { 0 }
    $contextPattern = $env:HEX_MEETING_PROJECT_CONTEXT_PATTERN
    $contextMatch = $contextPattern -and (Get-Section 'Context') -match $contextPattern
    $Project = if ($contextMatch -or $hits -ge 2) { $env:HEX_MEETING_PROJECT } else { 'other' }
}

$split = $text.IndexOf("`n# Input")
if ($split -lt 0) { throw "No '# Input' section in $InputFile" }
$people = ''
if ($env:HEX_MEETING_PROJECT -and $Project -eq $env:HEX_MEETING_PROJECT) {
    $rosterWork = Join-Path ([IO.Path]::GetTempPath()) ('hex-roster-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $rosterWork | Out-Null
    try {
        $rosterFile = Join-Path $rosterWork 'people.md'
        Invoke-HexCli -CliArgs @('git','-C',$RepoDir,'fetch','-q','origin') -OutputFile $rosterFile -TimeoutSeconds 60 -Stage notes
        Invoke-HexCli -CliArgs @('git','-C',$RepoDir,'show','origin/master:docs/reference/people-and-roles.md') -OutputFile $rosterFile -TimeoutSeconds 30 -Stage notes
        $roster = [IO.File]::ReadAllText($rosterFile, $utf8)
    } finally { Remove-Item -LiteralPath $rosterWork -Recurse -Force }
    if ($roster) { $people = "## People and roles`n$roster`n`n" }
} else {
    $people = "## People and roles`n(Not a configured project meeting: do not use configured project people or roles.)`n`n"
}
$full = $text.Substring(0, $split) + "`n# Input`n`n" + $people + $text.Substring($split + "`n# Input".Length).TrimStart()

$work = Join-Path $env:TEMP ("hex-notes-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $work | Out-Null
$promptFile = Join-Path $work 'meeting.md'
[IO.File]::WriteAllText($promptFile, $full, $utf8)
try {
    $responseFile = Join-Path $work 'answer.json'
    Invoke-HexCli -Kind claude -InputFile $promptFile -OutputFile $responseFile -Model $Model -TimeoutSeconds $TimeoutSeconds -Stage notes
    $out = [IO.File]::ReadAllText($responseFile, $utf8)
} finally {
    Remove-Item -LiteralPath $work -Recurse -Force
}
$result = ($out -join "`n").Trim()
if (-not $result) { throw 'Empty notes from claude' }
# The notes are one JSON object; drop a code fence or stray text around it, then check its shape.
$start = $result.IndexOf('{'); $end = $result.LastIndexOf('}')
if ($start -lt 0 -or $end -le $start) { throw "Notes are not a JSON object; response bytes=$([Text.Encoding]::UTF8.GetByteCount($result))" }
$result = $result.Substring($start, $end - $start + 1)
try { $parsed = $result | ConvertFrom-Json } catch { throw "Invalid notes JSON; response bytes=$([Text.Encoding]::UTF8.GetByteCount($result))" }
foreach ($field in 'title', 'context', 'topics', 'todo', 'to_settle') {
    if ($null -eq $parsed.$field) { throw "Notes JSON has no '$field' field" }
}
[IO.File]::WriteAllText($OutFile, $result + "`n", $utf8)
Write-Output "notes: $Project, $Model -> $OutFile"
