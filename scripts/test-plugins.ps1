param([string]$CargoExe = '')
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
if (-not $CargoExe) {
    $taskCargo = Get-Command cargo -ErrorAction SilentlyContinue
    if ($taskCargo) { $CargoExe = $taskCargo.Source }
    else { $CargoExe = Join-Path $env:USERPROFILE '.cargo/bin/cargo.exe' }
}
& $CargoExe build --locked -p piratewave-desktop -p piratewave-test-plugin
if ($LASTEXITCODE -ne 0) { throw 'Plugin fixture build failed' }
$taskMetadata = (& $CargoExe metadata --locked --no-deps --format-version 1) | ConvertFrom-Json
$taskFixture = Join-Path $taskMetadata.target_directory 'debug/piratewave_test_plugin.dll'
$taskExe = Join-Path $taskMetadata.target_directory 'debug/piratewave-desktop.exe'
$taskBundle = Join-Path (Get-Location) 'dist/plugin-fixtures/PirateWave Test.vst3'
$taskModuleDirectory = Join-Path $taskBundle 'Contents/x86_64-win'
New-Item -ItemType Directory -Force $taskModuleDirectory | Out-Null
Copy-Item -LiteralPath $taskFixture -Destination (Join-Path $taskModuleDirectory 'PirateWave Test.vst3') -Force
& $taskExe --plugin-self-test $taskBundle $taskFixture
if ($LASTEXITCODE -ne 0) { throw 'Native VST integration checks failed' }

& './scripts/build-vst-bridge.ps1' -Profile debug -CargoExe $CargoExe
& $CargoExe build --locked -p piratewave-test-plugin --target i686-pc-windows-msvc
if ($LASTEXITCODE -ne 0) { throw '32-bit VST test fixture build failed' }
$taskFixture32=Join-Path $taskMetadata.target_directory 'i686-pc-windows-msvc/debug/piratewave_test_plugin.dll'
& $taskExe --plugin-self-test $taskBundle $taskFixture32
if ($LASTEXITCODE -ne 0) { throw '32-bit VST2 bridge regression checks failed' }
