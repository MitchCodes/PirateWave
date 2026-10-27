param([switch]$DebugBuild, [switch]$Verify, [switch]$WithLoupedeck, [switch]$WithStreamDeck)
$ErrorActionPreference = 'Stop'
function Copy-ExtensionSdk([string]$Destination) {
    New-Item -ItemType Directory -Force "$Destination/crates/piratewave-extension-sdk/src","$Destination/crates/piratewave-example-extension/src","$Destination/scripts","$Destination/examples/gain-plugin/src" | Out-Null
    $taskSdkToml=@('[workspace]','members = ["crates/piratewave-extension-sdk", "crates/piratewave-example-extension", "examples/gain-plugin"]','resolver = "2"','[profile.release]','lto = "thin"','strip = true') -join "`n"
    [IO.File]::WriteAllText((Join-Path (Resolve-Path $Destination).Path 'Cargo.toml'), $taskSdkToml, [Text.UTF8Encoding]::new($false))
    foreach($taskSdkCrate in @('piratewave-extension-sdk','piratewave-example-extension')) {
        foreach($taskSdkFile in @('Cargo.toml','src/lib.rs')) { Copy-Item -LiteralPath "crates/$taskSdkCrate/$taskSdkFile" -Destination "$Destination/crates/$taskSdkCrate/$taskSdkFile" -Force }
    }
    foreach($taskExampleFile in @('Cargo.toml','manifest.json','README.md','src/lib.rs')) { Copy-Item -LiteralPath "examples/gain-plugin/$taskExampleFile" -Destination "$Destination/examples/gain-plugin/$taskExampleFile" -Force }
    Copy-Item -LiteralPath 'examples/README.md' -Destination "$Destination/examples/README.md" -Force
    Copy-Item -LiteralPath 'crates/piratewave-example-extension/manifest.json' -Destination "$Destination/crates/piratewave-example-extension/manifest.json" -Force
    Copy-Item -LiteralPath 'scripts/build-extensions.ps1','scripts/package-extension.py' -Destination "$Destination/scripts" -Force
    $taskSdkReadme = [IO.File]::ReadAllText((Resolve-Path 'docs/plugins/SDK_README.md').Path).Replace('](../PLUGIN_DEVELOPMENT.md)', '](../docs/PLUGIN_DEVELOPMENT.md)')
    [IO.File]::WriteAllText((Join-Path (Resolve-Path $Destination).Path 'README.md'), $taskSdkReadme, [Text.UTF8Encoding]::new($false))
    Copy-Item -LiteralPath 'LICENSE' -Destination "$Destination/LICENSE" -Force
    & $taskCargoExe generate-lockfile --offline --manifest-path "$Destination/Cargo.toml"
    if($LASTEXITCODE -ne 0){throw 'Standalone SDK lockfile generation failed'}
}

Set-Location $PSScriptRoot
$taskCargo = Get-Command cargo -ErrorAction SilentlyContinue
if ($taskCargo) { $taskCargoExe = $taskCargo.Source }
else { $taskCargoExe = Join-Path $env:USERPROFILE '.cargo/bin/cargo.exe' }
if (-not (Test-Path -LiteralPath $taskCargoExe)) { throw 'Install Rust using https://rustup.rs, then run this script again.' }
if ($Verify) {
    & $taskCargoExe fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { throw 'Formatting check failed' }
    & $taskCargoExe test --workspace --all-targets --locked
    if ($LASTEXITCODE -ne 0) { throw 'Tests failed' }
    & $taskCargoExe clippy --workspace --all-targets --locked -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'Clippy failed' }
}
if ($DebugBuild) { & $taskCargoExe build --locked; $taskProfile = 'debug' }
else { & $taskCargoExe build --release --locked; $taskProfile = 'release' }
if ($LASTEXITCODE -ne 0) { throw 'Build failed' }
& './scripts/build-vst-bridge.ps1' -Profile $taskProfile -CargoExe $taskCargoExe
if ($WithLoupedeck) { & './scripts/build-loupedeck.ps1' }
if ($WithStreamDeck) { & './scripts/build-streamdeck.ps1' }
$taskPackageText = & $taskCargoExe metadata --no-deps --format-version 1 --locked
if ($LASTEXITCODE -ne 0) { throw 'Could not read package version' }
$taskPackageMetadata = $taskPackageText | ConvertFrom-Json
$taskPackageVersion = ($taskPackageMetadata.packages | Where-Object { $_.name -eq 'piratewave-desktop' }).version
if (-not $taskPackageVersion) { throw 'Desktop package version was not found' }
New-Item -ItemType Directory -Force 'dist/PirateWave','dist/PirateWave/assets' | Out-Null
Copy-Item -LiteralPath 'assets/branding' -Destination 'dist/PirateWave/assets' -Recurse -Force
$taskBinary = Join-Path (Join-Path $taskPackageMetadata.target_directory $taskProfile) 'piratewave-desktop.exe'
& './scripts/build-extensions.ps1' -Profile $taskProfile -PirateWaveExe $taskBinary
Copy-Item -LiteralPath $taskBinary -Destination 'dist/PirateWave/PirateWave.exe' -Force
Copy-Item -LiteralPath 'dist/voice-training/native-worker-candidate/PatchVoiceWorker.exe' -Destination 'dist/PirateWave/PirateWaveVoiceWorker.exe' -Force
Copy-Item -LiteralPath 'dist/voice-pack/llvc-f8312-v1/msvcp140_1.dll' -Destination 'dist/PirateWave/msvcp140_1.dll' -Force
Copy-Item -LiteralPath 'dist/voice-pack/llvc-f8312-v1/WORKER_THIRD_PARTY_NOTICES.md' -Destination 'dist/PirateWave/VOICE_WORKER_THIRD_PARTY_NOTICES.md' -Force
$taskBridgeBinary=Join-Path (Join-Path $taskPackageMetadata.target_directory $taskProfile) 'piratewave-vst-bridge32.exe'
Copy-Item -LiteralPath $taskBridgeBinary -Destination 'dist/PirateWave/piratewave-vst-bridge32.exe' -Force
Copy-Item -LiteralPath 'README.md' -Destination 'dist/PirateWave/README.md' -Force
Copy-Item -LiteralPath 'LICENSE' -Destination 'dist/PirateWave/LICENSE' -Force
Copy-Item -LiteralPath 'CONTRIBUTING.md' -Destination 'dist/PirateWave/CONTRIBUTING.md' -Force
Copy-Item -LiteralPath 'scripts/voice-qualification.cmd' -Destination 'dist/PirateWave/Voice Qualification.cmd' -Force
New-Item -ItemType Directory -Force 'dist/PirateWave/docs/images','dist/PirateWave/crates/piratewave-core/examples','dist/PirateWave/design/prototype' | Out-Null
New-Item -ItemType Directory -Force 'dist/PirateWave/docs/audio' | Out-Null
Get-ChildItem -LiteralPath 'docs/audio' -File | Copy-Item -Destination 'dist/PirateWave/docs/audio' -Force
Copy-Item -LiteralPath 'docs/ARCHITECTURE.md' -Destination 'dist/PirateWave/docs/ARCHITECTURE.md' -Force
Copy-Item -LiteralPath 'docs/RACK_PROCESSORS.md' -Destination 'dist/PirateWave/docs/RACK_PROCESSORS.md' -Force
Copy-Item -LiteralPath 'docs/LOUPEDECK.md' -Destination 'dist/PirateWave/docs/LOUPEDECK.md' -Force
if ($WithLoupedeck) { Copy-Item -LiteralPath 'dist/PirateWave.lplug4' -Destination 'dist/PirateWave/PirateWave.lplug4' -Force }
Copy-Item -LiteralPath 'docs/STREAM_DECK.md' -Destination 'dist/PirateWave/docs/STREAM_DECK.md' -Force
Copy-Item -LiteralPath 'docs/EFFECT_TUNING.md' -Destination 'dist/PirateWave/docs/EFFECT_TUNING.md' -Force
Copy-Item -LiteralPath 'docs/TOOLS.md' -Destination 'dist/PirateWave/docs/TOOLS.md' -Force
if ($WithStreamDeck) { Copy-Item -LiteralPath 'dist/PirateWave.streamDeckPlugin' -Destination 'dist/PirateWave/PirateWave.streamDeckPlugin' -Force }
Copy-ExtensionSdk 'dist/PirateWave/sdk'
Copy-Item -LiteralPath 'dist/PirateWaveExamples.piratewaveext' -Destination 'dist/PirateWave/PirateWaveExamples.piratewaveext' -Force
New-Item -ItemType Directory -Force 'dist/PirateWave/docs/plugins' | Out-Null
Copy-Item -LiteralPath 'docs/PLUGIN_DEVELOPMENT.md' -Destination 'dist/PirateWave/docs/PLUGIN_DEVELOPMENT.md' -Force
Get-ChildItem -LiteralPath 'docs/plugins' -File | Copy-Item -Destination 'dist/PirateWave/docs/plugins' -Force
Copy-Item -LiteralPath 'docs/COOKBOOK.md','docs/AUDIO_RECOVERY.md','docs/DEVICE_DISCOVERY_MEMORY.md' -Destination 'dist/PirateWave/docs' -Force
Copy-Item -LiteralPath 'docs/WORKSPACES.md' -Destination 'dist/PirateWave/docs/WORKSPACES.md' -Force
Copy-Item -LiteralPath 'docs/WORKSPACE_LAYOUT.md' -Destination 'dist/PirateWave/docs/WORKSPACE_LAYOUT.md' -Force
Copy-Item -LiteralPath 'docs/AUTO_DUCKING.md' -Destination 'dist/PirateWave/docs/AUTO_DUCKING.md' -Force
Copy-Item -LiteralPath 'docs/RECORDING.md' -Destination 'dist/PirateWave/docs/RECORDING.md' -Force
Copy-Item -LiteralPath 'docs/APPLICATION_CAPTURE.md' -Destination 'dist/PirateWave/docs/APPLICATION_CAPTURE.md' -Force
Copy-Item -LiteralPath 'docs/AUTOMATIC_VOICE_CORRECTION.md' -Destination 'dist/PirateWave/docs/AUTOMATIC_VOICE_CORRECTION.md' -Force
Copy-Item -LiteralPath 'docs/VST_PLUGINS.md' -Destination 'dist/PirateWave/docs/VST_PLUGINS.md' -Force
Copy-Item -LiteralPath 'docs/VOICE_TRAINING_NOTICES.txt' -Destination 'dist/PirateWave/docs/VOICE_TRAINING_NOTICES.txt' -Force
foreach($taskVoiceDoc in @('VOICE_CHANGER.md','VOICE_RUNTIME_DECISION.md','VOICE_ACCEPTANCE.example.json','VOICE_CHANGER_VALIDATION.json','VOICE_TRAINING.md','VOICE_TRAINING_FEASIBILITY.json','VOICE_TRAINING_PROGRESS.json','VOICE_TRAINING_BETA_VALIDATION.json')) {
    $taskVoicePath=Join-Path 'docs' $taskVoiceDoc
    if(Test-Path -LiteralPath $taskVoicePath){Copy-Item -LiteralPath $taskVoicePath -Destination (Join-Path 'dist/PirateWave/docs' $taskVoiceDoc) -Force}
}
Copy-Item -LiteralPath 'docs/images/mixer.png' -Destination 'dist/PirateWave/docs/images/mixer.png' -Force
Copy-Item -LiteralPath 'crates/piratewave-core/examples/custom_effect.rs' -Destination 'dist/PirateWave/crates/piratewave-core/examples/custom_effect.rs' -Force
Get-ChildItem -LiteralPath 'design/prototype' | Copy-Item -Destination 'dist/PirateWave/design/prototype' -Recurse -Force
# The developer portable folder uses the repository README and documentation index.
Get-ChildItem -LiteralPath 'docs' -Filter '*.md' -File | Copy-Item -Destination 'dist/PirateWave/docs' -Force
& './scripts/third-party-notices.ps1' -CargoExe $taskCargoExe -OutputPath 'dist/PirateWave/THIRD_PARTY_NOTICES.md'
$taskArchive = "dist/PirateWave-v$taskPackageVersion-windows-x64.zip"
# Publish a fresh allowlisted staging tree. Developer examples/prototype sources
# and any unrelated files in an older portable folder never enter public builds.
$taskPublicStage = "dist/PirateWave-public-$taskPackageVersion-$PID"
if (Test-Path -LiteralPath $taskPublicStage) { throw 'Public staging directory already exists' }
New-Item -ItemType Directory -Path $taskPublicStage | Out-Null
foreach ($taskPublicFile in @('PirateWave.exe','PirateWaveVoiceWorker.exe','msvcp140_1.dll','piratewave-vst-bridge32.exe','PirateWaveExamples.piratewaveext','LICENSE','THIRD_PARTY_NOTICES.md','VOICE_WORKER_THIRD_PARTY_NOTICES.md','Voice Qualification.cmd')) {
    Copy-Item -LiteralPath (Join-Path 'dist/PirateWave' $taskPublicFile) -Destination (Join-Path $taskPublicStage $taskPublicFile)
}

$taskPortableReadme = [IO.File]::ReadAllText((Resolve-Path 'docs/PORTABLE_README.md').Path)
foreach ($taskReadmeGuide in @('PLUGIN_DEVELOPMENT','STREAM_DECK','TOOLS','COOKBOOK')) {
    $taskPortableReadme = $taskPortableReadme.Replace("]($taskReadmeGuide.md)", "](docs/$taskReadmeGuide.md)")
}
[IO.File]::WriteAllText((Join-Path (Resolve-Path $taskPublicStage).Path 'README.md'), $taskPortableReadme, [Text.UTF8Encoding]::new($false))
New-Item -ItemType Directory -Path (Join-Path $taskPublicStage 'docs') | Out-Null
foreach ($taskPublicDoc in @('COOKBOOK.md','AUDIO_RECOVERY.md','DEVICE_DISCOVERY_MEMORY.md','WORKSPACES.md','EFFECT_TUNING.md','TOOLS.md','STREAM_DECK.md','PLUGIN_DEVELOPMENT.md','WORKSPACE_LAYOUT.md','AUTO_DUCKING.md','LOUPEDECK.md','RECORDING.md','RACK_PROCESSORS.md','APPLICATION_CAPTURE.md','AUTOMATIC_VOICE_CORRECTION.md','VST_PLUGINS.md','VOICE_CHANGER.md','VOICE_TRAINING_NOTICES.txt','VOICE_TRAINING_BETA_VALIDATION.json')) {
    Copy-Item -LiteralPath (Join-Path 'docs' $taskPublicDoc) -Destination (Join-Path $taskPublicStage 'docs')
}
$taskGuideText = [IO.File]::ReadAllText((Resolve-Path 'docs/VOICE_TRAINING.md').Path)
$taskHistoryStart = $taskGuideText.IndexOf('The measured research history below')
if ($taskHistoryStart -lt 0) { throw 'Public training guide boundary was not found' }
[IO.File]::WriteAllText((Join-Path (Resolve-Path $taskPublicStage).Path 'docs/VOICE_TRAINING.md'), $taskGuideText.Substring(0, $taskHistoryStart), [Text.UTF8Encoding]::new($false))
Copy-ExtensionSdk (Join-Path $taskPublicStage 'sdk')
foreach ($taskMedia in @('audio','images','plugins')) {
    Copy-Item -LiteralPath (Join-Path 'dist/PirateWave/docs' $taskMedia) -Destination (Join-Path $taskPublicStage 'docs') -Recurse
}
Compress-Archive -Path "$taskPublicStage/*" -DestinationPath $taskArchive -Force
Write-Host "Built dist/PirateWave/PirateWave.exe and $taskArchive"
