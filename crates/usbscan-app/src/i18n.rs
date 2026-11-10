// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 应用内多语言文本。
//!
//! 支持简体中文、繁體中文、English、Deutsch、Français、Italiano、Español、
//! 日本語、한국어。语言在启动时从配置或系统区域读取，也可以在“关于”窗口中
//! 即时切换。

use std::sync::atomic::{AtomicU8, Ordering};

/// 支持的语言；顺序与翻译表一致。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Lang {
    ZhHans = 0,
    ZhHant = 1,
    En = 2,
    De = 3,
    Fr = 4,
    It = 5,
    Es = 6,
    Ja = 7,
    Ko = 8,
}

impl Lang {
    pub const ALL: [Self; 9] = [
        Self::ZhHans,
        Self::ZhHant,
        Self::En,
        Self::De,
        Self::Fr,
        Self::It,
        Self::Es,
        Self::Ja,
        Self::Ko,
    ];

    /// 配置文件与界面切换使用的稳定标识。
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::ZhHans => "zh-Hans",
            Self::ZhHant => "zh-Hant",
            Self::En => "en",
            Self::De => "de",
            Self::Fr => "fr",
            Self::It => "it",
            Self::Es => "es",
            Self::Ja => "ja",
            Self::Ko => "ko",
        }
    }

    /// 语言选择器中显示的原生名称。
    #[must_use]
    pub const fn native_name(self) -> &'static str {
        match self {
            Self::ZhHans => "简体中文",
            Self::ZhHant => "繁體中文",
            Self::En => "English",
            Self::De => "Deutsch",
            Self::Fr => "Français",
            Self::It => "Italiano",
            Self::Es => "Español",
            Self::Ja => "日本語",
            Self::Ko => "한국어",
        }
    }

    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }

    #[must_use]
    pub fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or(Self::En)
    }

    /// 解析配置中保存的语言标识；未知值返回 `None`。
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|lang| lang.code().eq_ignore_ascii_case(code))
    }

    /// 解析 BCP 47 / POSIX 区域标识，例如 `zh-TW`、`de_AT`、`en-US`。
    #[must_use]
    pub fn from_tag(tag: &str) -> Self {
        let tag = tag.to_ascii_lowercase().replace('_', "-");
        if tag.starts_with("zh") {
            if tag.contains("hant")
                || tag.contains("-tw")
                || tag.contains("-hk")
                || tag.contains("-mo")
            {
                return Self::ZhHant;
            }
            return Self::ZhHans;
        }
        for lang in Self::ALL {
            if !matches!(lang, Self::ZhHans | Self::ZhHant) && tag.starts_with(lang.code()) {
                return lang;
            }
        }
        Self::En
    }

    /// 读取系统区域；无法识别时回退到英文。
    #[must_use]
    pub fn from_system() -> Self {
        sys_locale::get_locale().map_or(Self::En, |locale| Self::from_tag(&locale))
    }
}

// 默认英文，启动时由 run() 根据配置或系统区域覆盖。
static CURRENT: AtomicU8 = AtomicU8::new(Lang::En as u8);

#[must_use]
pub fn current() -> Lang {
    Lang::from_index(usize::from(CURRENT.load(Ordering::Relaxed)))
}

pub fn set_language(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

/// 翻译表。每行依次为：键、[简体中文, 繁體中文, English, Deutsch, Français,
/// Italiano, Español, 日本語, 한국어]。
///
/// 模板中的 `{}` 会由 [`t_args`] 按顺序替换。
static TABLE: &[(&str, [&str; 9])] = &[
    // 顶栏与侧栏
    ("ui.device", ["设备", "裝置", "Device", "Gerät", "Appareil", "Dispositivo", "Dispositivo", "デバイス", "장치"]),
    ("ui.resolution", ["分辨率", "解析度", "Resolution", "Auflösung", "Résolution", "Risoluzione", "Resolución", "解像度", "해상도"]),
    ("ui.fps", ["FPS", "FPS", "FPS", "FPS", "FPS", "FPS", "FPS", "FPS", "FPS"]),
    ("ui.refresh", ["刷新", "重新整理", "Refresh", "Aktualisieren", "Actualiser", "Aggiorna", "Actualizar", "更新", "새로 고침"]),
    ("ui.recordStart", ["录像", "錄影", "Record", "Aufnehmen", "Enregistrer", "Registra", "Grabar", "録画", "녹화"]),
    ("ui.recordStop", ["停止", "停止", "Stop", "Stopp", "Arrêter", "Interrompi", "Detener", "停止", "중지"]),
    ("ui.capture", ["拍照", "拍照", "Capture", "Auslösen", "Capturer", "Acquisisci", "Capturar", "撮影", "촬영"]),
    ("ui.rotate", ["旋转", "旋轉", "Rotate", "Drehen", "Pivoter", "Ruota", "Girar", "回転", "회전"]),
    ("ui.pages", ["页面", "頁面", "Pages", "Seiten", "Pages", "Pagine", "Páginas", "ページ", "페이지"]),
    ("ui.waiting", [
        "等待摄像头\n插上任一 USB 摄像头即可开始",
        "等待攝影機\n插上任一 USB 攝影機即可開始",
        "Waiting for a camera\nPlug in any USB camera to begin",
        "Kamera wird erwartet\nSchließe eine USB-Kamera an, um zu beginnen",
        "En attente d'une caméra\nBranchez n'importe quelle caméra USB pour commencer",
        "In attesa di una webcam\nCollega qualsiasi webcam USB per iniziare",
        "Esperando una cámara\nConecta cualquier cámara USB para empezar",
        "カメラを待機中\nUSB カメラを接続すると開始できます",
        "카메라 대기 중\nUSB 카메라를 연결하면 시작할 수 있습니다",
    ]),
    ("ui.scanSettings", ["扫描设置", "掃描設定", "Scan settings", "Scan-Einstellungen", "Réglages de numérisation", "Impostazioni di scansione", "Ajustes de escaneo", "スキャン設定", "스캔 설정"]),
    ("ui.outputDir", ["输出目录", "輸出目錄", "Output folder", "Ausgabeordner", "Dossier de sortie", "Cartella di output", "Carpeta de salida", "出力先", "출력 폴더"]),
    ("ui.browse", ["浏览…", "瀏覽…", "Browse…", "Durchsuchen…", "Parcourir…", "Sfoglia…", "Examinar…", "参照…", "찾아보기…"]),
    ("ui.fileTemplate", ["文件名模板", "檔名範本", "File name template", "Dateinamen-Vorlage", "Modèle de nom de fichier", "Modello nome file", "Plantilla de nombre", "ファイル名テンプレート", "파일 이름 템플릿"]),
    ("ui.folderTemplate", ["子目录模板", "子目錄範本", "Subfolder template", "Unterordner-Vorlage", "Modèle de sous-dossier", "Modello sottocartella", "Plantilla de subcarpeta", "サブフォルダーテンプレート", "하위 폴더 템플릿"]),
    ("ui.applySettings", ["应用设置", "套用設定", "Apply settings", "Einstellungen anwenden", "Appliquer", "Applica impostazioni", "Aplicar ajustes", "設定を適用", "설정 적용"]),
    ("ui.importImages", ["导入图片", "匯入圖片", "Import images", "Bilder importieren", "Importer des images", "Importa immagini", "Importar imágenes", "画像を読み込む", "이미지 가져오기"]),
    ("ui.exportPdf", ["导出 PDF", "匯出 PDF", "Export PDF", "PDF exportieren", "Exporter en PDF", "Esporta PDF", "Exportar PDF", "PDF を書き出す", "PDF 내보내기"]),
    ("ui.enhance", ["图像增强", "影像增強", "Image enhancement", "Bildverbesserung", "Amélioration d'image", "Miglioramento immagine", "Mejora de imagen", "画像補正", "이미지 보정"]),
    ("ui.autoCrop", ["自动裁边（实时显示裁剪框）", "自動裁邊（即時顯示裁切框）", "Auto crop (live frame)", "Auto-Zuschnitt (Live-Rahmen)", "Recadrage auto (cadre en direct)", "Ritaglio auto (cornice live)", "Recorte automático (marco en vivo)", "自動トリミング（枠をリアルタイム表示）", "자동 자르기(실시간 프레임)"]),
    ("ui.autoDeskew", ["自动纠偏", "自動校正", "Auto deskew", "Automatisch ausrichten", "Redressement auto", "Raddrizza auto", "Enderezar automáticamente", "自動傾き補正", "자동 기울기 보정"]),
    ("ui.perspective", ["透视校正", "透視校正", "Perspective correction", "Perspektivkorrektur", "Correction de perspective", "Correzione prospettica", "Corrección de perspectiva", "台形補正", "원근 보정"]),
    ("ui.removeShadow", ["去阴影", "去除陰影", "Remove shadows", "Schatten entfernen", "Supprimer les ombres", "Rimuovi ombre", "Quitar sombras", "影を除去", "그림자 제거"]),
    ("ui.whiteBackground", ["白底净化", "白底淨化", "Clean white background", "Weißhintergrund", "Fond blanc", "Sfondo bianco", "Fondo blanco", "白背景補正", "흰 배경 정리"]),
    ("ui.strength", ["强度", "強度", "Strength", "Stärke", "Intensité", "Intensità", "Intensidad", "強さ", "강도"]),
    ("ui.renameTitle", ["重命名页面", "重新命名頁面", "Rename page", "Seite umbenennen", "Renommer la page", "Rinomina pagina", "Cambiar nombre", "ページ名を変更", "페이지 이름 변경"]),
    ("ui.cancel", ["取消", "取消", "Cancel", "Abbrechen", "Annuler", "Annulla", "Cancelar", "キャンセル", "취소"]),
    ("ui.save", ["保存", "儲存", "Save", "Speichern", "Enregistrer", "Salva", "Guardar", "保存", "저장"]),
    ("ui.about", [
        "把任意 USB 摄像头变成文档扫描仪。拍摄、自动裁边、透视校正、离线 OCR 与 PDF 归档，全程本地完成。",
        "把任意 USB 攝影機變成文件掃描器。拍攝、自動裁邊、透視校正、離線 OCR 與 PDF 歸檔，全程在本機完成。",
        "Turn any USB camera into a document scanner. Capture, auto-crop, perspective correction, offline OCR and PDF archiving, all locally.",
        "Jede USB-Kamera wird zum Dokumentenscanner. Aufnehmen, automatisch zuschneiden, Perspektive korrigieren, Offline-OCR und PDF-Archivierung – alles lokal.",
        "Transformez n'importe quelle caméra USB en scanner de documents. Capture, recadrage automatique, correction de perspective, OCR hors ligne et archivage PDF, entièrement en local.",
        "Trasforma qualsiasi webcam USB in uno scanner per documenti. Acquisizione, ritaglio automatico, correzione prospettica, OCR offline e archiviazione PDF, tutto in locale.",
        "Convierte cualquier cámara USB en un escáner de documentos. Captura, recorte automático, corrección de perspectiva, OCR sin conexión y archivado en PDF, todo en local.",
        "どんな USB カメラもドキュメントスキャナーに。撮影、自動トリミング、台形補正、オフライン OCR、PDF 保存まで、すべて端末内で完結します。",
        "어떤 USB 카메라든 문서 스캐너로. 촬영, 자동 자르기, 원근 보정, 오프라인 OCR, PDF 보관까지 모두 기기에서 처리합니다.",
    ]),
    ("ui.autoCheckUpdates", ["自动检查更新", "自動檢查更新", "Check for updates automatically", "Automatisch nach Updates suchen", "Rechercher les mises à jour automatiquement", "Controlla automaticamente gli aggiornamenti", "Buscar actualizaciones automáticamente", "自動的に更新を確認", "자동으로 업데이트 확인"]),
    ("ui.checkUpdates", ["检查更新", "檢查更新", "Check for updates", "Nach Updates suchen", "Rechercher des mises à jour", "Controlla aggiornamenti", "Buscar actualizaciones", "更新を確認", "업데이트 확인"]),
    ("ui.checkingUpdates", ["正在检查…", "正在檢查…", "Checking…", "Prüfe…", "Vérification…", "Controllo…", "Buscando…", "確認中…", "확인 중…"]),
    ("ui.updateHint", ["勾选后每次启动会访问 GitHub Releases 检查新版本。", "勾選後每次啟動會造訪 GitHub Releases 檢查新版本。", "When enabled, startup checks GitHub Releases for a new version.", "Aktiviert prüft der Start GitHub Releases auf eine neue Version.", "Une fois activé, le démarrage vérifie les nouvelles versions sur GitHub Releases.", "Se attivo, all'avvio viene cercata una nuova versione su GitHub Releases.", "Si está activado, al iniciar se buscan nuevas versiones en GitHub Releases.", "有効にすると起動時に GitHub Releases で新しいバージョンを確認します。", "활성화하면 시작 시 GitHub Releases에서 새 버전을 확인합니다."]),
    ("ui.viewOnGithub", ["在 GitHub 查看", "在 GitHub 查看", "View on GitHub", "Auf GitHub ansehen", "Voir sur GitHub", "Vedi su GitHub", "Ver en GitHub", "GitHub で見る", "GitHub에서 보기"]),
    ("ui.close", ["关闭", "關閉", "Close", "Schließen", "Fermer", "Chiudi", "Cerrar", "閉じる", "닫기"]),
    ("ui.language", ["语言", "語言", "Language", "Sprache", "Langue", "Lingua", "Idioma", "言語", "언어"]),
    ("ui.auto", ["自动", "自動", "Auto", "Automatisch", "Auto", "Auto", "Auto", "自動", "자동"]),
    ("ui.pagePrefix", ["第 ", "第 ", "Page ", "Seite ", "Page ", "Pagina ", "Página ", "", ""]),
    ("ui.pageSuffix", [" 页", " 頁", "", "", "", "", "", " ページ", " 페이지"]),
    ("ui.pageCount", ["{} 页", "{} 頁", "{} pages", "{} Seiten", "{} pages", "{} pagine", "{} páginas", "{} ページ", "{} 페이지"]),
    ("ui.pageCountOne", ["{} 页", "{} 頁", "{} page", "{} Seite", "{} page", "{} pagina", "{} página", "{} ページ", "{} 페이지"]),
    // 状态栏与提示
    ("status.ready", ["就绪", "就緒", "Ready", "Bereit", "Prêt", "Pronto", "Listo", "準備完了", "준비됨"]),
    ("status.detecting", ["正在检测摄像头…", "正在偵測攝影機…", "Detecting camera…", "Kamera wird gesucht…", "Recherche de la caméra…", "Ricerca della webcam…", "Detectando cámara…", "カメラを検出しています…", "카메라 감지 중…"]),
    ("status.mfInitFailed", ["Media Foundation 初始化失败：{}", "Media Foundation 初始化失敗：{}", "Media Foundation initialization failed: {}", "Media Foundation konnte nicht initialisiert werden: {}", "Échec de l'initialisation de Media Foundation : {}", "Inizializzazione di Media Foundation non riuscita: {}", "Error al inicializar Media Foundation: {}", "Media Foundation の初期化に失敗しました: {}", "Media Foundation 초기화 실패: {}"]),
    ("status.enumerateFailed", ["枚举摄像头失败：{}", "列舉攝影機失敗：{}", "Failed to enumerate cameras: {}", "Kameras konnten nicht aufgezählt werden: {}", "Échec de l'énumération des caméras : {}", "Enumerazione delle webcam non riuscita: {}", "Error al enumerar las cámaras: {}", "カメラの列挙に失敗しました: {}", "카메라 열거 실패: {}"]),
    ("status.waitingCamera", ["等待摄像头：{}", "等待攝影機：{}", "Waiting for camera: {}", "Warte auf Kamera: {}", "En attente de la caméra : {}", "In attesa della webcam: {}", "Esperando la cámara: {}", "カメラを待機中: {}", "카메라 대기 중: {}"]),
    ("status.savedDevice", ["已保存的设备", "已儲存的裝置", "Saved device", "Gespeichertes Gerät", "Appareil enregistré", "Dispositivo salvato", "Dispositivo guardado", "保存済みのデバイス", "저장된 장치"]),
    ("status.noCamera", ["未检测到摄像头，请插入 USB 摄像头", "未偵測到攝影機，請插入 USB 攝影機", "No camera detected. Plug in a USB camera.", "Keine Kamera gefunden. Bitte eine USB-Kamera anschließen.", "Aucune caméra détectée. Branchez une caméra USB.", "Nessuna webcam rilevata. Collega una webcam USB.", "No se detectó ninguna cámara. Conecta una cámara USB.", "カメラが見つかりません。USB カメラを接続してください。", "카메라를 찾을 수 없습니다. USB 카메라를 연결하세요."]),
    ("status.openCameraFailed", ["无法打开摄像头：{}", "無法開啟攝影機：{}", "Failed to open camera: {}", "Kamera konnte nicht geöffnet werden: {}", "Impossible d'ouvrir la caméra : {}", "Impossibile aprire la webcam: {}", "No se pudo abrir la cámara: {}", "カメラを開けません: {}", "카메라를 열 수 없습니다: {}"]),
    ("status.connected", ["已连接 {}  {}×{}", "已連接 {}  {}×{}", "Connected {}  {}×{}", "Verbunden: {}  {}×{}", "Connecté : {}  {}×{}", "Connesso: {}  {}×{}", "Conectado: {}  {}×{}", "接続中: {}  {}×{}", "연결됨: {}  {}×{}"]),
    ("status.cameraError", ["摄像头错误：{}", "攝影機錯誤：{}", "Camera error: {}", "Kamera-Fehler: {}", "Erreur de la caméra : {}", "Errore webcam: {}", "Error de la cámara: {}", "カメラエラー: {}", "카메라 오류: {}"]),
    ("status.noFrameCapture", ["尚未取得画面，请稍候再拍", "尚未取得畫面，請稍候再拍", "No frame yet. Please wait before capturing.", "Noch kein Bild. Bitte vor der Aufnahme warten.", "Pas encore d'image. Patientez avant de capturer.", "Nessun fotogramma. Attendi prima di acquisire.", "Aún no hay imagen. Espera antes de capturar.", "映像がまだありません。少し待ってから撮影してください。", "아직 화면이 없습니다. 잠시 후 촬영하세요."]),
    ("toast.captured", ["已拍照", "已拍照", "Captured", "Aufgenommen", "Capturé", "Acquisito", "Capturado", "撮影しました", "촬영됨"]),
    ("toast.rotated", ["已旋转 {}°", "已旋轉 {}°", "Rotated {}°", "Um {}° gedreht", "Pivoté de {}°", "Ruotato di {}°", "Girado {}°", "{}° 回転しました", "{}° 회전됨"]),
    ("status.createDirFailed", ["创建目录失败：{}", "建立目錄失敗：{}", "Failed to create folder: {}", "Ordner konnte nicht erstellt werden: {}", "Impossible de créer le dossier : {}", "Impossibile creare la cartella: {}", "No se pudo crear la carpeta: {}", "フォルダーを作成できません: {}", "폴더 생성 실패: {}"]),
    ("status.saveImageFailed", ["保存图片失败：{}", "儲存圖片失敗：{}", "Failed to save image: {}", "Bild konnte nicht gespeichert werden: {}", "Impossible d'enregistrer l'image : {}", "Impossibile salvare l'immagine: {}", "No se pudo guardar la imagen: {}", "画像を保存できません: {}", "이미지 저장 실패: {}"]),
    ("status.savedPage", ["已保存：{}（{}×{}）", "已儲存：{}（{}×{}）", "Saved: {} ({}×{})", "Gespeichert: {} ({}×{})", "Enregistré : {} ({}×{})", "Salvato: {} ({}×{})", "Guardado: {} ({}×{})", "保存しました: {}（{}×{}）", "저장됨: {} ({}×{})"]),
    ("status.newVersion", ["发现新版本 v{} · 点击更新", "發現新版本 v{} · 點擊更新", "New version v{} available · Click to update", "Neue Version v{} verfügbar · Zum Aktualisieren klicken", "Nouvelle version v{} disponible · Cliquez pour mettre à jour", "Nuova versione v{} disponibile · Clicca per aggiornare", "Nueva versión v{} disponible · Haz clic para actualizar", "新しいバージョン v{} があります · クリックして更新", "새 버전 v{} 사용 가능 · 클릭하여 업데이트"]),
    ("status.installingUpdate", ["正在安装更新…", "正在安裝更新…", "Installing update…", "Update wird installiert…", "Installation de la mise à jour…", "Installazione dell'aggiornamento…", "Instalando actualización…", "更新をインストールしています…", "업데이트 설치 중…"]),
    ("status.updateDownloadFailed", ["更新下载失败：{}", "更新下載失敗：{}", "Update download failed: {}", "Update-Download fehlgeschlagen: {}", "Échec du téléchargement de la mise à jour : {}", "Download dell'aggiornamento non riuscito: {}", "Error al descargar la actualización: {}", "更新のダウンロードに失敗しました: {}", "업데이트 다운로드 실패: {}"]),
    ("status.updateFailedRetry", ["更新失败，点击重试", "更新失敗，點擊重試", "Update failed. Click to retry.", "Update fehlgeschlagen. Zum Wiederholen klicken.", "Échec de la mise à jour. Cliquez pour réessayer.", "Aggiornamento non riuscito. Clicca per riprovare.", "Error de actualización. Haz clic para reintentar.", "更新に失敗しました。クリックして再試行。", "업데이트 실패. 클릭하여 다시 시도."]),
    ("status.downloadingUpdate", ["正在下载更新…", "正在下載更新…", "Downloading update…", "Update wird heruntergeladen…", "Téléchargement de la mise à jour…", "Download dell'aggiornamento…", "Descargando actualización…", "更新をダウンロードしています…", "업데이트 다운로드 중…"]),
    ("status.upToDate", ["已是最新版本", "已是最新版本", "You're up to date", "Sie verwenden die neueste Version", "Vous êtes à jour", "Hai già l'ultima versione", "Estás al día", "最新バージョンです", "최신 버전입니다"]),
    ("status.updateCheckFailed", ["检查更新失败：{}", "檢查更新失敗：{}", "Update check failed: {}", "Update-Prüfung fehlgeschlagen: {}", "Échec de la vérification des mises à jour : {}", "Controllo aggiornamenti non riuscito: {}", "Error al buscar actualizaciones: {}", "更新の確認に失敗しました: {}", "업데이트 확인 실패: {}"]),
    ("status.switchingCamera", ["正在切换到 {}…", "正在切換到 {}…", "Switching to {}…", "Wechsle zu {}…", "Passage à {}…", "Passaggio a {}…", "Cambiando a {}…", "{} に切り替えています…", "{} (으)로 전환 중…"]),
    ("status.resolutionAuto", ["分辨率：自动", "解析度：自動", "Resolution: Auto", "Auflösung: Automatisch", "Résolution : Auto", "Risoluzione: Auto", "Resolución: Auto", "解像度: 自動", "해상도: 자동"]),
    ("status.resolution", ["分辨率：{} × {}", "解析度：{} × {}", "Resolution: {} × {}", "Auflösung: {} × {}", "Résolution : {} × {}", "Risoluzione: {} × {}", "Resolución: {} × {}", "解像度: {} × {}", "해상도: {} × {}"]),
    ("status.recordFps", ["录像帧率：{} fps", "錄影幀率：{} fps", "Recording frame rate: {} fps", "Aufnahme-Framerate: {} fps", "Fréquence d'enregistrement : {} fps", "Frame rate di registrazione: {} fps", "Frecuencia de grabación: {} fps", "録画フレームレート: {} fps", "녹화 프레임률: {} fps"]),
    ("status.recordingSaved", ["录像已保存：{}", "錄影已儲存：{}", "Recording saved: {}", "Aufnahme gespeichert: {}", "Enregistrement sauvegardé : {}", "Registrazione salvata: {}", "Grabación guardada: {}", "録画を保存しました: {}", "녹화 저장됨: {}"]),
    ("toast.recordingSaved", ["录像已保存", "錄影已儲存", "Recording saved", "Aufnahme gespeichert", "Enregistrement sauvegardé", "Registrazione salvata", "Grabación guardada", "録画を保存しました", "녹화 저장됨"]),
    ("status.recordingSaveFailed", ["录像保存失败：{}", "錄影儲存失敗：{}", "Failed to save recording: {}", "Aufnahme konnte nicht gespeichert werden: {}", "Impossible d'enregistrer la vidéo : {}", "Impossibile salvare la registrazione: {}", "No se pudo guardar la grabación: {}", "録画を保存できません: {}", "녹화 저장 실패: {}"]),
    ("status.noFrameRecord", ["尚未取得画面，无法开始录像", "尚未取得畫面，無法開始錄影", "No frame yet. Cannot start recording.", "Noch kein Bild. Aufnahme kann nicht gestartet werden.", "Pas encore d'image. Impossible de démarrer l'enregistrement.", "Nessun fotogramma. Impossibile avviare la registrazione.", "Aún no hay imagen. No se puede iniciar la grabación.", "映像がまだありません。録画を開始できません。", "아직 화면이 없습니다. 녹화를 시작할 수 없습니다."]),
    ("dir.recordings", ["录像", "錄影", "Recordings", "Aufnahmen", "Enregistrements", "Registrazioni", "Grabaciones", "録画", "녹화"]),
    ("file.scan", ["扫描", "掃描", "Scan", "Scan", "Scan", "Scansione", "Escaneo", "スキャン", "스캔"]),
    ("file.import", ["导入", "匯入", "Import", "Import", "Import", "Importa", "Importar", "読み込み", "가져오기"]),
    ("file.recording", ["录像", "錄影", "Recording", "Aufnahme", "Enregistrement", "Registrazione", "Grabación", "録画", "녹화"]),
    ("status.createRecordDirFailed", ["创建录像目录失败：{}", "建立錄影目錄失敗：{}", "Failed to create recordings folder: {}", "Aufnahmeordner konnte nicht erstellt werden: {}", "Impossible de créer le dossier d'enregistrement : {}", "Impossibile creare la cartella delle registrazioni: {}", "No se pudo crear la carpeta de grabaciones: {}", "録画フォルダーを作成できません: {}", "녹화 폴더 생성 실패: {}"]),
    ("status.recordingNow", ["录像中… {}×{} @ {} fps", "錄影中… {}×{} @ {} fps", "Recording… {}×{} @ {} fps", "Aufnahme… {}×{} @ {} fps", "Enregistrement… {}×{} @ {} fps", "Registrazione… {}×{} @ {} fps", "Grabando… {}×{} @ {} fps", "録画中… {}×{} @ {} fps", "녹화 중… {}×{} @ {} fps"]),
    ("toast.recordingStarted", ["开始录像", "開始錄影", "Recording started", "Aufnahme gestartet", "Enregistrement démarré", "Registrazione avviata", "Grabación iniciada", "録画を開始しました", "녹화 시작됨"]),
    ("status.loadImageFailed", ["读取图片失败：{}", "讀取圖片失敗：{}", "Failed to read image: {}", "Bild konnte nicht gelesen werden: {}", "Impossible de lire l'image : {}", "Impossibile leggere l'immagine: {}", "No se pudo leer la imagen: {}", "画像を読み込めません: {}", "이미지 읽기 실패: {}"]),
    ("status.noBoundary", ["未检测到可信的文档边界，已跳过", "未偵測到可信的文件邊界，已略過", "No reliable document boundary found; skipped.", "Keine zuverlässige Dokumentgrenze gefunden; übersprungen.", "Aucune limite de document fiable détectée ; ignoré.", "Nessun bordo documento affidabile; ignorato.", "No se detectó un borde fiable; omitido.", "信頼できる書類の境界が見つからないためスキップしました。", "신뢰할 수 있는 문서 경계를 찾지 못해 건너뛰었습니다."]),
    ("status.autoProcessFailed", ["自动处理失败：{}", "自動處理失敗：{}", "Automatic processing failed: {}", "Automatische Verarbeitung fehlgeschlagen: {}", "Échec du traitement automatique : {}", "Elaborazione automatica non riuscita: {}", "Error en el procesamiento automático: {}", "自動処理に失敗しました: {}", "자동 처리 실패: {}"]),
    ("status.noCorrectionNeeded", ["无需校正", "無需校正", "No correction needed", "Keine Korrektur nötig", "Aucune correction nécessaire", "Nessuna correzione necessaria", "No se necesita corrección", "補正は不要です", "보정 필요 없음"]),
    ("status.writeBackFailed", ["写回失败：{}", "寫回失敗：{}", "Failed to write back: {}", "Zurückschreiben fehlgeschlagen: {}", "Échec de l'écriture : {}", "Scrittura non riuscita: {}", "Error al escribir: {}", "書き戻しに失敗しました: {}", "쓰기 실패: {}"]),
    ("status.autoCropped", ["已自动裁边并纠正畸变", "已自動裁邊並校正變形", "Auto-cropped and corrected distortion", "Automatisch zugeschnitten und verzerrungskorrigiert", "Recadré et distorsion corrigée automatiquement", "Ritagliato e distorsione corretta automaticamente", "Recortado y distorsión corregida automáticamente", "自動でトリミングし歪みを補正しました", "자동으로 자르고 왜곡을 보정했습니다"]),
    ("status.deskewed", ["已自动纠偏", "已自動校正", "Auto-deskewed", "Automatisch ausgerichtet", "Redressé automatiquement", "Raddrizzato automaticamente", "Enderezado automáticamente", "自動で傾きを補正しました", "자동으로 기울기를 보정했습니다"]),
    ("status.invalidFileName", ["文件名无效", "檔名無效", "Invalid file name", "Ungültiger Dateiname", "Nom de fichier invalide", "Nome file non valido", "Nombre de archivo no válido", "ファイル名が無効です", "파일 이름이 잘못되었습니다"]),
    ("status.fileExists", ["同名文件已存在", "同名檔案已存在", "A file with that name already exists", "Eine Datei mit diesem Namen existiert bereits", "Un fichier du même nom existe déjà", "Esiste già un file con questo nome", "Ya existe un archivo con ese nombre", "同じ名前のファイルが既に存在します", "같은 이름의 파일이 이미 있습니다"]),
    ("status.fileMissing", ["图片文件不存在或已被移动", "圖片檔案不存在或已被移動", "The image file is missing or has been moved", "Die Bilddatei fehlt oder wurde verschoben", "Le fichier image est introuvable ou a été déplacé", "Il file immagine non esiste o è stato spostato", "El archivo de imagen no existe o se ha movido", "画像ファイルが見つからないか、移動されています", "이미지 파일이 없거나 이동되었습니다"]),
    ("status.renameFailed", ["重命名失败：{}", "重新命名失敗：{}", "Rename failed: {}", "Umbenennen fehlgeschlagen: {}", "Échec du renommage : {}", "Rinomina non riuscita: {}", "Error al cambiar el nombre: {}", "名前の変更に失敗しました: {}", "이름 변경 실패: {}"]),
    ("status.renamed", ["已重命名", "已重新命名", "Renamed", "Umbenannt", "Renommé", "Rinominato", "Nombre cambiado", "名前を変更しました", "이름 변경됨"]),
    ("status.movedToTrash", ["已移到回收目录", "已移到回收目錄", "Moved to trash", "In den Papierkorb verschoben", "Déplacé dans la corbeille", "Spostato nel cestino", "Movido a la papelera", "ゴミ箱に移動しました", "휴지통으로 이동됨"]),
    ("dialog.images", ["图片", "圖片", "Images", "Bilder", "Images", "Immagini", "Imágenes", "画像", "이미지"]),
    ("status.imagesImported", ["图片导入完成", "圖片匯入完成", "Images imported", "Bilder importiert", "Images importées", "Immagini importate", "Imágenes importadas", "画像を読み込みました", "이미지 가져오기 완료"]),
    ("status.noPagesToExport", ["没有可导出的页面", "沒有可匯出的頁面", "No pages to export", "Keine Seiten zum Exportieren", "Aucune page à exporter", "Nessuna pagina da esportare", "No hay páginas para exportar", "書き出せるページがありません", "내보낼 페이지가 없습니다"]),
    ("status.pdfExported", ["已导出 PDF：{}", "已匯出 PDF：{}", "PDF exported: {}", "PDF exportiert: {}", "PDF exporté : {}", "PDF esportato: {}", "PDF exportado: {}", "PDF を書き出しました: {}", "PDF 내보냄: {}"]),
    ("status.pdfExportFailed", ["导出 PDF 失败：{}", "匯出 PDF 失敗：{}", "PDF export failed: {}", "PDF-Export fehlgeschlagen: {}", "Échec de l'export PDF : {}", "Esportazione PDF non riuscita: {}", "Error al exportar PDF: {}", "PDF の書き出しに失敗しました: {}", "PDF 내보내기 실패: {}"]),
    ("status.selectPageFirst", ["请先选择页面", "請先選擇頁面", "Select a page first", "Bitte zuerst eine Seite auswählen", "Sélectionnez d'abord une page", "Seleziona prima una pagina", "Selecciona primero una página", "先にページを選択してください", "먼저 페이지를 선택하세요"]),
    ("status.exported", ["已导出：{}", "已匯出：{}", "Exported: {}", "Exportiert: {}", "Exporté : {}", "Esportato: {}", "Exportado: {}", "書き出しました: {}", "내보냄: {}"]),
    ("status.exportFailed", ["导出失败：{}", "匯出失敗：{}", "Export failed: {}", "Export fehlgeschlagen: {}", "Échec de l'export : {}", "Esportazione non riuscita: {}", "Error al exportar: {}", "書き出しに失敗しました: {}", "내보내기 실패: {}"]),
    ("status.tesseractNotFound", ["未找到 Tesseract，请安装或在设置中指定路径", "找不到 Tesseract，請安裝或在設定中指定路徑", "Tesseract not found. Install it or set its path in the settings.", "Tesseract nicht gefunden. Bitte installieren oder den Pfad in den Einstellungen angeben.", "Tesseract introuvable. Installez-le ou indiquez son chemin dans les réglages.", "Tesseract non trovato. Installalo o indica il percorso nelle impostazioni.", "No se encontró Tesseract. Instálalo o indica la ruta en los ajustes.", "Tesseract が見つかりません。インストールするか設定でパスを指定してください。", "Tesseract를 찾을 수 없습니다. 설치하거나 설정에서 경로를 지정하세요."]),
    ("status.tesseractInstalled", ["Tesseract 安装完成", "Tesseract 安裝完成", "Tesseract installed", "Tesseract installiert", "Tesseract installé", "Tesseract installato", "Tesseract instalado", "Tesseract をインストールしました", "Tesseract 설치 완료"]),
    ("tesseract.title", ["未找到 Tesseract", "找不到 Tesseract", "Tesseract not found", "Tesseract nicht gefunden", "Tesseract introuvable", "Tesseract non trovato", "No se encontró Tesseract", "Tesseract が見つかりません", "Tesseract를 찾을 수 없습니다"]),
    ("tesseract.body", ["OCR 需要 Tesseract。是否现在从 GitHub 下载并安装？安装可能需要管理员权限，完成后会自动继续识别。", "OCR 需要 Tesseract。是否現在從 GitHub 下載並安裝？安裝可能需要系統管理員權限，完成後會自動繼續辨識。", "OCR requires Tesseract. Download and install it from GitHub now? The installer may request administrator permission, and OCR will continue automatically when it finishes.", "OCR benötigt Tesseract. Jetzt von GitHub herunterladen und installieren? Der Installer benötigt möglicherweise Administratorrechte; danach wird die OCR automatisch fortgesetzt.", "L'OCR nécessite Tesseract. Le télécharger et l'installer depuis GitHub maintenant ? L'installateur peut demander des droits administrateur ; l'OCR reprendra automatiquement à la fin.", "L'OCR richiede Tesseract. Scaricarlo e installarlo da GitHub ora? L'installer potrebbe richiedere i privilegi di amministratore; al termine l'OCR continuerà automaticamente.", "El OCR requiere Tesseract. ¿Descargarlo e instalarlo desde GitHub ahora? El instalador puede pedir permisos de administrador; el OCR continuará automáticamente al terminar.", "OCR には Tesseract が必要です。今すぐ GitHub からダウンロードしてインストールしますか？インストーラーが管理者権限を求める場合があります。完了後、OCR は自動的に続行されます。", "OCR에는 Tesseract가 필요합니다. 지금 GitHub에서 내려받아 설치할까요? 설치 관리자가 관리자 권한을 요청할 수 있으며, 완료되면 OCR이 자동으로 계속됩니다."]),
    ("tesseract.install", ["下载并安装", "下載並安裝", "Download and install", "Herunterladen und installieren", "Télécharger et installer", "Scarica e installa", "Descargar e instalar", "ダウンロードしてインストール", "다운로드 및 설치"]),
    ("tesseract.installing", ["正在下载并安装 Tesseract…这可能需要几分钟。", "正在下載並安裝 Tesseract…這可能需要幾分鐘。", "Downloading and installing Tesseract… This may take a few minutes.", "Tesseract wird heruntergeladen und installiert… Dies kann einige Minuten dauern.", "Téléchargement et installation de Tesseract… Cela peut prendre quelques minutes.", "Download e installazione di Tesseract… Potrebbe richiedere qualche minuto.", "Descargando e instalando Tesseract… Esto puede tardar unos minutos.", "Tesseract をダウンロードしてインストールしています…数分かかる場合があります。", "Tesseract를 다운로드하고 설치하는 중… 몇 분 걸릴 수 있습니다."]),
    ("tesseract.installed", ["Tesseract 安装完成，正在继续 OCR…", "Tesseract 安裝完成，正在繼續 OCR…", "Tesseract installed. Continuing OCR…", "Tesseract installiert. OCR wird fortgesetzt…", "Tesseract installé. Reprise de l'OCR…", "Tesseract installato. Ripresa dell'OCR…", "Tesseract instalado. Continuando con el OCR…", "Tesseract をインストールしました。OCR を続行します…", "Tesseract 설치 완료. OCR을 계속합니다…"]),
    ("tesseract.failed", ["安装失败：{}", "安裝失敗：{}", "Installation failed: {}", "Installation fehlgeschlagen: {}", "Échec de l'installation : {}", "Installazione non riuscita: {}", "Error de instalación: {}", "インストールに失敗しました: {}", "설치 실패: {}"]),
    ("status.ocrDone", ["OCR 完成：{} 个字符", "OCR 完成：{} 個字元", "OCR complete: {} characters", "OCR abgeschlossen: {} Zeichen", "OCR terminé : {} caractères", "OCR completato: {} caratteri", "OCR completado: {} caracteres", "OCR 完了: {} 文字", "OCR 완료: {}자"]),
    ("status.ocrFailed", ["OCR 失败：{}", "OCR 失敗：{}", "OCR failed: {}", "OCR fehlgeschlagen: {}", "Échec de l'OCR : {}", "OCR non riuscito: {}", "Error de OCR: {}", "OCR に失敗しました: {}", "OCR 실패: {}"]),
    ("status.settingsSaved", ["设置已保存", "設定已儲存", "Settings saved", "Einstellungen gespeichert", "Réglages enregistrés", "Impostazioni salvate", "Ajustes guardados", "設定を保存しました", "설정 저장됨"]),
    ("status.outputDirUpdated", ["输出目录已更新", "輸出目錄已更新", "Output folder updated", "Ausgabeordner aktualisiert", "Dossier de sortie mis à jour", "Cartella di output aggiornata", "Carpeta de salida actualizada", "出力先を更新しました", "출력 폴더 업데이트됨"]),
    ("status.outputDirSaved", ["输出目录已保存，将在新文档中生效", "輸出目錄已儲存，將在新文件中生效", "Output folder saved; it will apply to new documents", "Ausgabeordner gespeichert; gilt für neue Dokumente", "Dossier de sortie enregistré ; il s'appliquera aux nouveaux documents", "Cartella di output salvata; sarà usata per i nuovi documenti", "Carpeta de salida guardada; se aplicará a documentos nuevos", "出力先を保存しました。新しいドキュメントから適用されます", "출력 폴더가 저장되었습니다. 새 문서부터 적용됩니다"]),
    ("status.redetecting", ["正在重新检测设备…", "正在重新偵測裝置…", "Re-detecting devices…", "Geräte werden erneut gesucht…", "Nouvelle détection des appareils…", "Nuova ricerca dei dispositivi…", "Volviendo a detectar dispositivos…", "デバイスを再検出しています…", "장치 다시 감지 중…"]),
    ("status.languageChanged", ["语言已切换", "語言已切換", "Language changed", "Sprache geändert", "Langue modifiée", "Lingua cambiata", "Idioma cambiado", "言語を切り替えました", "언어가 변경되었습니다"]),
    ("status.noInstaller", ["该版本没有可自动安装的安装包", "此版本沒有可自動安裝的安裝包", "This release has no installer for automatic installation", "Diese Version enthält kein Installationspaket für die automatische Installation", "Cette version ne contient pas d'installateur pour l'installation automatique", "Questa versione non include un installer per l'installazione automatica", "Esta versión no incluye un instalador para la instalación automática", "このバージョンには自動インストール用のインストーラーがありません", "이 버전에는 자동 설치용 설치 파일이 없습니다"]),
    // 右键菜单
    ("menu.open", ["打开", "開啟", "Open", "Öffnen", "Ouvrir", "Apri", "Abrir", "開く", "열기"]),
    ("menu.rename", ["重命名", "重新命名", "Rename", "Umbenennen", "Renommer", "Rinomina", "Cambiar nombre", "名前を変更", "이름 변경"]),
    ("menu.reveal", ["在资源管理器中显示", "在檔案總管中顯示", "Show in File Explorer", "Im Datei-Explorer anzeigen", "Afficher dans l'Explorateur", "Mostra in Esplora file", "Mostrar en el Explorador", "エクスプローラーで表示", "파일 탐색기에서 보기"]),
    ("menu.autoCrop", ["自动裁边", "自動裁邊", "Auto crop", "Auto-Zuschnitt", "Recadrage auto", "Ritaglio auto", "Recorte automático", "自動トリミング", "자동 자르기"]),
    ("menu.autoDeskew", ["自动纠偏", "自動校正", "Auto deskew", "Auto ausrichten", "Redressement auto", "Raddrizza auto", "Enderezar auto", "自動傾き補正", "자동 기울기 보정"]),
    ("menu.ocr", ["OCR 识别", "OCR 辨識", "OCR recognition", "OCR-Erkennung", "Reconnaissance OCR", "Riconoscimento OCR", "Reconocimiento OCR", "OCR 認識", "OCR 인식"]),
    ("menu.exportJpg", ["导出 JPG", "匯出 JPG", "Export JPG", "JPG exportieren", "Exporter en JPG", "Esporta JPG", "Exportar JPG", "JPG を書き出す", "JPG 내보내기"]),
    ("menu.delete", ["删除", "刪除", "Delete", "Löschen", "Supprimer", "Elimina", "Eliminar", "削除", "삭제"]),
    ("dialog.updateTitle", ["发现新版本", "發現新版本", "Update available", "Update verfügbar", "Mise à jour disponible", "Aggiornamento disponibile", "Actualización disponible", "新しいバージョンがあります", "새 버전 사용 가능"]),
    ("dialog.updateBody", ["新版本 v{} 已发布，是否现在下载并安装？安装程序会自动关闭当前版本。", "新版本 v{} 已發布，是否現在下載並安裝？安裝程式會自動關閉目前版本。", "Version v{} is available. Download and install it now? The installer will close the current version.", "Version v{} ist verfügbar. Jetzt herunterladen und installieren? Der Installer schließt die aktuelle Version.", "La version v{} est disponible. La télécharger et l'installer maintenant ? L'installateur fermera la version actuelle.", "La versione v{} è disponibile. Scaricarla e installarla ora? L'installer chiuderà la versione corrente.", "La versión v{} está disponible. ¿Descargarla e instalarla ahora? El instalador cerrará la versión actual.", "バージョン v{} が公開されました。今すぐダウンロードしてインストールしますか？インストーラーが現在のバージョンを終了します。", "버전 v{}이(가) 공개되었습니다. 지금 내려받아 설치할까요? 설치 관리자가 현재 버전을 종료합니다."]),
    ("dialog.updateNow", ["立即更新", "立即更新", "Update now", "Jetzt aktualisieren", "Mettre à jour", "Aggiorna ora", "Actualizar ahora", "今すぐ更新", "지금 업데이트"]),
    ("dialog.later", ["稍后", "稍後", "Later", "Später", "Plus tard", "Più tardi", "Más tarde", "あとで", "나중에"]),
];

fn find_row(key: &str) -> Option<&'static [&'static str; 9]> {
    TABLE
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, row)| row)
}

/// 翻译一个键；未知键原样返回，便于开发时发现遗漏。
#[must_use]
pub fn t(key: &'static str) -> &'static str {
    find_row(key).map_or(key, |row| row[current().index()])
}

/// 翻译并依次替换模板中的 `{}`。
#[must_use]
pub fn t_args(key: &'static str, args: &[&str]) -> String {
    let mut text = t(key).to_string();
    for arg in args {
        let Some(index) = text.find("{}") else {
            break;
        };
        text.replace_range(index..index + 2, arg);
    }
    text
}

/// 本地化的页面数量文本。
#[must_use]
pub fn page_count(count: usize) -> String {
    let key = if count == 1 {
        "ui.pageCountOne"
    } else {
        "ui.pageCount"
    };
    t_args(key, &[&count.to_string()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_locale_tags() {
        assert_eq!(Lang::from_tag("zh-CN"), Lang::ZhHans);
        assert_eq!(Lang::from_tag("zh-Hans-CN"), Lang::ZhHans);
        assert_eq!(Lang::from_tag("zh-TW"), Lang::ZhHant);
        assert_eq!(Lang::from_tag("zh-Hant-HK"), Lang::ZhHant);
        assert_eq!(Lang::from_tag("de_AT"), Lang::De);
        assert_eq!(Lang::from_tag("fr-FR"), Lang::Fr);
        assert_eq!(Lang::from_tag("pt-BR"), Lang::En);
    }

    #[test]
    fn every_language_has_every_key() {
        for lang in Lang::ALL {
            for (key, row) in TABLE {
                if matches!(*key, "ui.pagePrefix" | "ui.pageSuffix") {
                    continue;
                }
                assert!(!row[lang.index()].is_empty(), "{key} is empty for {lang:?}");
            }
        }
    }

    #[test]
    fn placeholders_match_across_languages() {
        for (key, row) in TABLE {
            let expected = row[0].matches("{}").count();
            for (index, value) in row.iter().enumerate() {
                assert_eq!(
                    value.matches("{}").count(),
                    expected,
                    "{key} has a different placeholder count at language index {index}"
                );
            }
        }
    }

    #[test]
    fn formats_placeholders_in_order() {
        set_language(Lang::En);
        assert_eq!(
            t_args("status.connected", &["Cam", "640", "480"]),
            "Connected Cam  640×480"
        );
        assert_eq!(page_count(1), "1 page");
        assert_eq!(page_count(3), "3 pages");
    }
}
