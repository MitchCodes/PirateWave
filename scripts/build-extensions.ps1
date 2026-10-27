param([ValidateSet('debug','release')][string]$Profile='release',[Parameter(Mandatory=$true)][string]$PirateWaveExe)
$ErrorActionPreference='Stop'
Set-Location (Split-Path $PSScriptRoot)
$taskPirateWave=(Resolve-Path -LiteralPath $PirateWaveExe).Path
$taskArgs=@('build','--locked','-p','piratewave-example-extension')
if($Profile -eq 'release'){$taskArgs+='--release'}
& cargo @taskArgs
if($LASTEXITCODE -ne 0){throw 'Example extension build failed'}
$taskMetadata=(& cargo metadata --locked --no-deps --format-version 1)|ConvertFrom-Json
$taskExample=Join-Path $taskMetadata.target_directory "$Profile/piratewave_example_extension.dll"
New-Item -ItemType Directory -Force 'dist/example-extension'|Out-Null
Copy-Item -LiteralPath $taskExample -Destination 'dist/example-extension/piratewave_example_extension.dll' -Force
Copy-Item -LiteralPath 'crates/piratewave-example-extension/manifest.json' -Destination 'dist/example-extension/manifest.json' -Force
Copy-Item -LiteralPath 'LICENSE' -Destination 'dist/example-extension/LICENSE' -Force
& python scripts/package-extension.py dist/example-extension dist/PirateWaveExamples.piratewaveext --piratewave $taskPirateWave
if($LASTEXITCODE -ne 0){throw 'Example extension package verification failed'}
$taskFixture=(Resolve-Path 'dist/example-extension').Path
$taskValidation=Start-Process -FilePath $taskPirateWave -ArgumentList @('--extension-self-test',('"'+$taskFixture+'"')) -WindowStyle Hidden -Wait -PassThru -RedirectStandardOutput 'dist/extensions-native.txt' -RedirectStandardError 'dist/extensions-native-errors.txt'
if($taskValidation.ExitCode -ne 0){Get-Content 'dist/extensions-native-errors.txt';throw 'Native extension integration failed'}
Get-Content 'dist/extensions-native.txt'
