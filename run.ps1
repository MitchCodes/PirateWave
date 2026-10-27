$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot
if (Test-Path -LiteralPath 'dist/PirateWave/PirateWave.exe') { & './dist/PirateWave/PirateWave.exe' }
else {
    $taskCargo = Get-Command cargo -ErrorAction SilentlyContinue
    $taskCargoExe = if ($taskCargo) { $taskCargo.Source } else { Join-Path $env:USERPROFILE '.cargo/bin/cargo.exe' }
    if (-not (Test-Path -LiteralPath $taskCargoExe)) { throw 'Install Rust using https://rustup.rs, or build/download the portable app first.' }
    & './scripts/build-vst-bridge.ps1' -Profile debug -CargoExe $taskCargoExe
    & $taskCargoExe run --locked
    if ($LASTEXITCODE -ne 0) { throw 'PirateWave could not be built or started' }
}
