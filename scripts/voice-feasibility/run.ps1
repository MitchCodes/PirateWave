param([string]$Python = 'python', [switch]$SkipSetup, [ValidateSet(1,2,4)][int]$ChunkFactor = 1,
      [string]$OutputRoot = 'dist/voice-feasibility')
$ErrorActionPreference = 'Stop'
Set-Location (Resolve-Path (Join-Path $PSScriptRoot '../..'))
$taskRoot = Join-Path $PWD '.tools/voice-feasibility'
$taskSource = Join-Path $taskRoot 'LLVC'
$taskPython = Join-Path $taskRoot 'python/Scripts/python.exe'
$taskWeights = Join-Path $taskRoot 'llvc.pth'
$taskRevision = '1627c5d358cf9bb2b92b0ccc513d8b36807c923d'
$taskWeightRevision = 'ebfe8c0fdeb974a7eeb463b3abbc8ce42a0e3851'
New-Item -ItemType Directory -Force $taskRoot | Out-Null
if (-not $SkipSetup) {
    if (-not (Test-Path -LiteralPath $taskSource)) {
        & git clone https://github.com/KoeAI/LLVC.git $taskSource
        if ($LASTEXITCODE -ne 0) { throw 'LLVC clone failed' }
        & git -C $taskSource checkout --detach $taskRevision
        if ($LASTEXITCODE -ne 0) { throw 'LLVC checkout failed' }
    }
    if (-not (Test-Path -LiteralPath $taskPython)) {
        & $Python -m venv (Join-Path $taskRoot 'python')
        if ($LASTEXITCODE -ne 0) { throw 'Python environment creation failed' }
    }
    & $taskPython -m pip install -r "$PSScriptRoot/requirements.txt" --extra-index-url https://download.pytorch.org/whl/cpu
    if ($LASTEXITCODE -ne 0) { throw 'Export dependency installation failed' }
    if (-not (Test-Path -LiteralPath $taskWeights)) {
        Invoke-WebRequest "https://huggingface.co/KoeAI/llvc_models/resolve/$taskWeightRevision/models/checkpoints/llvc/G_500000.pth" -OutFile $taskWeights -UseBasicParsing
    }
}
$taskOutput = Join-Path $OutputRoot "hop$ChunkFactor"
if (Test-Path -LiteralPath "$taskOutput/fixture.json") {
    throw 'Existing fixtures/reports are preserved. Use another -OutputRoot to regenerate.'
}
New-Item -ItemType Directory -Force $taskOutput | Out-Null
& $taskPython "$PSScriptRoot/export_llvc.py" --upstream $taskSource --checkpoint $taskWeights --out $taskOutput --chunk-factor $ChunkFactor *> "$taskOutput/export.log"
if ($LASTEXITCODE -ne 0) { throw "Export/reference failed; see $taskOutput/export.log" }
$taskCargo = Join-Path $env:USERPROFILE '.cargo/bin/cargo.exe'
$env:CARGO_INCREMENTAL = '0'
$env:CARGO_BUILD_JOBS = '1'
& $taskCargo build --release --locked --manifest-path tools/voice-probe/Cargo.toml --target-dir target/voice-probe
if ($LASTEXITCODE -ne 0) { throw 'Rust probe build failed' }
$taskBin = Join-Path $PWD 'dist/voice-feasibility/bin'
New-Item -ItemType Directory -Force $taskBin | Out-Null
Copy-Item -LiteralPath 'target/voice-probe/release/piratewave-voice-probe.exe' -Destination $taskBin -Force
$taskCapi = Join-Path $taskRoot 'python/Lib/site-packages/onnxruntime/capi'
foreach ($taskDll in @('onnxruntime.dll','DirectML.dll','onnxruntime_providers_shared.dll')) {
    Copy-Item -LiteralPath (Join-Path $taskCapi $taskDll) -Destination $taskBin -Force
}
# Keep DirectML beside the executable so Windows resolves the matching runtime.
Copy-Item -LiteralPath (Join-Path $taskRoot 'python/Lib/site-packages/onnxruntime/LICENSE') -Destination "$taskBin/ONNX_RUNTIME_LICENSE.txt" -Force
Copy-Item -LiteralPath (Join-Path $taskRoot 'python/Lib/site-packages/onnxruntime/ThirdPartyNotices.txt') -Destination "$taskBin/ONNX_RUNTIME_NOTICES.txt" -Force
Copy-Item -LiteralPath (Join-Path $taskSource 'LICENSE') -Destination (Join-Path $PWD 'dist/voice-feasibility/LLVC_LICENSE.txt') -Force
$taskRuntime = Join-Path $taskBin 'onnxruntime.dll'
$taskProbe = Join-Path $taskBin 'piratewave-voice-probe.exe'
$taskResults = @()
foreach ($taskProfile in @(@('cpu',1), @('cpu',2), @('directml',1))) {
    $taskBackend = $taskProfile[0]
    $taskThreads = $taskProfile[1]
    $taskName = "$taskBackend-$taskThreads"
    & $taskProbe $taskOutput $taskRuntime $taskBackend $taskThreads "$taskOutput/$taskName.json" *> "$taskOutput/$taskName.log"
    $taskResults += [PSCustomObject]@{ profile=$taskName; exitCode=$LASTEXITCODE; report="$taskName.json"; log="$taskName.log" }
}
$taskHardware = [PSCustomObject]@{
    cpu=(Get-CimInstance Win32_Processor | Select-Object Name,NumberOfCores,NumberOfLogicalProcessors)
    graphics=(Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion)
    runtimeSha256=(Get-FileHash -LiteralPath $taskRuntime -Algorithm SHA256).Hash.ToLowerInvariant()
    profiles=$taskResults
    gate='NOT QUALIFIED: one voice is in scope. Run intermediate-state qualification; offline timing alone cannot enable production routing.'
}
$taskHardware | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath "$taskOutput/summary.json" -Encoding utf8
Write-Host "Measured feasibility results: $taskOutput"
Write-Host 'Run qualify.ps1 for both chunk profiles. One voice is in scope; production validation remains required.'
