param([string]$VpkPath = 'vpk')
$ErrorActionPreference = 'Stop'
$env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
$tokens = $null
$parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile(
    (Join-Path $PSScriptRoot 'pack_release.ps1'), [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw 'Release script does not parse.' }
$assignment = $ast.Find({ param($node)
    $node -is [System.Management.Automation.Language.AssignmentStatementAst] -and
    $node.Left.Extent.Text -eq '$packArgs' -and $node.Operator -eq 'Equals'
}, $true)
if (-not $assignment) { throw 'Release argument assignment is missing.' }
$array = $assignment.Right.Find({ param($node)
    $node -is [System.Management.Automation.Language.ArrayLiteralAst]
}, $true)
# Read the real argument list as data. Never execute the release script or pack.
$packArguments = @($array.Elements | ForEach-Object {
    if ($_ -is [System.Management.Automation.Language.StringConstantExpressionAst]) { $_.Value }
    elseif ($_ -is [System.Management.Automation.Language.VariableExpressionAst]) { 'fixture' }
    elseif ($_ -is [System.Management.Automation.Language.ExpandableStringExpressionAst]) { $_.Value }
    else { throw 'Unexpected executable release argument.' }
})
$root = Join-Path ([IO.Path]::GetTempPath()) ('sidenote-pack-proof-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
try {
    $fakeRunner = Join-Path $root 'fake-vpk.ps1'
    @'
param([Parameter(ValueFromRemainingArguments = $true)][string[]]$PackArguments)
$PackArguments | ConvertTo-Json -Compress
'@ | Set-Content -LiteralPath $fakeRunner
    $captured = (& $fakeRunner @packArguments) | ConvertFrom-Json
    $locations = @()
    for ($i = 0; $i -lt $captured.Count; $i++) {
        if ($captured[$i] -eq '--shortcuts') { $locations += $captured[$i + 1] }
    }
    if ($locations.Count -ne 1 -or $locations[0] -cne 'StartMenuRoot') {
        throw 'Release must request exactly one Start menu location and no Desktop.'
    }
    if ($captured[0] -ne 'pack' -or $captured -notcontains 'sidenote.exe') {
        throw 'Release arguments lost the package command or main executable.'
    }
    $help = (& $VpkPath --skip-updates pack --help) -join "`n"
    if ($LASTEXITCODE -or $help -notmatch '(?s)--shortcuts.*StartMenuRoot') {
        throw 'Installed vpk help does not confirm StartMenuRoot.'
    }
    Write-Output '4 packaging checks passed: syntax, Start menu only, package arguments, installed vpk help. No package created.'
} finally {
    $resolved = [IO.Path]::GetFullPath($root)
    if (-not $resolved.StartsWith([IO.Path]::GetFullPath([IO.Path]::GetTempPath()), [StringComparison]::OrdinalIgnoreCase) -or
        -not (Split-Path -Leaf $resolved).StartsWith('sidenote-pack-proof-')) {
        throw 'Refusing cleanup outside the packaging proof temp folder.'
    }
    Remove-Item -LiteralPath $root -Recurse -Force
}
