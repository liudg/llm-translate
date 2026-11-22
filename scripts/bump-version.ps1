<#
.SYNOPSIS
  Sync the app version across package.json, src-tauri/Cargo.toml,
  src-tauri/tauri.conf.json, and src-tauri/Cargo.lock.

.EXAMPLE
  ./scripts/bump-version.ps1 -Version 0.2.0
#>
param(
    [Parameter(Mandatory)]
    [string]$Version
)

$ErrorActionPreference = 'Stop'

$Version = $Version.TrimStart('v')
if ($Version -notmatch '^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$') {
    throw "Invalid SemVer version: '$Version' (expected e.g. 0.2.0)"
}

$Root = Split-Path -Parent $PSScriptRoot
$Utf8NoBom = [System.Text.UTF8Encoding]::new($false)

function Update-RegexFile {
    param(
        [string]$Path,
        [string]$Pattern,
        [string]$Replacement
    )
    $raw = [System.IO.File]::ReadAllText($Path, $Utf8NoBom)
    $updated = [regex]::Replace($raw, $Pattern, $Replacement, 1)
    if ($updated -eq $raw) {
        # Allow idempotent re-run when version is already set.
        if (-not ($raw -match [regex]::Escape($Version))) {
            throw "Failed to update version in $Path"
        }
    }
    [System.IO.File]::WriteAllText($Path, $updated, $Utf8NoBom)
}

$pkgJson = Join-Path $Root 'package.json'
Update-RegexFile -Path $pkgJson `
    -Pattern '("version"\s*:\s*")[^"]+(")' `
    -Replacement "`${1}$Version`${2}"

$tauriConf = Join-Path $Root 'src-tauri/tauri.conf.json'
Update-RegexFile -Path $tauriConf `
    -Pattern '("version"\s*:\s*")[^"]+(")' `
    -Replacement "`${1}$Version`${2}"

$cargoToml = Join-Path $Root 'src-tauri/Cargo.toml'
Update-RegexFile -Path $cargoToml `
    -Pattern '(?m)^(version\s*=\s*")[^"]+(")' `
    -Replacement "`${1}$Version`${2}"

# Keep Cargo.lock in sync for the workspace package without touching dependencies.
Push-Location (Join-Path $Root 'src-tauri')
try {
    cargo metadata --format-version 1 --offline | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "cargo metadata failed while syncing Cargo.lock"
    }
} finally {
    Pop-Location
}

Write-Host "Updated version to $Version in:"
Write-Host "  - package.json"
Write-Host "  - src-tauri/Cargo.toml"
Write-Host "  - src-tauri/tauri.conf.json"
Write-Host "  - src-tauri/Cargo.lock"
Write-Host ""
Write-Host "Next steps to release:"
Write-Host "  git add package.json src-tauri/Cargo.toml src-tauri/tauri.conf.json src-tauri/Cargo.lock"
Write-Host "  git commit -m `"Release v$Version`""
Write-Host "  git tag -a v$Version -m `"v$Version`""
Write-Host "  git push origin HEAD --follow-tags"
