; Audido Inno Setup Script
; This script creates an installer for the Audido terminal-based audio player

#define MyAppName "Audido"
#define MyAppVersion "0.1.0"
#define MyAppPublisher "Audido"
#define MyAppURL "https://github.com/haidarptrw/audido"
#define MyAppExeName "audido-tui.exe"
#define MyAppDescription "A terminal-based audio player with queue management"

[Setup]
; NOTE: The value of AppId uniquely identifies this application. Do not use the same AppId value in installers for other applications.
; (To generate a new GUID, click Tools | Generate GUID inside the IDE.)
AppId={{E5F5F5F5-5F5F-5F5F-5F5F-5F5F5F5F5F5F}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
AllowNoIcons=yes
LicenseFile=
InfoBeforeFile=
InfoAfterFile=
; Uncomment the following line to run in non administrative install mode (install for current user only.)
;PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir=..\target\release
OutputBaseFilename=audido-setup-{#MyAppVersion}
SetupIconFile=
Compression=lzma
SolidCompression=yes
WizardStyle=modern
ArchitecturesInstallIn64BitMode=x64compatible
ChangesAssociations=yes
ChangesEnvironment=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "addtopath"; Description: "Add to PATH environment variable"; GroupDescription: "System Integration:"; Flags: unchecked
Name: "fileassoc"; Description: "Register file associations for audio formats (MP3, FLAC, WAV, OGG, M4A)"; GroupDescription: "System Integration:"; Flags: unchecked

[Files]
Source: "..\target\release\{#MyAppExeName}"; DestDir: "{app}\bin"; Flags: ignoreversion
; NOTE: Don't use "Flags: ignoreversion" on any shared system files

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\bin\{#MyAppExeName}"
Name: "{group}\{cm:UninstallProgram,{#MyAppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\bin\{#MyAppExeName}"; Tasks: desktopicon

[Registry]
; Add to PATH
Root: HKLM; Subkey: "SYSTEM\CurrentControlSet\Control\Session Manager\Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}\bin"; Tasks: addtopath; Check: NeedsAddPath('{app}\bin')

; File Associations - MP3
Root: HKCR; Subkey: ".mp3"; ValueType: string; ValueName: ""; ValueData: "Audido.mp3"; Flags: uninsdeletevalue; Tasks: fileassoc
Root: HKCR; Subkey: "Audido.mp3"; ValueType: string; ValueName: ""; ValueData: "MP3 Audio File"; Flags: uninsdeletekey; Tasks: fileassoc
Root: HKCR; Subkey: "Audido.mp3\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\bin\{#MyAppExeName},0"; Tasks: fileassoc
Root: HKCR; Subkey: "Audido.mp3\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\bin\{#MyAppExeName}"" ""%1"""; Tasks: fileassoc

; File Associations - FLAC
Root: HKCR; Subkey: ".flac"; ValueType: string; ValueName: ""; ValueData: "Audido.flac"; Flags: uninsdeletevalue; Tasks: fileassoc
Root: HKCR; Subkey: "Audido.flac"; ValueType: string; ValueName: ""; ValueData: "FLAC Audio File"; Flags: uninsdeletekey; Tasks: fileassoc
Root: HKCR; Subkey: "Audido.flac\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\bin\{#MyAppExeName},0"; Tasks: fileassoc
Root: HKCR; Subkey: "Audido.flac\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\bin\{#MyAppExeName}"" ""%1"""; Tasks: fileassoc

; File Associations - WAV
Root: HKCR; Subkey: ".wav"; ValueType: string; ValueName: ""; ValueData: "Audido.wav"; Flags: uninsdeletevalue; Tasks: fileassoc
Root: HKCR; Subkey: "Audido.wav"; ValueType: string; ValueName: ""; ValueData: "WAV Audio File"; Flags: uninsdeletekey; Tasks: fileassoc
Root: HKCR; Subkey: "Audido.wav\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\bin\{#MyAppExeName},0"; Tasks: fileassoc
Root: HKCR; Subkey: "Audido.wav\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\bin\{#MyAppExeName}"" ""%1"""; Tasks: fileassoc

; File Associations - OGG
Root: HKCR; Subkey: ".ogg"; ValueType: string; ValueName: ""; ValueData: "Audido.ogg"; Flags: uninsdeletevalue; Tasks: fileassoc
Root: HKCR; Subkey: "Audido.ogg"; ValueType: string; ValueName: ""; ValueData: "OGG Audio File"; Flags: uninsdeletekey; Tasks: fileassoc
Root: HKCR; Subkey: "Audido.ogg\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\bin\{#MyAppExeName},0"; Tasks: fileassoc
Root: HKCR; Subkey: "Audido.ogg\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\bin\{#MyAppExeName}"" ""%1"""; Tasks: fileassoc

; File Associations - M4A
Root: HKCR; Subkey: ".m4a"; ValueType: string; ValueName: ""; ValueData: "Audido.m4a"; Flags: uninsdeletevalue; Tasks: fileassoc
Root: HKCR; Subkey: "Audido.m4a"; ValueType: string; ValueName: ""; ValueData: "M4A Audio File"; Flags: uninsdeletekey; Tasks: fileassoc
Root: HKCR; Subkey: "Audido.m4a\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\bin\{#MyAppExeName},0"; Tasks: fileassoc
Root: HKCR; Subkey: "Audido.m4a\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\bin\{#MyAppExeName}"" ""%1"""; Tasks: fileassoc

[Code]
function NeedsAddPath(Param: string): boolean;
var
  OrigPath: string;
begin
  if not RegQueryStringValue(HKEY_LOCAL_MACHINE,
    'SYSTEM\CurrentControlSet\Control\Session Manager\Environment',
    'Path', OrigPath)
  then begin
    Result := True;
    exit;
  end;
  // look for the path with leading and trailing semicolon
  // Pos() returns 0 if not found
  Result := Pos(';' + Param + ';', ';' + OrigPath + ';') = 0;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
  begin
    // Refresh environment variables
    if WizardIsTaskSelected('addtopath') then
    begin
      // Notify system of environment variable changes
      RegWriteStringValue(HKEY_LOCAL_MACHINE,
        'SYSTEM\CurrentControlSet\Control\Session Manager\Environment',
        'PATHEXT', '.COM;.EXE;.BAT;.CMD;.VBS;.VBE;.JS;.JSE;.WSF;.WSH;.MSC');
    end;
  end;
end;

[Run]
Filename: "{app}\bin\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent unchecked
