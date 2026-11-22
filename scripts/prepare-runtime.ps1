<#
.SYNOPSIS
  Stage app-local VC++ CRT DLLs and DirectML.dll for packaging (DESIGN 6).

.DESCRIPTION
  - Copies DirectML.dll (from ort-cache) and MSVCP140 / MSVCP140_1 / VCRUNTIME140 /
    VCRUNTIME140_1 into src-tauri/resources/runtime/ (installed alongside llm-translate.exe).
  - Copies MSVCP140 / VCRUNTIME140 / VCRUNTIME140_1 into src-tauri/resources/llama/
    (installed alongside llama-server.exe).
  Run after fetch-llama.ps1, fetch-ocr.ps1, and at least one cargo build (so ort-cache exists).
#>
$ErrorActionPreference = 'Stop'

$Root = Split-Path -Parent $PSScriptRoot
$RuntimeDest = Join-Path $Root 'src-tauri/resources/runtime'
$LlamaDest = Join-Path $Root 'src-tauri/resources/llama'

if (-not (Test-Path (Join-Path $LlamaDest 'llama-server.exe'))) {
    throw "Missing llama-server.exe in $LlamaDest. Run ./scripts/fetch-llama.ps1 first."
}

# Locate MSVC x64 CRT redistributable DLLs (prefer VS Redist, fall back to System32).
function Find-CrtDir {
    $candidates = @()
    if ($env:VCToolsRedistDir) {
        $candidates += Join-Path $env:VCToolsRedistDir 'x64\Microsoft.VC*.CRT'
    }
    foreach ($pf in @(${env:ProgramFiles(x86)}, $env:ProgramFiles)) {
        if ($pf) {
            $candidates += Join-Path $pf 'Microsoft Visual Studio\*\*\VC\Redist\MSVC\*\x64\Microsoft.VC*.CRT'
        }
    }
    $dirs = Get-Item -Path $candidates -ErrorAction SilentlyContinue | Sort-Object FullName -Descending
    foreach ($d in $dirs) {
        if (Test-Path (Join-Path $d.FullName 'msvcp140.dll')) {
            return $d.FullName
        }
    }
    $sys32 = Join-Path $env:SystemRoot 'System32'
    if (Test-Path (Join-Path $sys32 'msvcp140.dll')) {
        Write-Warning "VS Redist CRT directory not found; falling back to $sys32"
        return $sys32
    }
    throw 'Could not locate Visual C++ x64 runtime DLLs.'
}

# Locate real DirectML.dll extracted by ort-sys (skip 0-byte symlinks in target/<profile>).
function Find-DirectMl {
    $cache = Join-Path $Root 'src-tauri/target/ort-cache'
    if (Test-Path $cache) {
        $found = Get-ChildItem -Path $cache -Recurse -Filter 'DirectML.dll' -File -ErrorAction SilentlyContinue |
            Where-Object { $_.Length -gt 1MB } |
            Sort-Object LastWriteTime -Descending |
            Select-Object -First 1
        if ($found) { return $found.FullName }
    }
    foreach ($profile in @('release', 'debug')) {
        $p = Join-Path $Root "src-tauri/target/$profile/DirectML.dll"
        if (Test-Path $p) {
            $item = Get-Item $p
            if ($item.LinkType -and $item.Target) {
                $target = $item.Target | Select-Object -First 1
                if (Test-Path $target) { return $target }
            }
            if ($item.Length -gt 1MB) { return $item.FullName }
        }
    }
    throw "DirectML.dll not found under src-tauri/target/ort-cache. Run 'cargo build --manifest-path src-tauri/Cargo.toml --release' once first."
}

$crtDir = Find-CrtDir
$dmlPath = Find-DirectMl
Write-Host "VC++ CRT : $crtDir"
Write-Host "DirectML : $dmlPath"

New-Item -ItemType Directory -Force $RuntimeDest | Out-Null
Get-ChildItem -Path $RuntimeDest -File | Remove-Item -Force

$AppDlls = @('msvcp140.dll', 'msvcp140_1.dll', 'vcruntime140.dll', 'vcruntime140_1.dll')
$LlamaDlls = @('msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll')

foreach ($dll in $AppDlls) {
    $src = Join-Path $crtDir $dll
    if (-not (Test-Path $src)) { throw "Missing $src" }
    Copy-Item -Path $src -Destination (Join-Path $RuntimeDest $dll) -Force
}
Copy-Item -Path $dmlPath -Destination (Join-Path $RuntimeDest 'DirectML.dll') -Force

foreach ($dll in $LlamaDlls) {
    $src = Join-Path $crtDir $dll
    Copy-Item -Path $src -Destination (Join-Path $LlamaDest $dll) -Force
}

$total = (Get-ChildItem $RuntimeDest | Measure-Object Length -Sum).Sum / 1MB
Write-Host ("Runtime DLLs staged in {0} ({1:N1} MB) and VC++ CRT copied to {2}" -f $RuntimeDest, $total, $LlamaDest)
Get-ChildItem $RuntimeDest | ForEach-Object {
    Write-Host ("  {0,-22} {1,10:N0} B  v{2}" -f $_.Name, $_.Length, $_.VersionInfo.FileVersion)
}
