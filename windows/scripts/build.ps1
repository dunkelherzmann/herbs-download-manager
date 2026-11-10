# Builds Herb's Download Manager for Windows: one static, portable .exe (SPECIFICATION.md §1).
#   powershell -File windows\scripts\build.ps1 [-NoDesktop]
# Puts it in dist\ and, unless -NoDesktop, on the Desktop (windows\pending.md: copied there on every build), then
# runs its --self-check. Needs Rust (rustup, per user) and the Visual Studio Build Tools.
param([switch]$NoDesktop)
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
Push-Location (Join-Path $root 'windows')
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} finally { Pop-Location }
$exe = Join-Path $root 'windows\target\release\HerbsDownloadManager.exe'
$dist = Join-Path $root 'dist'
New-Item -ItemType Directory -Force $dist | Out-Null
Copy-Item $exe (Join-Path $dist 'HerbsDownloadManager.exe') -Force
# hdm, the command line and MCP server, beside the app.
Copy-Item (Join-Path $root 'windows\target\release\hdm.exe') (Join-Path $dist 'hdm.exe') -Force
& $exe --self-check | Out-Host
if (-not $NoDesktop) {
    Copy-Item $exe (Join-Path ([Environment]::GetFolderPath('Desktop')) 'Herbs Download Manager.exe') -Force
    Copy-Item (Join-Path $dist 'hdm.exe') (Join-Path ([Environment]::GetFolderPath('Desktop')) 'hdm.exe') -Force
    Write-Host 'Copied to the Desktop'
}
Write-Host "Built dist\HerbsDownloadManager.exe ($((Get-Item $exe).Length / 1MB -as [int]) MB)"
