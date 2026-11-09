$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'dependency_notices.ps1')
$root = Join-Path ([IO.Path]::GetTempPath()) ('sidenote-notices-proof-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
function Assert([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}
function Package([string]$Name, [string]$License, [string[]]$Authors = @('Fixture author'), [string]$Repository = 'https://example.invalid/fixture') {
    $folder = Join-Path $root "sources/$Name"
    New-Item -ItemType Directory -Force -Path $folder | Out-Null
    [pscustomobject]@{ id=$Name; name=$Name; version='1.0.0'; authors=$Authors; repository=$Repository; license=$License; license_file=$null; source='registry+fixture'; manifest_path=(Join-Path $folder 'Cargo.toml') }
}
function Graph([object[]]$Packages) {
    [pscustomobject]@{packages=$Packages;workspace_members=@();resolve=[pscustomobject]@{nodes=@($Packages | ForEach-Object { [pscustomobject]@{id=$_.id} })}}
}
$packages = @(
    (Package 'mit-authors' 'MIT'),
    (Package 'apache-repository' 'Apache-2.0' @()),
    (Package 'alternatives' '(Zlib OR Apache-2.0) AND MIT'),
    (Package 'boost' 'BSL-1.0'),
    (Package 'upstream' 'MIT')
)
$upstream = Split-Path -Parent $packages[-1].manifest_path
[IO.File]::WriteAllText((Join-Path $upstream 'LICENSE'), 'Exact upstream fixture notice bytes.')
$metadata = Graph $packages
$metadata.packages += Package 'other-platform' 'LicenseRef-NotInGraph'
foreach ($run in @('first', 'second')) {
    $destination = Join-Path $root $run
    New-Item -ItemType Directory -Path $destination | Out-Null
    $result = Write-DependencyNotices -Metadata $metadata -Destination $destination
    Assert ($result.uncovered.Count -eq 0) 'Permissive dependencies must all have notices.'
    Assert ($result.coverage.Count -eq 5) 'Every dependency must appear in coverage.'
    Assert (-not (Test-Path "$destination/dependency-notices/other-platform-1.0.0")) 'Packages outside the selected platform graph must not be bundled.'
    $shared = @(Get-ChildItem "$destination/dependency-notices/standard-licenses" -File)
    Assert ($shared.Count -eq 4) 'Each standard text must be stored exactly once.'
    foreach ($name in @('mit-authors', 'apache-repository', 'alternatives', 'boost')) {
        $text = [IO.File]::ReadAllText("$destination/dependency-notices/$name-1.0.0/NOTICE.md")
        Assert ($text.Contains("Name: $name")) 'Notice must identify its own crate.'
        Assert ($text.Contains('Version: 1.0.0')) 'Notice must identify its version.'
        Assert ($text.Contains('Copyright holder (Cargo authors or repository):')) 'Notice needs metadata attribution.'
    }
    $repoNotice = [IO.File]::ReadAllText("$destination/dependency-notices/apache-repository-1.0.0/NOTICE.md")
    Assert ($repoNotice.Contains('Copyright holder (Cargo authors or repository): https://example.invalid/fixture')) 'Missing authors must fall back to the repository.'
    $choice = [IO.File]::ReadAllText("$destination/dependency-notices/alternatives-1.0.0/NOTICE.md")
    Assert ($choice.Contains('Declared licence expression: (Zlib OR Apache-2.0) AND MIT')) 'The complete expression must be retained.'
    Assert ([IO.File]::ReadAllText("$destination/dependency-notices/upstream-1.0.0/LICENSE") -ceq 'Exact upstream fixture notice bytes.') 'Original notices must remain byte-exact.'
}
$first = @(Get-ChildItem "$root/first" -Recurse -File)
$second = @(Get-ChildItem "$root/second" -Recurse -File)
Assert ($first.Count -eq $second.Count) 'Repeated generation must have the same file inventory.'
foreach ($file in $first) {
    $relative = $file.FullName.Substring((Join-Path $root 'first').Length)
    $other = (Join-Path $root 'second') + $relative
    Assert ((Get-FileHash -LiteralPath $file.FullName).Hash -ceq (Get-FileHash -LiteralPath $other).Hash) 'Generated bundles must be reproducible.'
}
$blocked = @(
    (Package 'copyleft' 'MIT OR LGPL-2.1-or-later'),
    (Package 'custom' 'LicenseRef-Unknown'),
    (Package 'broken-expression' 'MIT OR OR Apache-2.0'),
    (Package 'missing-holder' 'MIT' @() '')
)
$destination = Join-Path $root 'blocked'
New-Item -ItemType Directory -Path $destination | Out-Null
$result = Write-DependencyNotices -Metadata (Graph $blocked) -Destination $destination
Assert ($result.uncovered.Count -eq 4) 'Copyleft, custom, malformed and unattributed cases must remain uncovered.'
Assert (@(Get-ChildItem $destination -Recurse -Filter NOTICE.md).Count -eq 0) 'Uncovered dependencies must not receive fabricated notices.'
$approved = Package 'r-efi' 'MIT OR Apache-2.0 OR LGPL-2.1-or-later' @()
$approved.version = '5.3.0'
$destination = Join-Path $root 'approved-choice'
New-Item -ItemType Directory -Path $destination | Out-Null
$result = Write-DependencyNotices -Metadata (Graph @($approved)) -Destination $destination
Assert ($result.uncovered.Count -eq 0) 'The recorded MIT alternative must be usable.'
$notice = [IO.File]::ReadAllText("$destination/dependency-notices/r-efi-5.3.0/NOTICE.md")
Assert ($notice.Contains('Declared licence expression: MIT OR Apache-2.0 OR LGPL-2.1-or-later')) 'A choice must preserve the full declared expression.'
Assert ($notice.Contains('Selected licence alternative: MIT')) 'A choice must be recorded in its notice.'
$approved.license = 'Apache-2.0 OR LGPL-2.1-or-later'
$destination = Join-Path $root 'changed-choice'
New-Item -ItemType Directory -Path $destination | Out-Null
$result = Write-DependencyNotices -Metadata (Graph @($approved)) -Destination $destination
Assert ($result.uncovered.Count -eq 1) 'A changed declaration must invalidate the recorded choice.'
Write-Output "PASS: platform graph, metadata attribution, complete SPDX expressions, explicit OR choices, shared texts, upstream preservation, deterministic output and refusal cases. Evidence: $root"
