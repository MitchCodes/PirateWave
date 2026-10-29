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

$taskPreviousNoEditor = $env:PIRATEWAVE_TEST_VST2_NO_EDITOR
$taskPreviousProgramOnly = $env:PIRATEWAVE_TEST_VST2_PROGRAM_ONLY
try {
    $env:PIRATEWAVE_TEST_VST2_NO_EDITOR = '1'
    & $taskExe --plugin-no-editor-self-test $taskFixture
    if ($LASTEXITCODE -ne 0) { throw 'Native VST2 editor-free state check failed' }
    & $taskExe --plugin-no-editor-self-test $taskFixture32
    if ($LASTEXITCODE -ne 0) { throw 'Bridged VST2 editor-free state check failed' }
    $env:PIRATEWAVE_TEST_VST2_PROGRAM_ONLY = '1'
    & $taskExe --plugin-no-editor-self-test $taskFixture
    if ($LASTEXITCODE -ne 0) { throw 'Native VST2 program state check failed' }
    & $taskExe --plugin-no-editor-self-test $taskFixture32
    if ($LASTEXITCODE -ne 0) { throw 'Bridged VST2 program state check failed' }
} finally {
    $env:PIRATEWAVE_TEST_VST2_NO_EDITOR = $taskPreviousNoEditor
    $env:PIRATEWAVE_TEST_VST2_PROGRAM_ONLY = $taskPreviousProgramOnly
}
