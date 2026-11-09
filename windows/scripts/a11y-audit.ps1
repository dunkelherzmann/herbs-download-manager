# Every control explains itself (§3, §4): opens each window of the app (a test copy with its own data folder) and
# fails if a button, field, list or dropdown has no hover text or no name a screen reader can say (a11y.rs).
#   powershell -File windows\scripts\a11y-audit.ps1     (after windows\scripts\build.ps1)
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
$work = Join-Path $env:TEMP 'hdm-a11y'
if (Test-Path $work) { Remove-Item -Recurse -Force $work }
New-Item -ItemType Directory -Force $work | Out-Null
$env:HD_DATA_DIR = Join-Path $work 'data'
$env:HD_DOWNLOADS_DIR = $work
$report = Join-Path $work 'report.txt'
$exe = Join-Path $root 'dist\HerbsDownloadManager.exe'
Start-Process -FilePath $exe -ArgumentList '--add', 'http://127.0.0.1:9/a.bin', '--a11y-report', $report -Wait
$lines = @(Get-Content $report)  # an array even when the report is one line
$lines | Where-Object { $_ -like '# *' } | Write-Host
$bad = $lines | Where-Object { $_ -and $_ -notlike '# *' }
foreach ($w in "Herb's Download Manager", 'Settings', 'New Download', 'Add 2 Links', 'Batch Download', 'a.bin') {
    if (-not ($lines[0] -like "*$w*")) { Write-Host "The $w window didn't open"; exit 1 }
}
if ($bad) { Write-Host 'Controls without hover text or a name:'; $bad | Write-Host; exit 1 }
Write-Host 'every control has hover text and a name'
