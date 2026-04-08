; rgitui - Inno Setup 6 Installer Script
; https://github.com/noahbclarkson/rgitui

#define MyAppName "rgitui"
#ifndef MyAppVersion
  #define MyAppVersion "0.0.0-dev"
#endif
#define MyAppPublisher "rgitui contributors"
#define MyAppURL "https://github.com/noahbclarkson/rgitui"
#define MyAppExeName "rgitui.exe"

[Setup]
AppId={{B3A7F2E1-8C4D-4F6A-9E2B-1D5C8A3F7E90}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}/issues
AppUpdatesURL={#MyAppURL}/releases
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
AllowNoIcons=yes
LicenseFile=..\..\..\..\LICENSE
OutputDir=..\..\..\..\Output
OutputBaseFilename=rgitui-{#MyAppVersion}-x86_64-windows-setup
SetupIconFile=..\..\..\..\assets\icons\app-icon.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
Compression=lzma2/ultra
SolidCompression=yes
WizardStyle=modern
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
MinVersion=10.0
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "addtopath"; Description: "Add rgitui to PATH"; GroupDescription: "Environment:"

[Files]
Source: "..\..\..\..\target\release\rgitui.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{group}\Uninstall {#MyAppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent

[Code]
const
  EnvironmentKey = 'Environment';

procedure AddToPath(Dir: String);
var
  CurrentPath: String;
begin
  if not RegQueryStringValue(HKEY_CURRENT_USER, EnvironmentKey, 'Path', CurrentPath) then
    CurrentPath := '';
  if Pos(Uppercase(Dir), Uppercase(CurrentPath)) > 0 then
    Exit;
  if CurrentPath <> '' then
    CurrentPath := CurrentPath + ';';
  CurrentPath := CurrentPath + Dir;
  RegWriteStringValue(HKEY_CURRENT_USER, EnvironmentKey, 'Path', CurrentPath);
end;

procedure RemoveFromPath(Dir: String);
var
  CurrentPath: String;
  DirUpper: String;
  PathUpper: String;
  StartPos: Integer;
  EndPos: Integer;
begin
  if not RegQueryStringValue(HKEY_CURRENT_USER, EnvironmentKey, 'Path', CurrentPath) then
    Exit;
  DirUpper := Uppercase(Dir);
  PathUpper := Uppercase(CurrentPath);
  StartPos := Pos(DirUpper, PathUpper);
  if StartPos = 0 then
    Exit;
  EndPos := StartPos + Length(Dir);
  if (EndPos <= Length(CurrentPath)) and (CurrentPath[EndPos] = ';') then
    EndPos := EndPos + 1
  else if (StartPos > 1) and (CurrentPath[StartPos - 1] = ';') then
    StartPos := StartPos - 1;
  Delete(CurrentPath, StartPos, EndPos - StartPos);
  RegWriteStringValue(HKEY_CURRENT_USER, EnvironmentKey, 'Path', CurrentPath);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
  begin
    if IsTaskSelected('addtopath') then
      AddToPath(ExpandConstant('{app}'));
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    RemoveFromPath(ExpandConstant('{app}'));
end;
