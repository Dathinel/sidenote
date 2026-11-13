# Exercise the real wrapper and gate calls against a native argv recorder, without compiling Rust.
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
$temp = Join-Path ([IO.Path]::GetTempPath()) ('local-build-test-' + [guid]::NewGuid().ToString('N'))
$saved = @{}
foreach ($name in 'PATH','LOCALAPPDATA','CARGO_TARGET_DIR','CARGO_BUILD_TARGET_DIR') {
    $saved[$name] = [Environment]::GetEnvironmentVariable($name)
}
try {
    New-Item -ItemType Directory -Path $temp | Out-Null
    Add-Type -OutputAssembly (Join-Path $temp 'cargo.exe') -OutputType ConsoleApplication -TypeDefinition @'
using System;
class CargoArgs {
    static void Main(string[] args) {
        foreach (string arg in args) Console.WriteLine("arg:" + Convert.ToBase64String(System.Text.Encoding.UTF8.GetBytes(arg)));
        foreach (string name in new [] {"CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR", "CARGO_BUILD_BUILD_DIR"})
            Console.WriteLine("target:" + Environment.GetEnvironmentVariable(name));
    }
}
'@
    $env:PATH = "$temp;" + $env:PATH
    $env:LOCALAPPDATA = $temp
    $env:CARGO_TARGET_DIR = Join-Path $temp 'fallback'
    $env:CARGO_BUILD_TARGET_DIR = Join-Path $temp 'other-fallback'
    $gate = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'release_gate.ps1'))
    $calls = [regex]::Matches($gate, "(?m)^Gate 'clippy(?:-asr)?' \{ (& tools/local_build.ps1 .+) \}\r?$")
    if ($calls.Count -ne 2) { throw 'Expected both Clippy gate calls.' }
    $commands = @($calls | ForEach-Object { $_.Groups[1].Value })
    $commands += "& tools/local_build.ps1 clippy --target-dir 'target with spaces\' '--' -D warnings --config tool-only"
    $commands += "& tools/local_build.ps1 clippy '--target-dir=target equals' '--' -D warnings"
    foreach ($command in $commands) {
        $output = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -Command $command)
        if ($LASTEXITCODE) { throw "Wrapper exited $LASTEXITCODE" }
        $actual = @($output | Where-Object { $_.StartsWith('arg:') } | ForEach-Object {
            [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($_.Substring(4)))
        })
        $tokens = [Management.Automation.PSParser]::Tokenize($command, [ref]$null)
        $expected = @($tokens | Select-Object -Skip 2 | ForEach-Object { $_.Content })
        if (($actual | ConvertTo-Json -Compress) -cne ($expected | ConvertTo-Json -Compress)) {
            throw "Native argument mismatch: $command"
        }
        $target = if ($command.Contains('--target-dir=')) { Join-Path $root 'target equals' }
                  elseif ($command.Contains('--target-dir ')) { Join-Path $root 'target with spaces\' }
                  else { $env:CARGO_TARGET_DIR }
        $targets = @($output | Where-Object { $_.StartsWith('target:') })
        if ($targets.Count -ne 3 -or @($targets | Where-Object { $_ -cne "target:$target" }).Count) {
            throw 'Checked target directory did not reach the child.'
        }
    }
    $ErrorActionPreference = 'Continue'
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -Command '& tools/local_build.ps1 check --config build.target-dir=unchecked' 2>$null
    $ErrorActionPreference = 'Stop'
    if ($LASTEXITCODE -ne 1) { throw 'Unchecked Cargo config was accepted.' }
    Write-Output 'PASS: both gate calls, separator suffix, target overrides, and config rejection.'
} finally {
    foreach ($name in $saved.Keys) { [Environment]::SetEnvironmentVariable($name, $saved[$name]) }
    if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Recurse -Force }
}
