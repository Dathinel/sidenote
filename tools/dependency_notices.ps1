function Get-StandardLicenseIds([string]$Expression) {
    $supported = @('MIT', 'Apache-2.0', 'BSL-1.0', 'Zlib')
    $tokens = @([regex]::Matches($Expression, '[A-Za-z0-9.+-]+|[()]') | ForEach-Object Value)
    if (-not $tokens.Count -or (($tokens -join '') -ne ($Expression -replace '\s', ''))) {
        throw "Unsupported licence expression: $Expression"
    }
    $expectLicense = $true
    $depth = 0
    $ids = @()
    foreach ($token in $tokens) {
        if ($expectLicense) {
            if ($token -eq '(') { $depth++; continue }
            if ($token -cnotin $supported) { throw "Unsupported licence expression: $Expression" }
            $ids += $token
            $expectLicense = $false
        } elseif ($token -eq ')') {
            $depth--
            if ($depth -lt 0) { throw "Unbalanced licence expression: $Expression" }
        } elseif ($token -cin @('AND', 'OR')) {
            $expectLicense = $true
        } else {
            throw "Invalid licence expression: $Expression"
        }
    }
    if ($expectLicense -or $depth -ne 0) { throw "Incomplete licence expression: $Expression" }
    @($ids | Sort-Object -Unique)
}

function Write-DependencyNotices {
    param(
        [Parameter(Mandatory=$true)]$Metadata,
        [Parameter(Mandatory=$true)][string]$Destination,
        [string]$StandardLicenseRoot = (Join-Path $PSScriptRoot 'licenses'),
        [string]$LicenseChoicesPath = (Join-Path $PSScriptRoot 'license_choices.json')
    )
    $choices = Get-Content -LiteralPath $LicenseChoicesPath -Raw | ConvertFrom-Json
    $inventory = @('# Dependency notices', '', 'Notices for every dependency in the locked Cargo inventory.', '')
    $coverage = @()
    $usedStandards = @{}
    foreach ($package in $Metadata.packages | Sort-Object name, version) {
        if ($package.id -in $Metadata.workspace_members) { continue }
        if ($package.name -notmatch '^[A-Za-z0-9._+-]+$' -or $package.version -notmatch '^[A-Za-z0-9._+-]+$') {
            throw 'Invalid dependency notice folder name.'
        }
        $packageRoot = Split-Path -Parent $package.manifest_path
        $sourceRoot = [IO.Path]::GetFullPath($packageRoot)
        if ($package.source -like 'git+*') {
            $checkout = & git -C $packageRoot rev-parse --show-toplevel
            if ($LASTEXITCODE) { throw "Could not locate upstream Git notices for $($package.name)." }
            $sourceRoot = [IO.Path]::GetFullPath($checkout)
        }
        $noticeRoot = Join-Path $Destination "dependency-notices/$($package.name)-$($package.version)"
        $files = @(Get-ChildItem -LiteralPath $sourceRoot -Recurse -File | Where-Object {
            $_.FullName.Substring($sourceRoot.Length) -match '(?i)[\\/](licen[cs]es?|copying|copyright|notices?)([^\\/]*$|[\\/])'
        })
        if ($package.license_file) {
            $declared = if ([IO.Path]::IsPathRooted($package.license_file)) {
                [IO.Path]::GetFullPath($package.license_file)
            } else {
                [IO.Path]::GetFullPath((Join-Path $packageRoot $package.license_file))
            }
            if (-not $declared.StartsWith($sourceRoot.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) {
                throw "Licence file is outside the dependency source: $($package.name)"
            }
            $files += Get-Item -LiteralPath $declared -ErrorAction Stop
        }
        $files = @($files | Sort-Object FullName -Unique)
        $kind = 'upstream'
        $reason = ''
        $selected = ''
        if ($files.Count) {
            foreach ($file in $files) {
                $relative = $file.FullName.Substring($sourceRoot.Length).TrimStart('\', '/')
                $target = Join-Path $noticeRoot $relative
                New-Item -ItemType Directory -Force -Path (Split-Path -Parent $target) | Out-Null
                Copy-Item -LiteralPath $file.FullName -Destination $target -Force
            }
            $count = $files.Count
        } else {
            $count = 0
            $kind = 'uncovered'
            try {
                $choice = @($choices | Where-Object { $_.name -ceq $package.name -and $_.version -ceq $package.version })
                if ($choice.Count -gt 1) { throw 'Duplicate licence choices.' }
                if ($choice.Count) {
                    $choice = $choice[0]
                    if ($choice.declared -cne $package.license) { throw 'Declared expression changed; review the licence choice.' }
                    $alternatives = @($package.license -csplit '\s+OR\s+')
                    if ($alternatives.Count -lt 2 -or @($alternatives | Where-Object { $_ -cnotmatch '^[A-Za-z0-9.+-]+$' }).Count -or $choice.selected -cnotin $alternatives) {
                        throw 'Licence choice must select a declared standalone OR alternative.'
                    }
                    $selected = $choice.selected
                    $ids = @(Get-StandardLicenseIds $selected)
                } else {
                    $ids = @(Get-StandardLicenseIds $package.license)
                }
                $authors = @($package.authors | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Sort-Object -Unique)
                $holder = if ($authors.Count) { $authors -join '; ' } else { $package.repository }
                if ([string]::IsNullOrWhiteSpace($holder)) { throw 'Cargo metadata declares neither authors nor a repository.' }
                foreach ($id in $ids) {
                    $standard = Join-Path $StandardLicenseRoot "$id.txt"
                    if (-not (Test-Path -LiteralPath $standard -PathType Leaf) -or (Get-Item -LiteralPath $standard).Length -eq 0) {
                        throw "Standard licence text is missing: $id"
                    }
                }
                $notice = @(
                    "# $($package.name) $($package.version)", '',
                    "Name: $($package.name)", "Version: $($package.version)",
                    "Copyright holder (Cargo authors or repository): $holder",
                    "Repository: $($package.repository)",
                    "Declared licence expression: $($package.license)", '',
                    'The published crate contains no licence file. This notice is generated from its locked Cargo metadata.',
                    'Copyright attribution above comes from that metadata; no year is inferred.',
                    'Shared standard licence texts (relative to this notice):'
                )
                if ($selected) { $notice += @("Selected licence alternative: $selected", "Selection basis: $($choice.reason)") }
                foreach ($id in $ids) { $notice += "- ../standard-licenses/$id.txt"; $usedStandards[$id] = $true }
                New-Item -ItemType Directory -Force -Path $noticeRoot | Out-Null
                [IO.File]::WriteAllText((Join-Path $noticeRoot 'NOTICE.md'), ($notice -join "`n") + "`n")
                $kind = 'metadata'
                $count = 1
            } catch {
                $reason = $_.Exception.Message
            }
        }
        $coverage += [pscustomobject]@{ name=$package.name; version=$package.version; license=$package.license; selected_license=$selected; kind=$kind; notice_files=$count; reason=$reason }
        $inventory += "- $($package.name) $($package.version): $($package.license); $count notice file(s). [$kind]"
    }
    $shared = Join-Path $Destination 'dependency-notices/standard-licenses'
    if ($usedStandards.Count) {
        New-Item -ItemType Directory -Force -Path $shared | Out-Null
        foreach ($id in $usedStandards.Keys | Sort-Object) {
            Copy-Item -LiteralPath (Join-Path $StandardLicenseRoot "$id.txt") -Destination (Join-Path $shared "$id.txt") -Force
        }
    }
    [IO.File]::WriteAllText((Join-Path $Destination 'DEPENDENCY_NOTICES.md'), ($inventory -join "`n") + "`n")
    [IO.File]::WriteAllText((Join-Path $Destination 'DEPENDENCY_NOTICE_COVERAGE.json'), (ConvertTo-Json -InputObject @($coverage) -Depth 5))
    [pscustomobject]@{ coverage=@($coverage); uncovered=@($coverage | Where-Object notice_files -eq 0) }
}
