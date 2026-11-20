; Webox installer (Inno Setup 6). Per-user by default: installs into %LOCALAPPDATA%\Programs\Webox without admin rights, so
; the in-app updater can replace webox.exe later. Settings, sign-in and plugins stay in %APPDATA%\WebOx (the app's own folder)
; and are only removed when the user says so at uninstall.
;
;   iscc /DAppVersion=0.2.0 /DSourceDir=dist\webox-0.2.0-windows-x64 /DIconFile=dist\webox.ico /ODist packaging\windows\webox.iss
;   -> dist\Webox-Setup-0.2.0-x64.exe

#ifndef AppVersion
  #error Pass the version: /DAppVersion=x.y.z
#endif
#ifndef SourceDir
  #error Pass the folder with webox.exe and the licence files: /DSourceDir=...
#endif
#ifndef IconFile
  #error Pass the icon made by make-ico.ps1: /DIconFile=...
#endif

[Setup]
; Never change AppId: Windows keys the "Apps" entry by it, and src/platform/updater/registry.rs reads that entry.
AppId={{6F3A2D1C-8B4E-4C7A-9E15-2B7D0C4F8A61}
AppName=Webox
AppVersion={#AppVersion}
AppVerName=Webox {#AppVersion}
AppPublisher=PianoNic
AppPublisherURL=https://github.com/PianoNic/Webox
AppSupportURL=https://github.com/PianoNic/Webox/issues
AppUpdatesURL=https://github.com/PianoNic/Webox/releases
VersionInfoVersion={#AppVersion}
DefaultDirName={autopf}\Webox
DefaultGroupName=Webox
DisableProgramGroupPage=yes
; Per-user; an all-users install is possible from the command line (/ALLUSERS), but then the updater can't write the folder.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=commandline
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
; The app holds this mutex while it runs; setup and uninstall ask to close it first.
AppMutex=WeboxAppMutex
CloseApplications=yes
RestartApplications=no
SetupIconFile={#IconFile}
UninstallDisplayIcon={app}\webox.ico
UninstallDisplayName=Webox
LicenseFile={#SourceDir}\LICENSE
OutputBaseFilename=Webox-Setup-{#AppVersion}-x64
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ShowLanguageDialog=auto

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "de"; MessagesFile: "compiler:Languages\German.isl"

[CustomMessages]
en.DesktopIcon=Create a &desktop shortcut
de.DesktopIcon=&Desktopverknüpfung erstellen
en.Autostart=Start Webox when I sign in to Windows
de.Autostart=Webox beim Anmelden bei Windows starten
en.Extra=Additional options:
de.Extra=Weitere Optionen:
en.RemoveData=Also delete your Webox settings, sign-in and installed plugins (%APPDATA%\WebOx)?%n%nChoose No to keep them for a later install.
de.RemoveData=Auch Ihre Webox-Einstellungen, die Anmeldung und installierte Plugins löschen (%APPDATA%\WebOx)?%n%nWählen Sie Nein, um sie für eine spätere Installation zu behalten.
en.Launch=Start Webox
de.Launch=Webox starten

[Tasks]
Name: "desktopicon"; Description: "{cm:DesktopIcon}"; GroupDescription: "{cm:Extra}"; Flags: unchecked
Name: "autostart"; Description: "{cm:Autostart}"; GroupDescription: "{cm:Extra}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\webox.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#IconFile}"; DestDir: "{app}"; DestName: "webox.ico"; Flags: ignoreversion
Source: "{#SourceDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\THIRD_PARTY_NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Webox"; Filename: "{app}\webox.exe"; IconFilename: "{app}\webox.ico"; AppUserModelID: "Webox"
Name: "{autodesktop}\Webox"; Filename: "{app}\webox.exe"; IconFilename: "{app}\webox.ico"; Tasks: desktopicon

[Registry]
; The same Run value and arguments the app writes for "Webox zusammen mit meinem Computer starten" (src/platform/system.rs),
; so the installer and the setting switch one entry.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "WebOx"; ValueData: """{app}\webox.exe"" /minimized /autostartedWithWindows=true"; Tasks: autostart
; Removed at uninstall whether the installer or the app set it.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "WebOx"; Flags: uninsdeletevalue

[Run]
Filename: "{app}\webox.exe"; Description: "{cm:Launch}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; What an in-app update leaves next to the program. A plugins\ folder with the user's own files is never touched.
Type: files; Name: "{app}\webox.exe.old"
Type: files; Name: "{app}\webox.exe.new"
Type: files; Name: "{app}\webox.exe.write-test"

[Code]
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Data: String;
begin
  if CurUninstallStep = usPostUninstall then
  begin
    Data := ExpandConstant('{userappdata}\WebOx');
    if DirExists(Data) and (SuppressibleMsgBox(CustomMessage('RemoveData'), mbConfirmation, MB_YESNO or MB_DEFBUTTON2, IDNO) = IDYES) then
      DelTree(Data, True, True, True);
  end;
end;
