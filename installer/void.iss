; Збірка: cargo build --release --locked, потім ISCC installer\void.iss
; Результат: target\installer\Void-<версія>-setup.exe
#define AppVersion "0.1.0"

[Setup]
AppId={{386CCC0E-4621-4CC6-AFE3-8BBE85DC196A}
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

; The first language is the fallback: Inno Setup picks the entry matching the Windows UI
; language, and uses the first one listed when none matches. English therefore comes first.
[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "ukrainian"; MessagesFile: "compiler:Languages\Ukrainian.isl"
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"

[CustomMessages]
english.AutoStart=Start Void with Windows
ukrainian.AutoStart=Запускати Void разом з Windows
russian.AutoStart=Запускать Void вместе с Windows
english.LaunchVoid=Launch Void
ukrainian.LaunchVoid=Запустити Void
russian.LaunchVoid=Запустить Void

[Tasks]
Name: "autostart"; Description: "{cm:AutoStart}"; Flags: unchecked

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
Filename: "{app}\void.exe"; Description: "{cm:LaunchVoid}"; Flags: nowait postinstall skipifsilent
