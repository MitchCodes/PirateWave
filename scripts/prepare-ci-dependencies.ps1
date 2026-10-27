param([string]$DestinationRoot = (Split-Path $PSScriptRoot -Parent))
$ErrorActionPreference = 'Stop'
$taskRoot = [IO.Path]::GetFullPath($DestinationRoot)
New-Item -ItemType Directory -Force $taskRoot | Out-Null
Add-Type -AssemblyName System.IO.Compression.FileSystem
function Get-VerifiedArchive([string]$Url, [string]$Sha256, [string]$Name) {
    $taskCache = Join-Path $taskRoot '.tools/ci-dependencies'
    New-Item -ItemType Directory -Force $taskCache | Out-Null
    $taskPath = Join-Path $taskCache $Name
    if (-not (Test-Path -LiteralPath $taskPath) -or (Get-FileHash -LiteralPath $taskPath -Algorithm SHA256).Hash -ne $Sha256) {
        try {
            Invoke-WebRequest -Uri $Url -OutFile "$taskPath.download" -MaximumRetryCount 2 -RetryIntervalSec 2
            if ((Get-FileHash -LiteralPath "$taskPath.download" -Algorithm SHA256).Hash -ne $Sha256) { throw "Checksum mismatch: $Name" }
            Move-Item -LiteralPath "$taskPath.download" -Destination $taskPath -Force
        } catch {
            throw "Cannot restore pinned CI dependency '$Name' from $Url : $($_.Exception.Message)"
        } finally {
            if (Test-Path -LiteralPath "$taskPath.download") { Remove-Item -LiteralPath "$taskPath.download" -Force }
        }
    }
    return $taskPath
}
function Copy-ArchiveEntry([IO.Compression.ZipArchive]$Archive, [string]$EntryName, [string]$RelativePath) {
    $taskEntry = $Archive.GetEntry($EntryName)
    if (-not $taskEntry) { throw "Required dependency missing from archive: $EntryName" }
    # Destinations are explicit constants below, never paths taken from an archive.
    $taskDestination = Join-Path $taskRoot $RelativePath
    New-Item -ItemType Directory -Force (Split-Path $taskDestination -Parent) | Out-Null
    [IO.Compression.ZipFileExtensions]::ExtractToFile($taskEntry, $taskDestination, $true)
}
$taskPortable = Get-VerifiedArchive 'https://github.com/MitchCodes/PirateWave/releases/download/support-assets-v1/PirateWave-native-runtime-v1.zip' '8f1dc9ba3347d0680745058af57a13228cdb2a4d4e90d76880d792e6b7dd123b' 'piratewave-native-runtime-v1.zip'
$taskZip = [IO.Compression.ZipFile]::OpenRead($taskPortable)
try {
    Copy-ArchiveEntry $taskZip 'PirateWaveVoiceWorker.exe' 'dist/voice-training/native-worker-candidate/PatchVoiceWorker.exe'
    Copy-ArchiveEntry $taskZip 'msvcp140_1.dll' 'dist/voice-pack/llvc-f8312-v1/msvcp140_1.dll'
    Copy-ArchiveEntry $taskZip 'VOICE_WORKER_THIRD_PARTY_NOTICES.md' 'dist/voice-pack/llvc-f8312-v1/WORKER_THIRD_PARTY_NOTICES.md'
} finally { $taskZip.Dispose() }
$taskSdkArchive = Get-VerifiedArchive 'https://api.nuget.org/v3-flatcontainer/logipluginapi/6.4.2.3567/logipluginapi.6.4.2.3567.nupkg' '048cd1fbdc5ec089702172906123b5216303e31a4d00a75e28879a5e10d6df17' 'logipluginapi-6.4.2.3567.nupkg'
$taskZip = [IO.Compression.ZipFile]::OpenRead($taskSdkArchive)
try {
    foreach ($taskFile in @('PluginApi.dll','PluginApi.xml','LogiEventTracing.dll','websocket-sharp.dll')) {
        Copy-ArchiveEntry $taskZip "lib/net10.0/$taskFile" ".tools/ci-dependencies/sdk/$taskFile"
    }
} finally { $taskZip.Dispose() }
$taskSdkDirectory = Join-Path $taskRoot '.tools/ci-dependencies/sdk'
if ($env:GITHUB_ENV) { Add-Content -LiteralPath $env:GITHUB_ENV -Value "PIRATEWAVE_LOUPEDECK_SDK_DIR=$taskSdkDirectory" -Encoding utf8 }
Write-Host "Verified portable runtime dependencies and Loupedeck SDK: $taskSdkDirectory"
