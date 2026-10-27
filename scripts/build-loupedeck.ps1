param([string]$DotnetExe = '', [string]$PluginApiDir = '', [switch]$Verify, [string]$PirateWaveExe = '')
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
if (-not $DotnetExe) {
    $taskDotnet = Get-Command dotnet -ErrorAction SilentlyContinue
    $DotnetExe = if ($taskDotnet) { $taskDotnet.Source } else { Join-Path $env:ProgramFiles 'dotnet/dotnet.exe' }
}
if (-not $PluginApiDir) { $PluginApiDir = if ($env:PIRATEWAVE_LOUPEDECK_SDK_DIR) { $env:PIRATEWAVE_LOUPEDECK_SDK_DIR } else { 'C:/Program Files/Logi/LogiPluginService' } }
if (-not (Test-Path -LiteralPath (Join-Path $PluginApiDir 'PluginApi.dll'))) { throw 'Install Logi Plugin Service 6.4.1+ or supply -PluginApiDir. SDK assemblies are not bundled.' }
& $DotnetExe build integrations/loupedeck/PirateWavePlugin -c Release --nologo "-p:PluginApiDir=$PluginApiDir" -warnaserror
if ($LASTEXITCODE -ne 0) { throw 'Loupedeck plugin build failed' }
# Static package verification does not exercise Logi's class-discovery contract.
# Run this even without -Verify, before producing a distributable package.
& $DotnetExe run --project integrations/loupedeck/PirateWavePlugin.Tests -c Release "-p:PluginApiDir=$PluginApiDir" -- --check-installation
if ($LASTEXITCODE -ne 0) { throw 'Loupedeck loader/installation/editor checks failed' }
$taskStage = "dist/loupedeck-package-$PID"
New-Item -ItemType Directory -Force "$taskStage/bin", "$taskStage/metadata" | Out-Null
# Explicit allowlist: never package SDK/host dependencies, development links or profiles.
Copy-Item -LiteralPath 'integrations/loupedeck/PirateWavePlugin/bin/Release/net10.0-windows/PirateWavePlugin.dll' -Destination "$taskStage/bin/PirateWavePlugin.dll" -Force
Copy-Item -LiteralPath 'integrations/loupedeck/PirateWavePlugin/package/metadata/LoupedeckPackage.yaml' -Destination "$taskStage/metadata/LoupedeckPackage.yaml" -Force
Copy-Item -LiteralPath 'assets/branding/piratewave-256.png' -Destination "$taskStage/metadata/Icon256x256.png" -Force
$taskTool = Join-Path (Get-Location) '.tools/loupedeck/logiplugintool.exe'
if (-not (Test-Path -LiteralPath $taskTool)) {
    & $DotnetExe tool install --tool-path .tools/loupedeck LogiPluginTool --version 6.4.2.3599
    if ($LASTEXITCODE -ne 0) { throw 'Could not install official Logi Plugin Tool' }
}
& $taskTool pack $taskStage dist/PirateWave.lplug4
if ($LASTEXITCODE -ne 0) { throw 'Loupedeck packaging failed' }
& $taskTool verify dist/PirateWave.lplug4
if ($LASTEXITCODE -ne 0) { throw 'Loupedeck package verification failed' }
if ($Verify) {
    if (-not $PirateWaveExe) { $PirateWaveExe = (Resolve-Path 'target/debug/piratewave-desktop.exe').Path }
    & $DotnetExe run --project integrations/loupedeck/PirateWavePlugin.Tests -c Release "-p:PluginApiDir=$PluginApiDir" -- $PirateWaveExe
    if ($LASTEXITCODE -ne 0) { throw 'Loupedeck SDK/native transport integration tests failed' }
}
Write-Host 'Built verified dist/PirateWave.lplug4. Double-click it to install; existing profiles are not changed by this build.'
