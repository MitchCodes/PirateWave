param([ValidateSet('debug','release')][string]$Profile='debug',[string]$CargoExe='')
$ErrorActionPreference='Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
if (-not $CargoExe) {
    $taskCargo=Get-Command cargo -ErrorAction SilentlyContinue
    $CargoExe=if ($taskCargo) {$taskCargo.Source}else{Join-Path $env:USERPROFILE '.cargo/bin/cargo.exe'}
}
$taskRustup=Join-Path (Split-Path $CargoExe -Parent) 'rustup.exe'
$taskTargets=& $taskRustup target list --installed
if ($LASTEXITCODE -ne 0) {throw 'Could not inspect installed Rust targets'}
if ($taskTargets -notcontains 'i686-pc-windows-msvc') {
    & $taskRustup target add i686-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) {throw 'Install the i686-pc-windows-msvc Rust target to build the VST2 helper'}
}
$taskArgs=@('build','--locked','-p','piratewave-vst-bridge','--target','i686-pc-windows-msvc')
if ($Profile -eq 'release') {$taskArgs+='--release'}
& $CargoExe @taskArgs
if ($LASTEXITCODE -ne 0) {throw '32-bit VST helper build failed (requires MSVC x86 build tools)'}
$taskMetadata=(& $CargoExe metadata --locked --no-deps --format-version 1) | ConvertFrom-Json
$taskSource=Join-Path $taskMetadata.target_directory "i686-pc-windows-msvc/$Profile/piratewave-vst-bridge.exe"
$taskDestination=Join-Path $taskMetadata.target_directory "$Profile/piratewave-vst-bridge32.exe"
New-Item -ItemType Directory -Force (Split-Path $taskDestination -Parent) | Out-Null
Copy-Item -LiteralPath $taskSource -Destination $taskDestination -Force
Write-Host "Built $taskDestination"
