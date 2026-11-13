; =============================================================================
; KetraCopy 1.2.1 - Inno Setup installer script
; =============================================================================
;
; Build:
;   ISCC packaging\windows\ketracopy.iss
;   (or open this file in the Inno Setup 6 IDE and press Compile)
; Output:
;   dist\windows\KetraCopy-1.2.1-setup.exe
;
; What it does:
;   - installs ketra.exe (CLI, required) and ketracopy-gui.exe (optional; the
;     [Files] entry is silently skipped when the GUI binary was not built)
;   - supports PER-MACHINE and PER-USER installs (user picks in the dialog,
;     or force with ISCC /D or: setup.exe /CURRENTUSER | /ALLUSERS)
;   - optional tasks: add {app} to PATH, register Explorer context menu
;     (KetraCopy: Copy / Move / Test) for files and directories
;   - Windows 7+ (MinVersion 6.1), x86 and x64
;   - context menu entries are written by the PowerShell helper scripts
;     (PS 2.0 compatible so they work on a stock Windows 7 box); those
;     helpers are duplicated in integrations\windows\ - keep both in sync
; =============================================================================

#define MyAppName "KetraCopy"
#define MyAppVersion "1.2.1"
#define MyAppPublisher "KetraCopy contributors"
#define MyAppURL "https://github.com/maniesh-NR/KetraCopy"

[Setup]
AppId={{B4E7D3C2-1A96-4F5E-8C7D-9F0A1B2C3D4E}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
; Allow BOTH per-machine and per-user installs; the user chooses in the dialog.
PrivilegesRequiredOverridesAllowed=dialog commandline
DefaultDirName={autopf}\KetraCopy
DefaultGroupName={#MyAppName}
UninstallDisplayName={#MyAppName} {#MyAppVersion}
UninstallDisplayIcon={app}\ketra.exe
; MIT license page shown before install (points at the repository LICENSE):
LicenseFile=..\..\LICENSE
; Windows 7 (NT 6.1) or newer:
MinVersion=6.1
; 64-bit installs land in "Program Files", 32-bit in "Program Files (x86)".
; (Inno 6.3+ prefers "x64compatible"; "x64" is still accepted there.)
ArchitecturesInstallIn64BitMode=x64
; We modify PATH in [Code]; this makes Windows refresh environment broadcast.
ChangesEnvironment=yes
; No SetupIconFile yet (no .ico asset in the repo) - add one when available.
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
DisableProgramGroupPage=yes
OutputDir=..\..\dist\windows
OutputBaseFilename=KetraCopy-{#MyAppVersion}-setup
SetupLogging=yes

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; \
    GroupDescription: "Shortcuts:"
Name: "path"; Description: "Add KetraCopy to &PATH (use 'ketra' in any console)"; \
    GroupDescription: "Integration:"; Flags: checkedonce
Name: "contextmenu"; Description: "Add Explorer context-menu actions (Copy / Move / Test with KetraCopy)"; \
    GroupDescription: "Integration:"; Flags: checkedonce

[Files]
; ketra.exe - the CLI is the required payload (build with: cargo build --release -p ketra-cli)
Source: "..\..\target\release\ketra.exe"; DestDir: "{app}"; Flags: ignoreversion
; ketracopy-gui.exe - optional; skipped when the GUI crate has not been built:
Source: "..\..\target\release\ketracopy-gui.exe"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
; PowerShell helpers for the Explorer context menu (see [Code] below):
Source: "context-menu.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "context-menu-install.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "context-menu-uninstall.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\KetraCopy"; Filename: "{app}\ketracopy-gui.exe"; Check: GuiExists
Name: "{group}\KetraCopy CLI"; Filename: "{app}\ketra.exe"
Name: "{autodesktop}\KetraCopy"; Filename: "{app}\ketracopy-gui.exe"; Tasks: desktopicon; Check: GuiExists

[Run]
Filename: "{app}\ketracopy-gui.exe"; Description: "&Launch KetraCopy"; \
    Flags: nowait postinstall skipifsilent; Check: GuiExists

[Code]
const
  WM_SETTINGCHANGE = $001A;
  SMTO_ABORTIFHUNG = $0002;
  HWND_BROADCAST   = $FFFF;

procedure SendMessageTimeout(hWnd: HWND; Msg: UINT; wParam: WPARAM; lParam: string;
  fuFlags: UINT; uTimeout: UINT; var lpdwResult: DWORD);
  external 'SendMessageTimeoutW@user32.dll stdcall';

procedure BroadcastEnvironmentChange();
var
  Res: DWORD;
begin
  SendMessageTimeout(HWND_BROADCAST, WM_SETTINGCHANGE, 0, 'Environment',
    SMTO_ABORTIFHUNG, 5000, Res);
end;

function GuiExists(): Boolean;
begin
  Result := FileExists(ExpandConstant('{app}\ketracopy-gui.exe'));
end;

// Add or remove {app} from PATH (machine PATH for per-machine installs,
// user PATH for per-user installs). Idempotent: never creates duplicates.
procedure UpdatePath(AddDir: Boolean);
var
  RootKey: Integer;
  SubKey: String;
  Dir: String;
  OldPath: String;
  NewPath: String;
  Rest: String;
  Part: String;
  P: Integer;
  First: Boolean;
begin
  Dir := ExpandConstant('{app}');
  if IsAdminInstallMode then
  begin
    RootKey := HKEY_LOCAL_MACHINE;
    SubKey := 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment';
  end
  else
  begin
    RootKey := HKEY_CURRENT_USER;
    SubKey := 'Environment';
  end;

  OldPath := '';
  RegQueryStringValue(RootKey, SubKey, 'Path', OldPath);

  // Rebuild PATH without {app} (strip, then optionally re-append at the end).
  NewPath := '';
  First := True;
  Rest := OldPath;
  while Length(Rest) > 0 do
  begin
    P := Pos(';', Rest);
    if P = 0 then
    begin
      Part := Rest;
      Rest := '';
    end
    else
    begin
      Part := Copy(Rest, 1, P - 1);
      Rest := Copy(Rest, P + 1, MaxInt);
    end;
    Part := Trim(Part);
    if (Part <> '') and (CompareText(Part, Dir) <> 0) then
    begin
      if First then
        NewPath := Part
      else
        NewPath := NewPath + ';' + Part;
      First := False;
    end;
  end;

  if AddDir then
  begin
    if NewPath = '' then
      NewPath := Dir
    else
      NewPath := NewPath + ';' + Dir;
  end;

  if NewPath <> OldPath then
  begin
    if NewPath = '' then
      RegDeleteValue(RootKey, SubKey, 'Path')
    else
      RegWriteExpandStringValue(RootKey, SubKey, 'Path', NewPath);
    BroadcastEnvironmentChange();
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  PSExe: String;
  Script: String;
  Args: String;
  Res: Integer;
begin
  if CurStep = ssPostInstall then
  begin
    // PATH task
    if WizardIsTaskSelected('path') then
      UpdatePath(True);

    // Context-menu task: run the PowerShell helper (it writes the registry
    // verbs; per-machine installs are elevated, so -Scope machine works).
    if WizardIsTaskSelected('contextmenu') then
    begin
      PSExe := ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe');
      Script := ExpandConstant('{app}\context-menu-install.ps1');
      if FileExists(PSExe) and FileExists(Script) then
      begin
        if IsAdminInstallMode then
          Args := Format('-NoProfile -ExecutionPolicy Bypass -File "%s" -Path "%s" -Scope machine',
            [Script, ExpandConstant('{app}')])
        else
          Args := Format('-NoProfile -ExecutionPolicy Bypass -File "%s" -Path "%s" -Scope user',
            [Script, ExpandConstant('{app}')]);
        Exec(PSExe, Args, '', SW_SHOW, ewWaitUntilTerminated, Res);
      end;
    end;
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  PSExe: String;
  Script: String;
  Args: String;
  Res: Integer;
begin
  if CurUninstallStep = usUninstall then
  begin
    // Files are still on disk at this point, so the helper can be run.
    PSExe := ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe');
    Script := ExpandConstant('{app}\context-menu-uninstall.ps1');

    UpdatePath(False);

    if FileExists(PSExe) and FileExists(Script) then
    begin
      if IsAdminInstallMode then
        Args := Format('-NoProfile -ExecutionPolicy Bypass -File "%s" -Scope machine', [Script])
      else
        Args := Format('-NoProfile -ExecutionPolicy Bypass -File "%s" -Scope user', [Script]);
      Exec(PSExe, Args, '', SW_SHOW, ewWaitUntilTerminated, Res);
    end;
  end;
end;
