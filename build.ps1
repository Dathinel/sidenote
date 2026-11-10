param([switch]$Release, [string]$GnuBin = $env:RUST_GNU_BIN)
$ErrorActionPreference = 'Stop'
$env:PATH = "$env:USERPROFILE/.cargo/bin;$env:PATH"
if ($GnuBin -and (Test-Path -LiteralPath $GnuBin)) { $env:PATH = "$GnuBin;$env:PATH" }
if (-not (Get-Command as.exe -ErrorAction SilentlyContinue)) { throw 'A full MinGW GNU toolchain is needed. Set RUST_GNU_BIN to its bin folder.' }
$env:CARGO_BUILD_JOBS = '4'
$cargoArgs = @('build','--workspace','--locked','--bins')
if ($Release) { $cargoArgs += '--release' }
# Cargo finds .cargo/config.toml (linker, rustflags) from the working directory, not the manifest.
Push-Location -LiteralPath $PSScriptRoot
try { & cargo @cargoArgs } finally { Pop-Location }
exit $LASTEXITCODE
