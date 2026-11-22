<#
.SYNOPSIS
  Fetch the PP-OCRv6 Small models (about 31 MB) into src-tauri/resources/ocr/.

.DESCRIPTION
  Same files and SHA256 as the oar-ocr 0.11 model registry (ModelScope repo
  greatv/oar-ocr). Files already present with the right hash are skipped.

.EXAMPLE
  ./scripts/fetch-ocr.ps1
#>
$ErrorActionPreference = 'Stop'

$Base = 'https://www.modelscope.cn/api/v1/models/greatv/oar-ocr/repo?Revision=master&FilePath='
$Files = [ordered]@{
    'pp-ocrv6_small_det.onnx' = 'd73e0058b7a8086bbd57f3d10b8bcd4ff95363f67e06e2762b5e814fe9c9410e'
    'pp-ocrv6_small_rec.onnx' = '5435fd747c9e0efe15a96d0b378d5bd157e9492ed8fd80edf08f30d02fa24634'
    'ppocrv6_dict.txt'        = 'b5f2bfe2bdd9448429e3e82b51c789775d9b42f2403d082b00662eb77e401c5d'
}

$Root = Split-Path -Parent $PSScriptRoot
$Dest = Join-Path $Root 'src-tauri/resources/ocr'
New-Item -ItemType Directory -Force $Dest | Out-Null

foreach ($name in $Files.Keys) {
    $want = $Files[$name]
    $path = Join-Path $Dest $name
    if ((Test-Path $path) -and (Get-FileHash $path -Algorithm SHA256).Hash -eq $want) {
        Write-Host "ok       $name"
        continue
    }
    Write-Host "download $name"
    $tmp = "$path.part"
    $maxAttempts = 4
    for ($attempt = 1; $attempt -le $maxAttempts; $attempt++) {
        try {
            Invoke-WebRequest -Uri ($Base + $name) -OutFile $tmp -UseBasicParsing
            $got = (Get-FileHash $tmp -Algorithm SHA256).Hash
            if ($got -ne $want) {
                Remove-Item -Force -ErrorAction SilentlyContinue $tmp
                throw "SHA256 mismatch for ${name}: $got"
            }
            break
        } catch {
            Remove-Item -Force -ErrorAction SilentlyContinue $tmp
            if ($attempt -eq $maxAttempts) {
                throw
            }
            $wait = [math]::Pow(2, $attempt)
            Write-Warning "Attempt $attempt failed for ${name}: $($_.Exception.Message). Retrying in ${wait}s..."
            Start-Sleep -Seconds $wait
        }
    }
    Move-Item -Force $tmp $path
}
Write-Host "OCR models ready in $Dest"
