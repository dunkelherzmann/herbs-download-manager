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
# Without -Installer it downloads the latest release's installer (or -Version's). -Installer installs a Setup.exe you
# already have, once it matches its release's SHA256SUMS.txt (-SkipChecksum for a build of your own).
# A copy installed earlier for your account only is replaced by this one; downloads, settings and history stay. (When
# a different administrator account approves the UAC prompt, that account's copy is the one looked for.)
#
# Safety: everything that runs as administrator comes from a folder only administrators can write to and is checked
# there right before it runs, and the old per-account copy's uninstaller (which lives where your account can change
# it) runs without administrator rights.
#
# Plain ASCII on purpose: Windows PowerShell 5.1 misreads other characters in a file without a byte order mark.

function Install-HerbsDownloadManager {
    param(
        [string]$Version = '',
        [string]$Installer = '',
        [switch]$DesktopShortcut,
        [switch]$SkipChecksum
    )
    $ErrorActionPreference = 'Stop'
    $repo = 'dunkelherzmann/herbs-download-manager'
    $self = "https://github.com/$repo/releases/latest/download/install-windows.ps1"
    $appId = '{92541440-37F8-4FF0-8070-735886187A6A}_is1'
    $appName = "Herb's Download Manager"
    $uninstallKey = "Software\Microsoft\Windows\CurrentVersion\Uninstall\$appId"
    if ($Version -and $Version -notmatch '^v?\d+(\.\d+){1,3}$') { throw "-Version takes a version number such as 0.1.214, not '$Version'." }
    $Version = $Version.TrimStart('v')
    # Windows PowerShell 5.1 on older Windows 10 offers only TLS 1.0 by default, which GitHub refuses.
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    # 1. Administrator rights: ask for them and start again elevated.
    $admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
        [Security.Principal.WindowsBuiltInRole]::Administrator)
    if (-not $admin) {
        Write-Host 'Administrator rights are needed to install for all users; Windows will ask for them.'
        $pass = @()
        if ($Version) { $pass += "-Version $Version" }
        if ($DesktopShortcut) { $pass += '-DesktopShortcut' }
        if ($SkipChecksum) { $pass += '-SkipChecksum' }
        if ($PSCommandPath) {
            # -File takes double-quoted arguments (a Windows path can't hold a double quote).
            if ($Installer) { $pass += "-Installer `"$((Resolve-Path $Installer).Path)`"" }
            $argList = "-NoProfile -ExecutionPolicy Bypass -NoExit -File `"$PSCommandPath`" $($pass -join ' ')"
        } else {
            if ($Installer) { $pass += "-Installer '$((Resolve-Path $Installer).Path -replace "'", "''")'" }
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

    # 3. A folder only administrators (and the system) can write to: what runs elevated is put there, so nothing running
    # as you can swap it between its check and its start.
    $work = Join-Path $env:ProgramData ("HerbsDownloadManager-install-" + [guid]::NewGuid())
    New-Item -ItemType Directory -Path $work | Out-Null
    & icacls.exe $work /inheritance:r /grant:r '*S-1-5-32-544:(OI)(CI)F' '*S-1-5-18:(OI)(CI)F' | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Couldn't restrict $work to administrators." }
    $log = Join-Path $work 'install.log'
    try {
    $progress = $ProgressPreference; $ProgressPreference = 'SilentlyContinue'  # the progress bar slows 5.1 down a lot

    # 4. The installer, given or downloaded, and its release's checksum.
    if ($Installer) {
        $given = Get-Item (Resolve-Path $Installer).Path
        $name = $given.Name
        $setup = Join-Path $work $name
        Copy-Item -LiteralPath $given.FullName -Destination $setup
        Write-Host "Installing $($given.FullName)"
        if (-not $Version -and $name -match '^Herbs-Download-Manager-(\d+(\.\d+){1,3})-Windows-Setup\.exe$') { $Version = $Matches[1] }
    }
    $release = $null
    if (-not ($Installer -and $SkipChecksum)) {
        if ($Installer -and -not $Version) { throw "Can't tell which release $name is from: pass -Version, or -SkipChecksum for a build of your own." }
        $api = if ($Version) { "https://api.github.com/repos/$repo/releases/tags/v$Version" } else { "https://api.github.com/repos/$repo/releases/latest" }
        $release = Invoke-RestMethod $api -Headers @{ 'User-Agent' = 'install-windows.ps1' }
        $sums = $release.assets | Where-Object { $_.name -eq 'SHA256SUMS.txt' } | Select-Object -First 1
        if (-not $sums) { throw "Release $($release.tag_name) has no SHA256SUMS.txt, so nothing can be checked; nothing was installed." }
        $sumsText = (Invoke-WebRequest $sums.browser_download_url -UseBasicParsing).Content
        if ($sumsText -is [byte[]]) { $sumsText = [Text.Encoding]::ASCII.GetString($sumsText) }
    }
    if (-not $Installer) {
        $asset = $release.assets | Where-Object { $_.name -like '*-Windows-Setup.exe' } | Select-Object -First 1
        if (-not $asset) { throw "Release $($release.tag_name) has no Windows installer." }
        $name = $asset.name
        $setup = Join-Path $work $name
        Write-Host "Downloading $name ($($release.tag_name))"
        Invoke-WebRequest $asset.browser_download_url -OutFile $setup -UseBasicParsing
    }
    $ProgressPreference = $progress
    # The "downloaded from the internet" mark is what SmartScreen checks: without it the installer runs unprompted.
    Unblock-File $setup

    # 5. A copy installed for this account only is replaced by the all-users one (its data stays in %LOCALAPPDATA%).
    $mine = Get-ItemProperty "HKCU:\$uninstallKey" -ErrorAction SilentlyContinue
    $old = Get-ItemProperty "HKLM:\$uninstallKey" -ErrorAction SilentlyContinue
    # Only installed copies are closed; a copy run from a zip or a build folder is left alone.
    $dirs = @($mine.InstallLocation, $old.InstallLocation) | Where-Object { $_ } | ForEach-Object { $_.TrimEnd('\') + '\' }
    $running = @(Get-Process HerbsDownloadManager -ErrorAction SilentlyContinue | Where-Object {
        $path = $_.Path; $path -and @($dirs | Where-Object { $path.StartsWith($_, [StringComparison]::OrdinalIgnoreCase) }).Count })
    if ($running) {
        Write-Host "Closing the running $appName for the update."
        $running | Stop-Process -Force
        Start-Sleep -Seconds 1
    }
    if ($mine -and $mine.UninstallString) {
        Write-Host "Removing the copy installed for this account only ($($mine.InstallLocation))."
        # Its uninstaller sits in a folder your account can change, so it never runs as administrator here: a one-off
        # scheduled task runs it as you, unelevated. (A copy that was itself installed as administrator asks for
        # administrator rights to uninstall: Windows shows that prompt, and it's yours to answer.)
        $task = 'HerbsDownloadManager-remove-' + [guid]::NewGuid()
        $action = New-ScheduledTaskAction -Execute $mine.UninstallString.Trim('"') -Argument '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART'
        $principal = New-ScheduledTaskPrincipal -UserId ([Security.Principal.WindowsIdentity]::GetCurrent().Name) -LogonType Interactive -RunLevel Limited
        Register-ScheduledTask -TaskName $task -Action $action -Principal $principal | Out-Null
        try {
            Start-ScheduledTask -TaskName $task
            $deadline = (Get-Date).AddSeconds(120)
            while ((Test-Path "HKCU:\$uninstallKey") -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
        } finally {
            Stop-ScheduledTask -TaskName $task -ErrorAction SilentlyContinue
            Unregister-ScheduledTask -TaskName $task -Confirm:$false -ErrorAction SilentlyContinue
        }
        if (Test-Path "HKCU:\$uninstallKey") {
            Write-Warning "The copy for this account only is still there; remove it in Settings > Apps (the all-users one carries on)."
        }
    }

    # 6. Check the installer where only administrators can change it, then install for all users, silently.
    if ($SkipChecksum -and $Installer) {
        Write-Warning 'Not checked against a release (-SkipChecksum).'
    } else {
        $line = $sumsText -split "`n" | Where-Object { $_ -match ('\s\*?' + [regex]::Escape($name) + '\s*$') } | Select-Object -Last 1
        if (-not $line) { throw "SHA256SUMS.txt of $($release.tag_name) has no line for $name; nothing was installed." }
        $want = ($line -split '\s+')[0].ToLower()
        $got = (Get-FileHash $setup -Algorithm SHA256).Hash.ToLower()
        if ($want -ne $got) { throw "$name doesn't match SHA256SUMS.txt of $($release.tag_name) ($got, expected $want); nothing was installed." }
        Write-Host "Checksum matches SHA256SUMS.txt of $($release.tag_name)."
    }
    $setupArgs = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/ALLUSERS', '/NORESTARTAPPLICATIONS', "/LOG=`"$log`"")
    if ($DesktopShortcut) { $setupArgs += '/MERGETASKS=desktopicon' }
    Write-Host 'Installing for all users...'
    $p = Start-Process $setup -ArgumentList $setupArgs -Wait -PassThru
    if ($p.ExitCode -ne 0) { throw "The installer exited with $($p.ExitCode); its log is $log" }
    $all = Get-ItemProperty "HKLM:\$uninstallKey" -ErrorAction SilentlyContinue
    $dir = if ($all) { $all.InstallLocation.TrimEnd('\') } else { Join-Path $env:ProgramFiles $appName }
    $exe = Join-Path $dir 'HerbsDownloadManager.exe'
    if (-not (Test-Path $exe)) { throw "The installer finished but $exe isn't there; its log is $log" }
    Get-ChildItem $dir -Filter *.exe | Unblock-File
    } catch {
        # Kept only when the installer's log is in it, for finding out why.
        if (-not (Test-Path $log)) { Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue }
        throw
    }
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue

    # 7. Open it as the signed-in user, not as administrator (Explorer starts it without elevation).
    Start-Process explorer.exe -ArgumentList "`"$exe`""
    Write-Host ''
    Write-Host "$appName $($all.DisplayVersion) is installed in $dir, for all users."
    Write-Host 'It is in the Start menu; Settings > Apps removes it. This window can be closed.'
}

Install-HerbsDownloadManager @args
