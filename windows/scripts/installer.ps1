# Makes the Windows installer (windows\installer\HerbsDownloadManager.iss) from the app build.ps1 put in dist\:
#   powershell -ExecutionPolicy Bypass -File windows\scripts\installer.ps1
# Writes dist\Herbs-Download-Manager-<version>-Windows-Setup.exe. Needs Inno Setup 6 (winget install JRSoftware.InnoSetup).
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
$iscc = @(
    (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source,
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $iscc) { throw 'Inno Setup 6 is missing: winget install JRSoftware.InnoSetup' }
foreach ($f in 'HerbsDownloadManager.exe', 'hdm.exe') {
    if (-not (Test-Path (Join-Path $root "dist\$f"))) { throw "dist\$f is missing: run windows\scripts\build.ps1 first" }
}
$version = (Get-Content (Join-Path $root 'version.txt') -Raw).Trim()
& $iscc /Q "/DAppVersion=$version" (Join-Path $root 'windows\installer\HerbsDownloadManager.iss')
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$setup = Join-Path $root "dist\Herbs-Download-Manager-$version-Windows-Setup.exe"
Write-Host "Built $setup ($([math]::Round((Get-Item $setup).Length / 1MB, 1)) MB)"
