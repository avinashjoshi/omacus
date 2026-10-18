; The Windows installer, omacus-windows-x86_64-setup.exe, built by the release
; workflow with Inno Setup:
;
;   iscc /DVersion=0.1.17 /DExe=target\release\omacus.exe installer\omacus.iss
;
; Installs for the current user only (no admin prompt) into
; %LOCALAPPDATA%\Programs\Omacus, adds Omacus to Start (and, if ticked, the
; desktop), puts `omacus` on the PATH for the command line, and opens the app at
; the end. Omacus updates itself in place there; running a newer setup over it
; works too. Settings > Apps uninstalls it (sheets are kept).

#ifndef Version
  #error Pass /DVersion=x.y.z
#endif
#ifndef Exe
  #define Exe "..\target\release\omacus.exe"
#endif

[Setup]
; Never change: Windows matches upgrades and the uninstaller by this.
AppId={{25905D4B-E397-4A30-ABC6-79693A59BE81}
AppName=Omacus
AppVersion={#Version}
AppVerName=Omacus {#Version}
AppPublisher=Omacus
AppPublisherURL=https://omacus.com
AppSupportURL=https://github.com/avinashjoshi/omacus/issues
AppUpdatesURL=https://omacus.com/download/
PrivilegesRequired=lowest
DefaultDirName={autopf}\Omacus
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableReadyPage=yes
DisableWelcomePage=yes
UninstallDisplayIcon={app}\omacus.exe
UninstallDisplayName=Omacus
SetupIconFile=..\assets\icon.ico
WizardStyle=modern
; Art from installer/make-art.sh, at each display scale.
WizardImageFile=wizard-100.png,wizard-125.png,wizard-150.png,wizard-175.png,wizard-200.png,wizard-225.png,wizard-250.png
WizardSmallImageFile=small-100.png,small-125.png,small-150.png,small-175.png,small-200.png,small-225.png,small-250.png
WizardImageStretch=no
WizardSizePercent=100
Compression=lzma2/max
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
; Close a running Omacus to replace it, and open it again afterwards.
CloseApplications=yes
RestartApplications=no
ChangesEnvironment=yes
OutputDir=..\target\installer
OutputBaseFilename=omacus-windows-x86_64-setup

[Tasks]
Name: desktopicon; Description: "Add Omacus to the desktop"; Flags: unchecked

[Files]
Source: "{#Exe}"; DestDir: "{app}"; DestName: "omacus.exe"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Omacus"; Filename: "{app}\omacus.exe"; Comment: "Notepad calculator"
Name: "{autodesktop}\Omacus"; Filename: "{app}\omacus.exe"; Tasks: desktopicon

[Registry]
; `omacus 2+2` from a terminal. Added only if missing; removed on uninstall (below).
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; \
  ValueData: "{olddata};{app}"; Check: NotOnPath(ExpandConstant('{app}'))
; Omacus registers omacus:// share links itself when it starts; tidy that up.
Root: HKCU; Subkey: "Software\Classes\omacus"; Flags: uninsdeletekey dontcreatekey

[Run]
Filename: "{app}\omacus.exe"; Description: "Open Omacus"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; Left by updating in place (the old and staged copies of omacus.exe).
Type: files; Name: "{app}\omacus.exe.old"
Type: files; Name: "{app}\omacus.exe.new"

[Code]
function PathValue(): String;
begin
  if not RegQueryStringValue(HKCU, 'Environment', 'Path', Result) then
    Result := '';
end;

function NotOnPath(Dir: String): Boolean;
begin
  Result := Pos(';' + Uppercase(Dir) + ';', ';' + Uppercase(PathValue()) + ';') = 0;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Path, Dir: String;
  At: Integer;
begin
  if CurUninstallStep <> usPostUninstall then
    exit;
  Path := ';' + PathValue() + ';';
  Dir := ';' + ExpandConstant('{app}') + ';';
  At := Pos(Uppercase(Dir), Uppercase(Path));
  if At = 0 then
    exit;
  Delete(Path, At, Length(Dir) - 1);
  Path := Copy(Path, 2, Length(Path) - 2);
  RegWriteExpandStringValue(HKCU, 'Environment', 'Path', Path);
end;
