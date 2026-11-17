# The Windows part of scripts/release.sh: builds the commit in a throwaway worktree, makes the installer
# (windows\scripts\installer.ps1) and zips the app with hdm.exe, which must stay beside it (the browser's native host).
# Copied to the home folder and run there:
#   powershell -NoProfile -ExecutionPolicy Bypass -File hdm-release.ps1 -Commit SHA -Zip NAME.zip -Setup NAME.exe
param([Parameter(Mandatory)][string]$Commit, [Parameter(Mandatory)][string]$Zip, [Parameter(Mandatory)][string]$Setup)
$repo = "$env:USERPROFILE\Projects\herbs-download-manager"
$wt = "$env:USERPROFILE\Projects\wt-hdm-release"
$out = Join-Path $env:USERPROFILE $Zip
Set-Location $repo
git fetch -q origin
if (Test-Path $wt) { git worktree remove --force $wt 2>$null; Remove-Item -Recurse -Force $wt -ErrorAction SilentlyContinue }
git worktree prune
git worktree add -q --detach $wt $Commit
if ($LASTEXITCODE -ne 0) { exit 1 }
Set-Location $wt
git log --oneline -1
powershell -NoProfile -ExecutionPolicy Bypass -File windows\scripts\build.ps1 -NoDesktop
if ($LASTEXITCODE -ne 0) { exit 1 }
# The installer, from the same dist\ files.
powershell -NoProfile -ExecutionPolicy Bypass -File windows\scripts\installer.ps1
if ($LASTEXITCODE -ne 0) { exit 1 }
$version = (Get-Content version.txt -Raw).Trim()
Copy-Item "dist\Herbs-Download-Manager-$version-Windows-Setup.exe" (Join-Path $env:USERPROFILE $Setup) -Force
# In the zip, a folder holding both programs.
$stage = Join-Path $env:TEMP 'hdm-release'
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
$folder = Join-Path $stage 'Herbs Download Manager'
New-Item -ItemType Directory -Force $folder | Out-Null
Copy-Item dist\HerbsDownloadManager.exe, dist\hdm.exe $folder
Remove-Item $out -ErrorAction SilentlyContinue
Compress-Archive -Path $folder -DestinationPath $out
# The zipped app starts and passes its self-check.
$check = Join-Path $stage 'check'
Expand-Archive $out $check
& (Join-Path $check 'Herbs Download Manager\HerbsDownloadManager.exe') --self-check | Out-Host
$status = $LASTEXITCODE
Set-Location $repo
git worktree remove --force $wt
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
Write-Host "Zipped $out"
exit $status
