; Inno Setup script for the desigma Windows installer.
; Built by scripts/package-windows-installer.sh (CI: .github/workflows/release.yml).
;
;   iscc /DAppVersion=0.1.0-alpha.9 /DNumericVersion=0.1.0.9 /DBinDir=<dir with desigma.exe> scripts\windows\desigma.iss
;
; Output: dist\desigma-<AppVersion>-x86_64-windows-setup.exe

#ifndef AppVersion
  #define AppVersion "0.0.0-dev"
#endif
#ifndef NumericVersion
  #define NumericVersion "0.0.0.0"
#endif
#ifndef BinDir
  #define BinDir "target\x86_64-pc-windows-msvc\release"
#endif

#define AppName "desigma"
#define AppExe "desigma.exe"

[Setup]
; Stable GUID: never change it, upgrades install over the previous version through it.
AppId={{6F3B8C52-2D41-4E7A-9B0C-D3516A1E7A94}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=deepfounder-ai
AppPublisherURL=https://github.com/deepfounder-ai/desigma
AppSupportURL=https://github.com/deepfounder-ai/desigma/issues
AppUpdatesURL=https://github.com/deepfounder-ai/desigma/releases
VersionInfoVersion={#NumericVersion}
VersionInfoProductVersion={#NumericVersion}
VersionInfoProductName={#AppName}
VersionInfoDescription={#AppName} setup
VersionInfoCompany=deepfounder-ai
; Per-user by default (no UAC prompt); the wizard offers "all users" via a dialog.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; Paths below are relative to the repository root.
SourceDir=..\..
OutputDir=dist
OutputBaseFilename=desigma-{#AppVersion}-x86_64-windows-setup
SetupIconFile=assets\icon\desigma.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
LicenseFile=LICENSE-MIT
Compression=lzma2/ultra
SolidCompression=yes
WizardStyle=modern
ChangesAssociations=yes
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "fileassoc"; Description: "Open .desigma files with {#AppName}"; GroupDescription: "File associations:"

[Files]
Source: "{#BinDir}\desigma.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinDir}\desigma-mcp.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinDir}\desigma-diff.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
Source: "LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion
Source: "README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "crates\desigma-fonts\assets\inter\OFL.txt"; DestDir: "{app}\licenses"; DestName: "Inter-OFL.txt"; Flags: ignoreversion
Source: "crates\desigma-app\assets\icons\lucide\LICENSE"; DestDir: "{app}\licenses"; DestName: "Lucide-LICENSE.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Registry]
; HKA is HKCU for a per-user install and HKLM for an all-users install.
Root: HKA; Subkey: "Software\Classes\.desigma"; ValueType: string; ValueName: ""; ValueData: "desigma.Document"; Flags: uninsdeletevalue; Tasks: fileassoc
Root: HKA; Subkey: "Software\Classes\.desigma\OpenWithProgids"; ValueType: string; ValueName: "desigma.Document"; ValueData: ""; Flags: uninsdeletevalue; Tasks: fileassoc
Root: HKA; Subkey: "Software\Classes\desigma.Document"; ValueType: string; ValueName: ""; ValueData: "desigma design"; Flags: uninsdeletekey; Tasks: fileassoc
Root: HKA; Subkey: "Software\Classes\desigma.Document\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExe},0"; Tasks: fileassoc
Root: HKA; Subkey: "Software\Classes\desigma.Document\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Tasks: fileassoc

; The desigma:// link scheme: clicking a session link opens the Join dialog.
Root: HKA; Subkey: "Software\Classes\desigma"; ValueType: string; ValueName: ""; ValueData: "URL:desigma session link"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\desigma"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\desigma\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExe},0"
Root: HKA; Subkey: "Software\Classes\desigma\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent
; In-app update ("Restart to update" runs this setup with /SILENT /RELAUNCH=1): postinstall entries do not run
; silently, so start the new version here instead. A plain silent install (/VERYSILENT /CURRENTUSER) does not.
Filename: "{app}\{#AppExe}"; Flags: nowait skipifnotsilent; Check: Relaunching

[Code]
// True when Setup was started with /RELAUNCH=1, which only the updater inside desigma passes.
function Relaunching: Boolean;
begin
  Result := ExpandConstant('{param:RELAUNCH|0}') = '1';
end;
