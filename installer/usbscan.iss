; UsbScan installer (Inno Setup 6)
; Build with: iscc /DMyAppVersion=0.1.3 installer\usbscan.iss
; or set the USBSCAN_VERSION environment variable.

#define MyAppName "UsbScan"
#ifndef MyAppVersion
  #define MyAppVersion GetEnv('USBSCAN_VERSION')
#endif
#define MyAppPublisher "Kaiyuan"
#define MyAppURL "https://github.com/Kaiyuan/usbscan"
#define MyAppExeName "usbscan.exe"

[Setup]
AppId={{8B4F6C2E-5B7A-4F0D-9C61-2A7E4D8F3B10}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}/releases
DefaultDirName={localappdata}\Programs\usbscan
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir=..\dist
OutputBaseFilename=usbscan-{#MyAppVersion}-setup
SetupIconFile=..\assets\icons\usbscan_box.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=yes
RestartApplications=no
AllowNoIcons=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "chinesesimplified"; MessagesFile: "languages\ChineseSimplified.isl"
Name: "chinesetraditional"; MessagesFile: "languages\ChineseTraditional.isl"
Name: "german"; MessagesFile: "compiler:Languages\German.isl"
Name: "french"; MessagesFile: "compiler:Languages\French.isl"
Name: "italian"; MessagesFile: "compiler:Languages\Italian.isl"
Name: "spanish"; MessagesFile: "compiler:Languages\Spanish.isl"
Name: "japanese"; MessagesFile: "compiler:Languages\Japanese.isl"
Name: "korean"; MessagesFile: "compiler:Languages\Korean.isl"

[CustomMessages]
english.VCRedistPrompt=UsbScan needs the Microsoft Visual C++ 2015-2022 Redistributable (x64), which is not installed on this computer.%n%nDownload it from Microsoft and install it now? This requires administrator permission. If you skip this step, UsbScan may not start.
chinesesimplified.VCRedistPrompt=UsbScan 需要 Microsoft Visual C++ 2015–2022 运行库（x64），但当前系统尚未安装。%n%n是否现在从 Microsoft 下载并安装？此操作需要管理员权限。如果跳过，UsbScan 可能无法启动。
chinesetraditional.VCRedistPrompt=UsbScan 需要 Microsoft Visual C++ 2015–2022 執行階段（x64），但目前系統尚未安裝。%n%n是否要立即從 Microsoft 下載並安裝？此操作需要系統管理員權限。若略過，UsbScan 可能無法啟動。
german.VCRedistPrompt=UsbScan benötigt das Microsoft Visual C++ 2015–2022 Redistributable (x64), das auf diesem Computer nicht installiert ist.%n%nJetzt von Microsoft herunterladen und installieren? Dafür sind Administratorrechte erforderlich. Wenn du diesen Schritt überspringst, startet UsbScan möglicherweise nicht.
french.VCRedistPrompt=UsbScan nécessite le package redistribuable Microsoft Visual C++ 2015–2022 (x64), qui n'est pas installé sur cet ordinateur.%n%nLe télécharger depuis Microsoft et l'installer maintenant ? Cette opération nécessite des droits d'administrateur. Si vous ignorez cette étape, UsbScan risque de ne pas démarrer.
italian.VCRedistPrompt=UsbScan richiede il Microsoft Visual C++ 2015–2022 Redistributable (x64), che non è installato su questo computer.%n%nScaricarlo da Microsoft e installarlo ora? L'operazione richiede i privilegi di amministratore. Se salti questo passaggio, UsbScan potrebbe non avviarsi.
spanish.VCRedistPrompt=UsbScan necesita el paquete redistribuible de Microsoft Visual C++ 2015–2022 (x64), que no está instalado en este equipo.%n%n¿Descargarlo desde Microsoft e instalarlo ahora? Esta operación requiere permisos de administrador. Si omites este paso, es posible que UsbScan no se inicie.
japanese.VCRedistPrompt=UsbScan を実行するには Microsoft Visual C++ 2015–2022 再頒布可能パッケージ（x64）が必要ですが、このコンピューターにはインストールされていません。%n%nMicrosoft から今すぐダウンロードしてインストールしますか？管理者権限が必要です。この手順をスキップすると、UsbScan が起動しない可能性があります。
korean.VCRedistPrompt=UsbScan을 실행하려면 Microsoft Visual C++ 2015–2022 재배포 가능 패키지(x64)가 필요하지만 이 컴퓨터에 설치되어 있지 않습니다.%n%n지금 Microsoft에서 다운로드하여 설치하시겠습니까? 관리자 권한이 필요합니다. 이 단계를 건너뛰면 UsbScan이 시작되지 않을 수 있습니다.

english.VCRedistDownloadFailed=Could not download the Visual C++ runtime. Check your network connection and try again, or install it manually from:%nhttps://aka.ms/vs/17/release/vc_redist.x64.exe
chinesesimplified.VCRedistDownloadFailed=无法下载 Visual C++ 运行库。请检查网络连接后重试，或手动安装：%nhttps://aka.ms/vs/17/release/vc_redist.x64.exe
chinesetraditional.VCRedistDownloadFailed=無法下載 Visual C++ 執行階段。請檢查網路連線後重試，或手動安裝：%nhttps://aka.ms/vs/17/release/vc_redist.x64.exe
german.VCRedistDownloadFailed=Die Visual C++-Laufzeit konnte nicht heruntergeladen werden. Prüfe deine Netzwerkverbindung und versuche es erneut, oder installiere sie manuell von:%nhttps://aka.ms/vs/17/release/vc_redist.x64.exe
french.VCRedistDownloadFailed=Impossible de télécharger le runtime Visual C++. Vérifiez votre connexion réseau et réessayez, ou installez-le manuellement depuis :%nhttps://aka.ms/vs/17/release/vc_redist.x64.exe
italian.VCRedistDownloadFailed=Impossibile scaricare il runtime Visual C++. Controlla la connessione di rete e riprova, oppure installalo manualmente da:%nhttps://aka.ms/vs/17/release/vc_redist.x64.exe
spanish.VCRedistDownloadFailed=No se pudo descargar el runtime de Visual C++. Comprueba la conexión de red e inténtalo de nuevo, o instálalo manualmente desde:%nhttps://aka.ms/vs/17/release/vc_redist.x64.exe
japanese.VCRedistDownloadFailed=Visual C++ ランタイムをダウンロードできませんでした。ネットワーク接続を確認して再試行するか、次の URL から手動でインストールしてください:%nhttps://aka.ms/vs/17/release/vc_redist.x64.exe
korean.VCRedistDownloadFailed=Visual C++ 런타임을 다운로드하지 못했습니다. 네트워크 연결을 확인한 후 다시 시도하거나 다음 주소에서 직접 설치하세요:%nhttps://aka.ms/vs/17/release/vc_redist.x64.exe

english.VCRedistInstallFailed=The Visual C++ runtime could not be installed. UsbScan may not start. You can install it manually from:%nhttps://aka.ms/vs/17/release/vc_redist.x64.exe
chinesesimplified.VCRedistInstallFailed=Visual C++ 运行库安装失败，UsbScan 可能无法启动。你可以手动安装：%nhttps://aka.ms/vs/17/release/vc_redist.x64.exe
chinesetraditional.VCRedistInstallFailed=Visual C++ 執行階段安裝失敗，UsbScan 可能無法啟動。你可以手動安裝：%nhttps://aka.ms/vs/17/release/vc_redist.x64.exe
german.VCRedistInstallFailed=Die Visual C++-Laufzeit konnte nicht installiert werden. UsbScan startet möglicherweise nicht. Du kannst sie manuell installieren von:%nhttps://aka.ms/vs/17/release/vc_redist.x64.exe
french.VCRedistInstallFailed=Le runtime Visual C++ n'a pas pu être installé. UsbScan risque de ne pas démarrer. Vous pouvez l'installer manuellement depuis :%nhttps://aka.ms/vs/17/release/vc_redist.x64.exe
italian.VCRedistInstallFailed=Impossibile installare il runtime Visual C++. UsbScan potrebbe non avviarsi. Puoi installarlo manualmente da:%nhttps://aka.ms/vs/17/release/vc_redist.x64.exe
spanish.VCRedistInstallFailed=No se pudo instalar el runtime de Visual C++. Es posible que UsbScan no se inicie. Puedes instalarlo manualmente desde:%nhttps://aka.ms/vs/17/release/vc_redist.x64.exe
japanese.VCRedistInstallFailed=Visual C++ ランタイムをインストールできませんでした。UsbScan が起動しない可能性があります。次の URL から手動でインストールしてください:%nhttps://aka.ms/vs/17/release/vc_redist.x64.exe
korean.VCRedistInstallFailed=Visual C++ 런타임을 설치하지 못했습니다. UsbScan이 시작되지 않을 수 있습니다. 다음 주소에서 직접 설치하세요:%nhttps://aka.ms/vs/17/release/vc_redist.x64.exe

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "..\target\release\usbscan.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\THIRD_PARTY.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\assets\icons\TABLER-ICONS-LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{group}\{cm:UninstallProgram,{#MyAppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; 确保卸载后不残留安装目录、旧版开始菜单快捷方式以及运行时配置/tessdata。
Type: filesandordirs; Name: "{app}"
Type: filesandordirs; Name: "{group}"
Type: files; Name: "{userprograms}\usbscan.lnk"
Type: filesandordirs; Name: "{userappdata}\usbscan"
Type: filesandordirs; Name: "{localappdata}\usbscan"

[Code]
// UsbScan links against the MSVC runtime, so the Microsoft Visual C++
// 2015-2022 Redistributable (x64) must be present. Inno Setup 6.1 or newer
// provides the built-in download support used below.
const
  VCRedistUrl = 'https://aka.ms/vs/17/release/vc_redist.x64.exe';
  VCRedistFile = 'vc_redist.x64.exe';

var
  DownloadPage: TDownloadWizardPage;

function VCRedistInstalled(): Boolean;
var
  Installed: Cardinal;
  RuntimeVersion: String;
  RootKey: Integer;
begin
  RootKey := HKLM;
  if IsWin64 then
    RootKey := HKLM64;

  Result := RegQueryDWordValue(RootKey,
    'SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64',
    'Installed', Installed) and (Installed = 1);

  if not Result then
    Result := RegQueryStringValue(RootKey,
      'SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64',
      'Version', RuntimeVersion) and (RuntimeVersion <> '');
end;

procedure InitializeWizard;
begin
  DownloadPage := CreateDownloadPage(SetupMessage(msgWizardPreparing),
    SetupMessage(msgPreparingDesc), nil);
end;

function DownloadVCRedist(): Boolean;
begin
  Result := False;
  DownloadPage.Clear;
  DownloadPage.Add(VCRedistUrl, VCRedistFile, '');
  DownloadPage.Show;
  try
    try
      DownloadPage.Download;
      Result := True;
    except
      if DownloadPage.AbortedByUser then
        Log('Visual C++ runtime download aborted by the user.')
      else
        MsgBox(ExpandConstant('{cm:VCRedistDownloadFailed}') + #13#10#13#10 +
          GetExceptionMessage, mbError, MB_OK);
    end;
  finally
    DownloadPage.Hide;
  end;
end;

function RunVCRedist(): Boolean;
var
  RedistPath: String;
  ResultCode: Integer;
  Launched: Boolean;
begin
  Result := False;
  RedistPath := ExpandConstant('{tmp}\') + VCRedistFile;
  if not FileExists(RedistPath) then
  begin
    Log('Visual C++ runtime installer not found: ' + RedistPath);
    Exit;
  end;

  ResultCode := -1;
  Launched := False;
  try
    if IsAdmin then
      Launched := Exec(RedistPath, '/install /quiet /norestart', '', SW_HIDE,
        ewWaitUntilTerminated, ResultCode)
    else
      Launched := ShellExec('runas', RedistPath, '/install /quiet /norestart', '',
        SW_SHOWNORMAL, ewWaitUntilTerminated, ResultCode);
  except
    Log('Unable to launch the Visual C++ runtime installer: ' +
      GetExceptionMessage);
  end;

  Log(Format('Visual C++ runtime installer exit code: %d', [ResultCode]));

  // 0 = success, 1638 = a newer version is already installed,
  // 1641/3010 = success but a reboot is required.
  Result := Launched and ((ResultCode = 0) or (ResultCode = 1638) or
    (ResultCode = 1641) or (ResultCode = 3010));

  if not Result then
    Result := VCRedistInstalled();
end;

function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
  if CurPageID <> wpReady then
    Exit;

  if VCRedistInstalled() then
    Exit;

  if WizardSilent then
  begin
    Log('Silent install: skipping the Visual C++ runtime prompt.');
    Exit;
  end;

  if MsgBox(ExpandConstant('{cm:VCRedistPrompt}'), mbConfirmation, MB_YESNO) <>
    IDYES then
  begin
    Log('User declined to install the Visual C++ runtime.');
    Exit;
  end;

  if not DownloadVCRedist() then
    Exit;

  if RunVCRedist() then
    Log('Visual C++ runtime installed successfully.')
  else
    MsgBox(ExpandConstant('{cm:VCRedistInstallFailed}'), mbError, MB_OK);
end;
