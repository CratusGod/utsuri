; Inno Setup script — per-user install, no admin rights needed.
#define AppVersion GetEnv("DS_VERSION")
#if AppVersion == ""
  #define AppVersion "1.1.0"
#endif

[Setup]
AppId={{9910EB1A-E5FE-43F9-B79D-1CAF25C3AF14}
AppName=Utsuri
AppVersion={#AppVersion}
AppPublisher=Utsuri
DefaultDirName={localappdata}\Programs\Utsuri
DefaultGroupName=Utsuri
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir=..\dist
OutputBaseFilename=Utsuri-Setup
SetupIconFile=..\utsuri\assets\utsuri.ico
UninstallDisplayIcon={app}\Utsuri.exe
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no

[Tasks]
Name: "autostart"; Description: "Start Utsuri when I sign in to Windows"; Flags: checkedonce
Name: "contextmenu"; Description: "Add ""Utsuri"" to the right-click menu for files"; Flags: checkedonce
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
Source: "..\dist\Utsuri\*"; DestDir: "{app}"; Flags: recursesubdirs ignoreversion

[Icons]
Name: "{autoprograms}\Utsuri"; Filename: "{app}\Utsuri.exe"
Name: "{autodesktop}\Utsuri"; Filename: "{app}\Utsuri.exe"; Tasks: desktopicon
Name: "{usersendto}\Utsuri"; Filename: "{app}\Utsuri.exe"

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Utsuri"; ValueData: """{app}\Utsuri.exe"" --quiet"; Tasks: autostart; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\*\shell\Utsuri"; ValueType: string; ValueName: "MUIVerb"; ValueData: "Utsuri…"; Tasks: contextmenu; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\*\shell\Utsuri"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\Utsuri.exe"; Tasks: contextmenu
Root: HKCU; Subkey: "Software\Classes\*\shell\Utsuri"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"; Tasks: contextmenu
Root: HKCU; Subkey: "Software\Classes\*\shell\Utsuri\command"; ValueType: string; ValueName: ""; ValueData: """{app}\Utsuri.exe"" ""%1"""; Tasks: contextmenu
Root: HKCU; Subkey: "Software\Utsuri"; Flags: uninsdeletekey

[Run]
Filename: "{app}\Utsuri.exe"; Description: "Launch Utsuri now"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{cmd}"; Parameters: "/C taskkill /F /IM Utsuri.exe"; Flags: runhidden; RunOnceId: "KillUtsuri"
