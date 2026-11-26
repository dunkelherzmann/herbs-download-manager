; The Windows installer (SPECIFICATION.md §1): puts the app and hdm.exe (the browser extension's helper, which must
; stay beside it) in the user's own Programs folder, adds Start menu and optional desktop shortcuts, and an uninstaller
; in Settings ▸ Apps. No administrator rights; nothing outside the user's account. With /ALLUSERS on its command line
; (windows/install-windows.ps1, run as administrator) it installs for every account in Program Files instead.
; Built by windows\scripts\installer.ps1 with Inno Setup from the files build.ps1 put in dist\.
;
; Release installers are built by .github/workflows/release-windows.yml and aren't code-signed: opened from a browser
; download, SmartScreen stops them the first time with "Windows protected your PC" (More info ▸ Run anyway lets them
; run); install-windows.ps1 installs without that warning (README.md, the release notes).

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
; Windows' numeric file version can't hold a branch build's "+dev.N" (scripts/version.sh): it gets the part before.
#define NumVersion (Pos("+", AppVersion) > 0 ? Copy(AppVersion, 1, Pos("+", AppVersion) - 1) : AppVersion)
#define AppName "Herb's Download Manager"
#define AppExe "HerbsDownloadManager.exe"
#define Repo "https://github.com/dunkelherzmann/herbs-download-manager"

[Setup]
AppId={{92541440-37F8-4FF0-8070-735886187A6A}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=dunkelherzmann
AppPublisherURL={#Repo}
AppSupportURL={#Repo}
AppUpdatesURL={#Repo}/releases
AppCopyright=Copyright (c) 2026 Herbert Cisneros
VersionInfoVersion={#NumVersion}
; The product name and version are the app's, the same as in its programs' file details.
VersionInfoTextVersion={#AppVersion}
VersionInfoProductName={#AppName}
VersionInfoProductTextVersion={#AppVersion}
VersionInfoDescription={#AppName} Setup
; {autopf} is %LOCALAPPDATA%\Programs for one account, Program Files with /ALLUSERS.
DefaultDirName={autopf}\{#AppName}
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=commandline
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
SetupIconFile=..\icons\app.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
; The wizard follows Windows' light or dark app mode, as the app does.
WizardStyle=modern dynamic
Compression=lzma2/max
SolidCompression=yes
OutputDir=..\..\dist
OutputBaseFilename=Herbs-Download-Manager-{#AppVersion}-Windows-Setup
; A running copy is closed for the update (it asks first) and opened again afterwards.
CloseApplications=yes
RestartApplications=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[Files]
Source: "..\..\dist\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\dist\hdm.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"; Comment: "Download files over many connections at once"; AppUserModelID: "dunkelherzmann.HerbsDownloadManager"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "Open {#AppName}"; Flags: nowait postinstall skipifsilent

[Code]
{ The app tells the browsers where hdm.exe is each time it starts (SPECIFICATION.md §13): registry entries that name a
  manifest in the data folder, which names hdm.exe, and Edge's entry for the extension from Edge Add-ons.
  Uninstalling removes those entries only when they lead to this
  installed copy, so a portable copy that registered itself later keeps working. The download list and settings in
  %LOCALAPPDATA%\Herb's Download Manager stay, so a reinstall finds them. }
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Manifest: AnsiString;
  Wanted: String;
  Keys: array of String;
  I: Integer;
begin
  if CurUninstallStep <> usPostUninstall then Exit;
  if not LoadStringFromFile(ExpandConstant('{localappdata}\Herb''s Download Manager\native-host-chromium.json'), Manifest) then Exit;
  Wanted := ExpandConstant('{app}\hdm.exe');
  StringChangeEx(Wanted, '\', '\\', True);
  if Pos(Lowercase(Wanted), Lowercase(String(Manifest))) = 0 then Exit;
  Keys := ['Software\Google\Chrome', 'Software\Chromium', 'Software\Microsoft\Edge',
           'Software\BraveSoftware\Brave-Browser', 'Software\Mozilla'];
  for I := 0 to GetArrayLength(Keys) - 1 do
    RegDeleteKeyIncludingSubkeys(HKEY_CURRENT_USER, Keys[I] + '\NativeMessagingHosts\org.herbsdownloadmanager.hdm');
  { The extension from Edge Add-ons, which Edge installed because the app named it: Edge removes it too. }
  RegDeleteKeyIncludingSubkeys(HKEY_CURRENT_USER, 'Software\Microsoft\Edge\Extensions\claljmehooppmcegokpkiffgknfdknao');
end;
