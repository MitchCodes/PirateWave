param([Parameter(Mandatory)][string]$CargoExe, [Parameter(Mandatory)][string]$OutputPath, [string]$ManifestPath='Cargo.toml')
$ErrorActionPreference = 'Stop'
$taskMetadataText = & $CargoExe metadata --manifest-path $ManifestPath --format-version 1 --locked --filter-platform x86_64-pc-windows-msvc
if ($LASTEXITCODE -ne 0) { throw 'Cannot enumerate dependency licenses' }
$taskMetadata = $taskMetadataText | ConvertFrom-Json
$taskResolved = [System.Collections.Generic.HashSet[string]]::new()
foreach ($taskNode in $taskMetadata.resolve.nodes) { [void]$taskResolved.Add($taskNode.id) }
$taskNotices = [System.Text.StringBuilder]::new()
[void]$taskNotices.AppendLine('# Third-party notices')
[void]$taskNotices.AppendLine('PirateWave contains the following Rust dependencies (local modifications are identified below). Source distributions are available at the linked crates.io pages. Declared license expressions and shipped license notices follow. Some development dependencies may also be listed.')
foreach ($taskPackage in ($taskMetadata.packages | Sort-Object name,version)) {
    $taskPackageRoot = Split-Path -Parent $taskPackage.manifest_path
    $taskModifications = Join-Path $taskPackageRoot 'PIRATEWAVE_CHANGES.md'
    $taskVendored = Test-Path -LiteralPath $taskModifications
    if ((-not $taskPackage.source -and -not $taskVendored) -or -not $taskResolved.Contains($taskPackage.id)) { continue }
    [void]$taskNotices.AppendLine("`n## $($taskPackage.name) $($taskPackage.version)")
    [void]$taskNotices.AppendLine("Declared license: $($taskPackage.license)")
    [void]$taskNotices.AppendLine("Source: https://crates.io/crates/$($taskPackage.name)/$($taskPackage.version)")
    $taskPackageRoot = Split-Path -Parent $taskPackage.manifest_path
    if ($taskVendored) {
        [void]$taskNotices.AppendLine((Get-Content -LiteralPath $taskModifications -Raw -Encoding utf8))
    }
    $taskFiles = @(Get-ChildItem -LiteralPath $taskPackageRoot -File | Where-Object { $_.Name -match '^(LICEN[CS]E|COPYING|NOTICE|UNLICENSE)([._-].*)?$' })
    if ($taskPackage.license_file) {
        $taskLicensePath = Join-Path $taskPackageRoot $taskPackage.license_file
        if (Test-Path -LiteralPath $taskLicensePath -PathType Leaf) { $taskFiles += Get-Item -LiteralPath $taskLicensePath }
    }
    foreach ($taskFile in ($taskFiles | Sort-Object FullName -Unique)) {
        [void]$taskNotices.AppendLine("`n### $($taskFile.Name)`n")
        [void]$taskNotices.AppendLine((Get-Content -LiteralPath $taskFile.FullName -Raw -Encoding utf8))
    }
}
Set-Content -LiteralPath $OutputPath -Value $taskNotices.ToString() -Encoding utf8
