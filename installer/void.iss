; Збірка: cargo build --release --locked, потім ISCC installer\void.iss
; Результат: target\installer\Void-<версія>-setup.exe
#define AppVersion "0.1.0"

[Setup]
AppId={{6F0B7C1E-2D4A-4C5B-9E3A-C1A3D1E5F0A7}
AppName=Void
AppVersion={#AppVersion}
AppPublisher=ZUBRYTSKYI
DefaultDirName={autopf}\Void
DefaultGroupName=Void
DisableProgramGroupPage=yes
; Установка для поточного користувача, права адміністратора не потрібні.
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\target\installer
OutputBaseFilename=Void-{#AppVersion}-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayName=Void
UninstallDisplayIcon={app}\void.exe
CloseApplications=yes
CloseApplicationsFilter=void.exe
LicenseFile=..\LICENSE

[Languages]
Name: "ukrainian"; MessagesFile: "compiler:Languages\Ukrainian.isl"

[Tasks]
Name: "autostart"; Description: "Запускати Void разом з Windows"; Flags: unchecked

[Files]
Source: "..\target\release\void.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Void"; Filename: "{app}\void.exe"

[Registry]
; Те саме значення, що перемикає пункт «Запускати з Windows» у меню трею.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Void"; ValueData: """{app}\void.exe"""; Tasks: autostart; Flags: uninsdeletevalue

[UninstallRun]
; Повертає попередній status line користувача.
Filename: "{app}\void.exe"; Parameters: "disconnect-statusline"; Flags: runhidden waituntilterminated; RunOnceId: "DisconnectStatusLine"
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM void.exe"; Flags: runhidden skipifdoesntexist; RunOnceId: "StopVoid"

[Run]
; Підключає status line Claude Code, зберігаючи попередній. Нічого не треба налаштовувати вручну.
Filename: "{app}\void.exe"; Parameters: "connect-statusline"; Flags: runhidden waituntilterminated
Filename: "{app}\void.exe"; Description: "Запустити Void"; Flags: nowait postinstall skipifsilent
