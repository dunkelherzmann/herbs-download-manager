# Feature tests on Windows: tests\features.scenario run by the built app (a test copy with its own data folder), every
# step through the app's own command code. Prints one PASS or FAIL line per feature.
#   powershell -File windows\scripts\features.ps1     (after windows\scripts\build.ps1)
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
$work = Join-Path $env:TEMP 'hdm-features'
if (Test-Path $work) { Remove-Item -Recurse -Force $work }
New-Item -ItemType Directory -Force $work | Out-Null
cargo build -q --manifest-path (Join-Path $root 'Cargo.toml') -p hd-mock
$port = 20000 + (Get-Random -Maximum 20000)
$mockOut = Join-Path $work 'mock.txt'
$mock = Start-Process -FilePath (Join-Path $root 'target\debug\hd-mock.exe') -ArgumentList '--features', '--port', $port `
    -RedirectStandardOutput $mockOut -PassThru -WindowStyle Hidden
try {
    for ($i = 0; $i -lt 50 -and -not ((Test-Path $mockOut) -and (Select-String -Path $mockOut -Pattern '^sha256' -Quiet)); $i++) { Start-Sleep -Milliseconds 100 }
    $sum = ((Select-String -Path $mockOut -Pattern '^sha256').Line -split ' ')[2]
    $env:HD_DATA_DIR = Join-Path $work 'data'
    $env:HD_DOWNLOADS_DIR = Join-Path $work 'dl'
    $report = Join-Path $work 'report.txt'
    $app = Start-Process -FilePath (Join-Path $root 'dist\HerbsDownloadManager.exe') -PassThru -Wait -ArgumentList @(
        '--features', (Join-Path $root 'tests\features.scenario'), $report,
        "L=http://127.0.0.1:$port", "M1=http://127.0.0.1:$($port + 1)", "M2=http://127.0.0.1:$($port + 2)",
        "DL=$($env:HD_DOWNLOADS_DIR)", "SMALL_SHA256=$sum")
    if (Test-Path $report) { Get-Content $report | Write-Host } else { Write-Host 'the app wrote no report' }
    exit $app.ExitCode
} finally {
    Stop-Process -Id $mock.Id -Force -ErrorAction SilentlyContinue
}
