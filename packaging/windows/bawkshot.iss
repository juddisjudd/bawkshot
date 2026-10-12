#ifndef AppVersion
  #error Pass the version, for example: iscc /DAppVersion=0.1.0 bawkshot.iss
#endif
#define RunKey "Software\Microsoft\Windows\CurrentVersion\Run"

[Setup]
AppId={{6F2B8C51-3D0E-4A9B-9C47-B2A5E1D7F083}
AppName=bawkshot
AppVersion={#AppVersion}
AppPublisher=juddisjudd
AppPublisherURL=https://github.com/juddisjudd/bawkshot
AppSupportURL=https://github.com/juddisjudd/bawkshot/issues
AppUpdatesURL=https://github.com/juddisjudd/bawkshot/releases
AppMutex=bawkshot-single-instance
CloseApplications=no
DefaultDirName={autopf}\bawkshot
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
UsePreviousTasks=no
SourceDir=..\..
OutputDir=dist
OutputBaseFilename=bawkshot-v{#AppVersion}-windows-x64-setup
SetupIconFile=assets\brand\bawkshot.ico
UninstallDisplayIcon={app}\bawkshot.exe
UninstallDisplayName=bawkshot
VersionInfoVersion={#AppVersion}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern dynamic

[Tasks]
Name: startup; Description: "Start bawkshot when I sign in to Windows"; Flags: unchecked

[Files]
Source: "target\release\bawkshot.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\bawkshot"; Filename: "{app}\bawkshot.exe"

[Registry]
Root: HKCU; Subkey: "{#RunKey}"; ValueType: string; ValueName: "bawkshot"; ValueData: """{app}\bawkshot.exe"""; Tasks: startup

[Run]
Filename: "{app}\bawkshot.exe"; Description: "Start bawkshot"; Flags: nowait postinstall skipifsilent

[Messages]
SetupAppRunningError=bawkshot is running.%n%nRight-click its tray icon and choose "quit bawkshot". Then click OK to continue, or Cancel to exit.
UninstallAppRunningError=bawkshot is running.%n%nRight-click its tray icon and choose "quit bawkshot". Then click OK to continue, or Cancel to exit.

[Code]
// bawkshot's own setting writes this entry too, so it goes only when it starts the copy being removed.
procedure CurUninstallStepChanged(Step: TUninstallStep);
var
  Command: String;
begin
  if (Step = usUninstall)
    and RegQueryStringValue(HKCU, '{#RunKey}', 'bawkshot', Command)
    and (CompareText(Command, '"' + ExpandConstant('{app}\bawkshot.exe') + '"') = 0) then
    RegDeleteValue(HKCU, '{#RunKey}', 'bawkshot');
end;
