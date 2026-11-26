# Installs Herb's Download Manager on Windows 10 or 11 (64-bit) for all users, without the "Windows protected your PC"
# (SmartScreen) warning. The app isn't code-signed, and this script is how to install it cleanly anyway.
#
# NOTE: run it as administrator. It installs into Program Files for every account on the computer, which needs
# administrator rights; if it isn't elevated it asks Windows for them (the UAC prompt) and starts again.
#
# NOTE: why SmartScreen stays quiet. SmartScreen only checks files that carry the "downloaded from the internet" mark
# a browser adds to what it saves. This script fetches the installer itself (PowerShell adds no mark), checks it
# against the release's SHA256SUMS.txt, removes the mark from any installer you give it, and runs it silently. The
# installed programs carry no mark either, so they open without a warning too. Nothing in Windows' security settings
# is changed: no Defender exclusion, SmartScreen stays on for everything else.
#
# Smart App Control (Windows 11, Settings > Privacy & security > Windows Security > App & browser control) is the one
# thing it can't get past: when it's On, Windows blocks every program that isn't signed, however it got there. The
# script stops and says so rather than turning it off.
#
# Easiest: open PowerShell as administrator (right-click Start > Terminal (Admin)) and paste
#     irm https://github.com/dunkelherzmann/herbs-download-manager/releases/latest/download/install-windows.ps1 | iex
# From a saved copy (a downloaded .ps1 needs the policy bypass to run):
#     powershell -ExecutionPolicy Bypass -File install-windows.ps1 [-Version 0.1.212] [-Installer PATH] [-DesktopShortcut]
# -Installer installs a Setup.exe you already have (it's unblocked first). Without it, the script uses a
# Herbs-Download-Manager-*-Windows-Setup.exe beside itself, or else downloads the latest release (or -Version's).
# A copy installed earlier for your account only is replaced by this one; downloads, settings and history stay. (When
# a different administrator account approves the UAC prompt, that account's copy is the one looked for.)
#
# Plain ASCII on purpose: Windows PowerShell 5.1 misreads other characters in a file without a byte order mark.

function Install-HerbsDownloadManager {
    param(
        [string]$Version = '',
        [string]$Installer = '',
        [switch]$DesktopShortcut
    )
    $ErrorActionPreference = 'Stop'
    $repo = 'dunkelherzmann/herbs-download-manager'
    $self = "https://github.com/$repo/releases/latest/download/install-windows.ps1"
    $appId = '{92541440-37F8-4FF0-8070-735886187A6A}_is1'
    $appName = "Herb's Download Manager"
    # Windows PowerShell 5.1 on older Windows 10 offers only TLS 1.0 by default, which GitHub refuses.
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    # 1. Administrator rights: ask for them and start again elevated.
    $admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
        [Security.Principal.WindowsBuiltInRole]::Administrator)
    if (-not $admin) {
        Write-Host 'Administrator rights are needed to install for all users; Windows will ask for them.'
        $pass = @()
        if ($Version) { $pass += "-Version '$Version'" }
        if ($Installer) { $pass += "-Installer '$((Resolve-Path $Installer).Path -replace "'", "''")'" }
        if ($DesktopShortcut) { $pass += '-DesktopShortcut' }
        if ($PSCommandPath) {
            $argList = "-NoProfile -ExecutionPolicy Bypass -NoExit -File `"$PSCommandPath`" $($pass -join ' ')"
        } else {
            # Run with irm | iex: the elevated window fetches the script again.
            $argList = "-NoProfile -ExecutionPolicy Bypass -NoExit -Command `"[Net.ServicePointManager]::SecurityProtocol = 'Tls12'; & ([scriptblock]::Create((irm '$self'))) $($pass -join ' ')`""
        }
        try {
            Start-Process powershell.exe -Verb RunAs -ArgumentList $argList
        } catch {
            throw 'Administrator rights were refused, so nothing was installed. Run it again and click Yes.'
        }
        return
    }

    # 2. Smart App Control blocks unsigned programs outright (0 off, 1 on, 2 evaluating).
    $sac = (Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\CI\Policy' -Name VerifiedAndReputablePolicyState -ErrorAction SilentlyContinue).VerifiedAndReputablePolicyState
    if ($sac -eq 1) {
        throw ("Smart App Control is On, and it blocks every program that isn't signed, this one included. " +
               "Nothing was installed. Turning it off (Windows Security > App & browser control > Smart App Control) " +
               "is your call: Windows can't turn it back on without a reset.")
    }
    if ($sac -eq 2) {
        Write-Warning 'Smart App Control is in evaluation mode: if Windows blocks the app, it has decided to turn itself On (see the notes at the top of this script).'
    }

    # 3. The installer: given, beside the script, or downloaded from the release and checked against SHA256SUMS.txt.
    $temp = $null
    if (-not $Installer -and $PSCommandPath) {
        $near = Get-ChildItem (Split-Path $PSCommandPath) -Filter 'Herbs-Download-Manager-*-Windows-Setup.exe' -ErrorAction SilentlyContinue |
            Sort-Object LastWriteTime -Descending | Select-Object -First 1
        if ($near) { $Installer = $near.FullName }
    }
    if ($Installer) {
        $Installer = (Resolve-Path $Installer).Path
        Write-Host "Installing $Installer"
    } else {
        $api = if ($Version) { "https://api.github.com/repos/$repo/releases/tags/v$($Version.TrimStart('v'))" } else { "https://api.github.com/repos/$repo/releases/latest" }
        $release = Invoke-RestMethod $api -Headers @{ 'User-Agent' = 'install-windows.ps1' }
        $asset = $release.assets | Where-Object { $_.name -like '*-Windows-Setup.exe' } | Select-Object -First 1
        $sums = $release.assets | Where-Object { $_.name -eq 'SHA256SUMS.txt' } | Select-Object -First 1
        if (-not $asset) { throw "Release $($release.tag_name) has no Windows installer." }
        $temp = Join-Path ([IO.Path]::GetTempPath()) ("hdm-install-" + [guid]::NewGuid())
        New-Item -ItemType Directory -Path $temp | Out-Null
        $Installer = Join-Path $temp $asset.name
        Write-Host "Downloading $($asset.name) ($($release.tag_name))"
        $progress = $ProgressPreference; $ProgressPreference = 'SilentlyContinue'  # the progress bar slows 5.1 down a lot
        Invoke-WebRequest $asset.browser_download_url -OutFile $Installer -UseBasicParsing
        $ProgressPreference = $progress
        if ($sums) {
            $line = (Invoke-WebRequest $sums.browser_download_url -UseBasicParsing).Content -split "`n" |
                Where-Object { $_ -match [regex]::Escape($asset.name) + '\s*$' } | Select-Object -First 1
            if (-not $line) { throw "SHA256SUMS.txt has no line for $($asset.name)." }
            $want = ($line -split '\s+')[0].ToLower()
            $got = (Get-FileHash $Installer -Algorithm SHA256).Hash.ToLower()
            if ($want -ne $got) { throw "The download doesn't match SHA256SUMS.txt ($got, expected $want); nothing was installed." }
            Write-Host 'Checksum matches SHA256SUMS.txt.'
        } else {
            Write-Warning 'This release has no SHA256SUMS.txt, so the download could not be checked.'
        }
    }
    # The "downloaded from the internet" mark is what SmartScreen checks: without it the installer runs unprompted.
    Unblock-File $Installer

    # 4. A copy installed for this account only is replaced by the all-users one (its data stays in %LOCALAPPDATA%).
    $running = @(Get-Process HerbsDownloadManager -ErrorAction SilentlyContinue)
    if ($running) {
        Write-Host "Closing the running $appName for the update."
        $running | Stop-Process -Force
        Start-Sleep -Seconds 1
    }
    $mine = Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$appId" -ErrorAction SilentlyContinue
    if ($mine -and $mine.UninstallString) {
        Write-Host "Removing the copy installed for this account only ($($mine.InstallLocation))."
        $uninstaller = $mine.UninstallString.Trim('"')
        $p = Start-Process $uninstaller -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -Wait -PassThru
        if ($p.ExitCode -ne 0) { Write-Warning "Its uninstaller exited with $($p.ExitCode); carrying on." }
    }

    # 5. Install for all users, silently.
    $log = Join-Path ([IO.Path]::GetTempPath()) 'hdm-install.log'
    $setupArgs = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/ALLUSERS', '/NORESTARTAPPLICATIONS', "/LOG=`"$log`"")
    if ($DesktopShortcut) { $setupArgs += '/MERGETASKS=desktopicon' }
    Write-Host 'Installing for all users...'
    $p = Start-Process $Installer -ArgumentList $setupArgs -Wait -PassThru
    if ($temp) { Remove-Item $temp -Recurse -Force -ErrorAction SilentlyContinue }
    if ($p.ExitCode -ne 0) { throw "The installer exited with $($p.ExitCode); its log is $log" }
    $all = Get-ItemProperty "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$appId" -ErrorAction SilentlyContinue
    $dir = if ($all) { $all.InstallLocation.TrimEnd('\') } else { Join-Path $env:ProgramFiles $appName }
    $exe = Join-Path $dir 'HerbsDownloadManager.exe'
    if (-not (Test-Path $exe)) { throw "The installer finished but $exe isn't there; its log is $log" }
    Get-ChildItem $dir -Filter *.exe | Unblock-File

    # 6. Open it as the signed-in user, not as administrator (Explorer starts it without elevation).
    Start-Process explorer.exe -ArgumentList "`"$exe`""
    Write-Host ''
    Write-Host "$appName $($all.DisplayVersion) is installed in $dir, for all users."
    Write-Host 'It is in the Start menu; Settings > Apps removes it. This window can be closed.'
}

Install-HerbsDownloadManager @args
