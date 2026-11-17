# Builds Herb's Download Manager for Windows: one static, portable .exe (SPECIFICATION.md §1).
#   powershell -File windows\scripts\build.ps1 [-NoDesktop]
# Puts it in dist\ and, unless -NoDesktop, on the Desktop (windows\pending.md: copied there on every build), then
# runs its --self-check. Needs Rust (rustup, per user) and the Visual Studio Build Tools.
param([switch]$NoDesktop)
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
# The app's version for About, as scripts/version.sh counts it: VERSION, then the commits on main since VERSION last
# changed; a build with commits main hasn't got adds the branch and how many (0.1.157+windows-parity-drop.10), so a
# branch build never outranks main (core/build.rs reads version.txt).
$base = (Get-Content (Join-Path $root 'VERSION') -Raw).Trim()
$since = git -C $root log -1 --format=%H -- VERSION 2>$null
$patch = '0'; $extra = '0'
if ($since) {
    $point = 'HEAD'
    foreach ($ref in 'origin/main', 'main') {
        git -C $root rev-parse --verify -q "$ref^{commit}" 2>$null | Out-Null
        if ($LASTEXITCODE -eq 0) { $mb = git -C $root merge-base HEAD $ref 2>$null; if ($mb) { $point = $mb.Trim() }; break }
    }
    git -C $root merge-base --is-ancestor $since $point 2>$null
    if ($LASTEXITCODE -eq 0) {
        $patch = (git -C $root rev-list --count "$since..$point").Trim()
        $extra = (git -C $root rev-list --count "$point..HEAD").Trim()
    } else { $extra = (git -C $root rev-list --count "$since..HEAD").Trim() }
}
$version = "$base.$patch"
if ($extra -ne '0') {
    $branch = ((git -C $root rev-parse --abbrev-ref HEAD 2>$null) -replace '[^A-Za-z0-9-]', '-').Trim('-')
    if (-not $branch -or $branch -eq 'HEAD') { $branch = 'dev' }
    $version = "$version+$branch.$extra"
}
$global:LASTEXITCODE = 0
Set-Content -Path (Join-Path $root 'version.txt') -Value $version -NoNewline
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
    # Each new version of the browser extension goes to Downloads with its version in the name (Herbert, 2026-11-10).
    $version = (Get-Content (Join-Path $root 'extension\manifest.json') -Raw | ConvertFrom-Json).version
    $downloads = (New-Object -ComObject Shell.Application).NameSpace('shell:Downloads').Self.Path
    $ext = Join-Path $downloads "Herb's DM extension $version"
    if (Test-Path $ext) {
        Write-Host "Extension $version already in $downloads"
    } else {
        Copy-Item -Recurse (Join-Path $root 'extension') $ext
        Write-Host "Extension $version copied to $ext"
    }
}
Write-Host "Built dist\HerbsDownloadManager.exe ($((Get-Item $exe).Length / 1MB -as [int]) MB)"
