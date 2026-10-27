param([string]$PirateWaveExe = '', [switch]$Verify)
$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path $PSScriptRoot -Parent
Push-Location (Join-Path $taskRoot 'integrations/streamdeck')
try {
    if ([int](& node -p 'process.versions.node.split(".")[0]') -lt 24) { throw 'Stream Deck builds require Node.js 24+' }
    & npm.cmd ci --ignore-scripts
    if ($LASTEXITCODE -ne 0) { throw 'Stream Deck dependency restore failed' }
    foreach ($taskStep in @('check','build','test','validate','pack')) {
        & npm.cmd run $taskStep
        if ($LASTEXITCODE -ne 0) { throw "Stream Deck $taskStep failed" }
    }
    $taskPacked = Join-Path $taskRoot 'dist/com.piratewave.streamdeck.streamDeckPlugin'
    Copy-Item -LiteralPath $taskPacked -Destination (Join-Path $taskRoot 'dist/PirateWave.streamDeckPlugin') -Force
    & node scripts/verify-package.mjs (Join-Path $taskRoot 'dist/PirateWave.streamDeckPlugin')
    if ($LASTEXITCODE -ne 0) { throw 'Stream Deck package content verification failed' }
    if ($Verify) {
        if (-not $PirateWaveExe) { $PirateWaveExe = Join-Path $taskRoot 'target/debug/piratewave-desktop.exe' }
        & node test/native.mjs $PirateWaveExe
        if ($LASTEXITCODE -ne 0) { throw 'Stream Deck native pipe integration failed' }
    }
} finally { Pop-Location }
Write-Host 'Built verified dist/PirateWave.streamDeckPlugin. Install explicitly in Stream Deck; existing profiles are unchanged.'
