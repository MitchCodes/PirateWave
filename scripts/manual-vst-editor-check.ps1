param([string]$PluginPath = '', [int]$Seconds = 120)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)

if (-not $PluginPath) {
    Add-Type -AssemblyName System.Windows.Forms
    $picker = New-Object System.Windows.Forms.OpenFileDialog
    $picker.Title = 'Choose a VST plugin file to test'
    $picker.Filter = 'VST plugins (*.vst3;*.dll)|*.vst3;*.dll|All files (*.*)|*.*'
    if ($picker.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
        $PluginPath = $picker.FileName
    } else {
        $choice = [System.Windows.Forms.MessageBox]::Show('Choose a VST3 bundle folder instead?', 'Plugin editor check', [System.Windows.Forms.MessageBoxButtons]::YesNo)
        if ($choice -ne [System.Windows.Forms.DialogResult]::Yes) { return }
        $folder = New-Object System.Windows.Forms.FolderBrowserDialog
        $folder.Description = 'Choose the .vst3 bundle folder'
        if ($folder.ShowDialog() -ne [System.Windows.Forms.DialogResult]::OK) { return }
        $PluginPath = $folder.SelectedPath
    }
}

$metadata = (& cargo metadata --locked --no-deps --format-version 1) | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'Could not locate the test build' }
$exe = Join-Path $metadata.target_directory 'debug/piratewave-desktop.exe'
$isolated = Join-Path (Get-Location) 'dist/manual-vst-check/piratewave-desktop.exe'
if ((Test-Path -LiteralPath $isolated -PathType Leaf) -and
    (Test-Path -LiteralPath $exe -PathType Leaf) -and
    (Get-Item -LiteralPath $isolated).LastWriteTimeUtc -ge (Get-Item -LiteralPath $exe).LastWriteTimeUtc) {
    $exe = $isolated
}
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) {
    throw 'Build the debug application first with cargo build --locked -p piratewave-desktop'
}

Write-Host 'This test uses an isolated plugin instance and does not open your workspace or audio devices.'
Write-Host 'In the first editor, change a plugin-specific setting and try resizing or moving it between display scales.'
Write-Host 'Close the first editor to open a new instance, then check that the setting was restored.'
Write-Host "Each editor stays open for up to $Seconds seconds."
& $exe --plugin-editor-manual-check $PluginPath $Seconds
if ($LASTEXITCODE -ne 0) { throw 'The plugin editor check failed' }
