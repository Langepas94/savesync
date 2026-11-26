#ifndef AppVersion
#define AppVersion "1.0.0"
#endif
#ifndef SourceDir
#define SourceDir "..\..\target\release"
#endif
#ifndef OutputDirectory
#define OutputDirectory "..\..\dist"
#endif
[Setup]
AppId={{3552AC8F-432F-41EC-A63B-38A2E2F767AD}
AppName=SaveSync
AppVersion={#AppVersion}
AppPublisher=SaveSync Project
DefaultDirName={localappdata}\Programs\SaveSync
DefaultGroupName=SaveSync
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.10240
OutputDir={#OutputDirectory}
#ifdef ReleaseInstaller
OutputBaseFilename=savesync-setup
#else
OutputBaseFilename=savesync-setup-dev
#endif
#ifdef Signed
SignTool=savesync
SignedUninstaller=yes
#else
SignedUninstaller=no
#endif
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=no
RestartApplications=no
UninstallDisplayIcon={app}\savesync-app.exe
[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"
[Tasks]
Name: "autostart"; Description: "Запускать SaveSync при входе в Windows"; Flags: checkedonce
Name: "desktopicon"; Description: "Создать ярлык на рабочем столе"; Flags: unchecked
[Files]
Source: "{#SourceDir}\savesync-daemon.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\savesync-app.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\docs\OAUTH.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "..\..\docs\LAN.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "..\..\docs\QUICKSTART.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "..\..\docs\INSTALL-AND-TEST.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "..\..\docs\USER-GUIDE.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "..\..\docs\TRY-1.0.0.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "..\..\docs\STEAMOS-DELIVERY.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "..\..\docs\manifests-schema.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "..\..\crates\core\manifests\LICENSE"; DestDir: "{app}\licenses"; DestName: "manifests.txt"; Flags: ignoreversion
Source: "..\..\crates\core\manifests\LUDUSAVI-LICENSE"; DestDir: "{app}\licenses"; DestName: "ludusavi.txt"; Flags: ignoreversion
Source: "..\..\crates\core\manifests\LUDUSAVI-SOURCE.md"; DestDir: "{app}\licenses"; Flags: ignoreversion
[Icons]
Name: "{group}\SaveSync"; Filename: "{app}\savesync-app.exe"
Name: "{userdesktop}\SaveSync"; Filename: "{app}\savesync-app.exe"; Tasks: desktopicon
[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "SaveSync"; ValueData: """{app}\savesync-app.exe"" --tray"; Tasks: autostart; Flags: uninsdeletevalue
[Run]
Filename: "{app}\savesync-app.exe"; Description: "Открыть SaveSync"; Flags: nowait postinstall skipifsilent
Filename: "{app}\savesync-app.exe"; Parameters: "--tray"; Flags: nowait; Check: StartAfterSilentUpdate
[Code]
function StopProgram(const FileName, Parameters: String): Boolean;
var ExitCode: Integer;
begin
  Result := True;
  if FileExists(FileName) then
    Result := Exec(FileName, Parameters, '', SW_HIDE, ewWaitUntilTerminated, ExitCode) and (ExitCode = 0);
end;
function StopInstalled: Boolean;
begin
  Result := StopProgram(ExpandConstant('{app}\savesync-daemon.exe'), 'stop');
  if Result then Result := StopProgram(ExpandConstant('{app}\savesync-app.exe'), '--quit');
end;
function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  if not StopInstalled then
    Result := 'SaveSync did not stop safely. Wait for synchronization to finish and retry; no files were replaced.';
end;
function InitializeUninstall: Boolean;
begin
  Result := StopInstalled;
  if not Result then MsgBox('SaveSync did not stop safely. Uninstall cancelled.', mbError, MB_OK);
end;
function StartAfterSilentUpdate: Boolean;
begin
  Result := WizardSilent and (ExpandConstant('{param:STARTAFTERINSTALL|0}') = '1');
end;
