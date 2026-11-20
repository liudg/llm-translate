<#
.SYNOPSIS
  Fetch the pinned llama.cpp Vulkan release into src-tauri/resources/llama/.

.EXAMPLE
  ./scripts/fetch-llama.ps1
  ./scripts/fetch-llama.ps1 -Zip D:\Downloads\llama-b12541-bin-win-vulkan-x64.zip
#>
param(
    # Use an already downloaded release zip instead of downloading it.
    [string]$Zip
)

$ErrorActionPreference = 'Stop'

$Version = 'b12541'
$Url = "https://github.com/ggml-org/llama.cpp/releases/download/$Version/llama-$Version-bin-win-vulkan-x64.zip"
$Sha256 = 'bdb6fa660f7bd6c0eafa559cf005bef4c7ff2cb46acfb8059b2b724be822a501'

# Only what llama-server needs (about 89 MB).
$Files = @(
    'llama-server.exe', 'llama-server-impl.dll', 'llama-common.dll', 'llama.dll', 'mtmd.dll',
    'ggml.dll', 'ggml-base.dll', 'ggml-vulkan.dll', 'ggml-cpu-*.dll', 'libomp.dll', 'LICENSE-LLVM-OpenMP'
)

$Root = Split-Path -Parent $PSScriptRoot
$Dest = Join-Path $Root 'src-tauri/resources/llama'
$Work = Join-Path ([IO.Path]::GetTempPath()) "llama-$Version-fetch"

if (-not $Zip) {
    New-Item -ItemType Directory -Force $Work | Out-Null
    $Zip = Join-Path $Work "llama-$Version-bin-win-vulkan-x64.zip"
    if (-not (Test-Path $Zip) -or (Get-FileHash $Zip -Algorithm SHA256).Hash -ne $Sha256) {
        Write-Host "Downloading $Url"
        Invoke-WebRequest -Uri $Url -OutFile $Zip
    }
}

$hash = (Get-FileHash $Zip -Algorithm SHA256).Hash
if ($hash -ne $Sha256) {
    throw "SHA256 mismatch for ${Zip}: $hash (expected $Sha256)"
}

$Extract = Join-Path $Work 'extract'
if (Test-Path $Extract) { Remove-Item -Recurse -Force $Extract }
Expand-Archive -Path $Zip -DestinationPath $Extract

if (Test-Path $Dest) { Remove-Item -Recurse -Force $Dest }
New-Item -ItemType Directory -Force $Dest | Out-Null
foreach ($pattern in $Files) {
    $found = Get-ChildItem -Path $Extract -Recurse -Filter $pattern
    if (-not $found) { throw "Missing $pattern in the release zip" }
    $found | Copy-Item -Destination $Dest
}
Set-Content -Path (Join-Path $Dest 'VERSION') -Value $Version -NoNewline
Remove-Item -Recurse -Force $Extract

$size = (Get-ChildItem $Dest | Measure-Object Length -Sum).Sum / 1MB
Write-Host ("llama.cpp {0} -> {1} ({2:N0} MB)" -f $Version, $Dest, $size)
