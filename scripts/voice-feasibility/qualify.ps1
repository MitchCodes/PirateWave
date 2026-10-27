param(
    [string]$FixtureRoot = 'dist/voice-feasibility',
    [string]$ReportRoot = ('dist/voice-feasibility/qualification-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
)
$ErrorActionPreference = 'Stop'
Set-Location (Resolve-Path (Join-Path $PSScriptRoot '../..'))
$taskPython = Join-Path $PWD '.tools/voice-feasibility/python/Scripts/python.exe'
$taskProbe = Join-Path $PWD 'dist/voice-feasibility/bin/piratewave-voice-probe.exe'
$taskRuntime = Join-Path $PWD 'dist/voice-feasibility/bin/onnxruntime.dll'
if (Test-Path -LiteralPath $ReportRoot) { throw 'Use a new ReportRoot to preserve existing evidence.' }
foreach ($taskHop in @(1,2)) {
    if (-not (Test-Path -LiteralPath "$FixtureRoot/hop$taskHop/fixture.json")) {
        throw "Generate hop$taskHop fixtures before qualification."
    }
}
New-Item -ItemType Directory -Path $ReportRoot | Out-Null
$taskResults = @()
# Run diagnostic references first; never benchmark alongside a PyTorch job.
foreach ($taskHop in @(1,2)) {
    foreach ($taskBackend in @('cpu','directml')) {
        $taskName = "hop$taskHop-$taskBackend-state"
        & $taskPython "$PSScriptRoot/diagnose_states.py" `
            --upstream .tools/voice-feasibility/LLVC `
            --checkpoint .tools/voice-feasibility/llvc.pth `
            --fixture "$FixtureRoot/hop$taskHop" `
            --backend $taskBackend --seconds 12 `
            --native-repro "$ReportRoot/$taskName-repro" `
            --out "$ReportRoot/$taskName.json" *> "$ReportRoot/$taskName.log"
        $taskResults += [PSCustomObject]@{ profile=$taskName; exitCode=$LASTEXITCODE }
        if (Test-Path -LiteralPath "$ReportRoot/$taskName-repro/fixture.json") {
            & $taskProbe "$ReportRoot/$taskName-repro" $taskRuntime $taskBackend 1 `
                "$ReportRoot/$taskName-native.json" *> "$ReportRoot/$taskName-native.log"
            $taskResults += [PSCustomObject]@{ profile="$taskName-native"; exitCode=$LASTEXITCODE }
        }
    }
}
foreach ($taskHop in @(1,2)) {
    foreach ($taskProfile in @(@('cpu',1),@('cpu',2),@('directml',1))) {
        $taskBackend = $taskProfile[0]
        $taskThreads = $taskProfile[1]
        $taskName = "hop$taskHop-$taskBackend-$taskThreads"
        & $taskProbe "$FixtureRoot/hop$taskHop" $taskRuntime $taskBackend $taskThreads `
            "$ReportRoot/$taskName.json" *> "$ReportRoot/$taskName.log"
        $taskResults += [PSCustomObject]@{ profile=$taskName; exitCode=$LASTEXITCODE }
    }
}
[PSCustomObject]@{
    schema=1
    profiles=$taskResults
    cpu=(Get-CimInstance Win32_Processor | Select-Object Name,NumberOfCores,NumberOfLogicalProcessors)
    graphics=(Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion)
    probeSha256=(Get-FileHash -LiteralPath $taskProbe -Algorithm SHA256).Hash.ToLowerInvariant()
    runtimeSha256=(Get-FileHash -LiteralPath $taskRuntime -Algorithm SHA256).Hash.ToLowerInvariant()
    releaseQualified=$false
    note='Offline checks only. A passing subset does not qualify live routing, 30-minute stability, game impact, or listening.'
} | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath "$ReportRoot/summary.json" -Encoding utf8
Write-Host "Evidence saved to $ReportRoot"
if ($taskResults | Where-Object { $_.exitCode -ne 0 }) { exit 1 }
