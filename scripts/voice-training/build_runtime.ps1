param(
    [Parameter(Mandatory=$true)][ValidateSet('cpu','cuda')][string]$Backend,
    [Parameter(Mandatory=$true)][string]$BuildPython,
    [Parameter(Mandatory=$true)][string]$Destination,
    [Parameter(Mandatory=$true)][string]$Archive
)
# BUILD MACHINE ONLY. The generated application runtime never invokes pip.
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$destinationPath = [IO.Path]::GetFullPath($Destination)
if (Test-Path -LiteralPath $destinationPath) { throw 'Choose a new build directory; existing runtimes are immutable.' }
$archiveParent = Split-Path -Parent ([IO.Path]::GetFullPath($Archive))
New-Item -ItemType Directory -Path $destinationPath -Force | Out-Null
New-Item -ItemType Directory -Path $archiveParent -Force | Out-Null
$embed = Join-Path $destinationPath 'python-embed.zip'
Invoke-WebRequest -Uri 'https://www.python.org/ftp/python/3.12.10/python-3.12.10-embed-amd64.zip' -OutFile $embed
if ((Get-FileHash -LiteralPath $embed -Algorithm SHA256).Hash.ToLowerInvariant() -ne '4acbed6dd1c744b0376e3b1cf57ce906f9dc9e95e68824584c8099a63025a3c3') { throw 'Embedded Python checksum mismatch' }
Expand-Archive -LiteralPath $embed -DestinationPath $destinationPath
# Retain the verified input archive outside the runtime instead of silently deleting it.
Move-Item -LiteralPath $embed -Destination (Join-Path $archiveParent "$Backend-python-input.zip")
$index = if ($Backend -eq 'cuda') { 'https://download.pytorch.org/whl/cu124' } else { 'https://download.pytorch.org/whl/cpu' }
& $BuildPython -m pip install --target (Join-Path $destinationPath 'Lib/site-packages') --python-version 3.12 --platform win_amd64 --implementation cp --abi cp312 --only-binary=:all: --extra-index-url $index -r (Join-Path $PSScriptRoot "requirements-embedded-$Backend.lock.txt")
if ($LASTEXITCODE -ne 0) { throw 'Prebuilt dependency installation failed' }
Set-Content -LiteralPath (Join-Path $destinationPath 'python312._pth') -Value @('python312.zip','.','Lib/site-packages','worker','import site') -Encoding ascii
& $BuildPython (Join-Path $PSScriptRoot 'package_runtime.py') --root $repo --runtime $destinationPath --backend $Backend --out $Archive
if ($LASTEXITCODE -ne 0) { throw 'Runtime packaging failed' }
& $BuildPython (Join-Path $PSScriptRoot 'check_runtime.py') --runtime $destinationPath --out ($Archive + '.isolation.json')
if ($LASTEXITCODE -ne 0) { throw 'Runtime isolation check failed' }
