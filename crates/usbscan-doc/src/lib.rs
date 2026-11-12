// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 会话与页面模型。
//!
//! 一次会话对应一个文档，页面按拍摄顺序排列。会话清单默认写在
//! `<目录>/session.json`（主程序放在文档目录下，避免污染照片目录），
//! 采用临时文件 + 原子替换，崩溃后可以恢复。
//! 这里不依赖任何第三方库，便于在受限构建环境中继续工作。

mod json;

use std::fmt::Write as _;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub use json::Json;

/// 会话清单格式版本。
pub const SESSION_VERSION: u32 = 1;
/// 新文档的默认名称。
pub const DEFAULT_DOC_NAME: &str = "未命名文档";

/// 当前 Unix 时间戳（秒）。
#[must_use]
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

/// 会话中的一页。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// 拍摄序号，撤销后会被回收。
    pub seq: u64,
    /// 相对于输出根目录的成品文件路径。
    pub path: PathBuf,
    /// 相对于输出根目录的原始图像路径；未保留原图时为 `None`。
    pub raw: Option<PathBuf>,
    /// 成品宽度（像素）。
    pub width: u32,
    /// 成品高度（像素）。
    pub height: u32,
    /// 拍摄时间（Unix 秒）。
    pub captured_at: u64,
    /// 拍摄设备名。
    pub device: Option<String>,
}

impl Page {
    /// 用于写入 JSON 的路径形式：统一使用 `/`，避免反斜杠转义噪音。
    #[must_use]
    pub fn manifest_path(&self) -> String {
        path_to_manifest(&self.path)
    }
}

fn path_to_manifest(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// 一次拍摄会话。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub version: u32,
    pub doc_name: String,
    pub created_at: u64,
    pub updated_at: u64,
    /// 下一次拍摄使用的序号。
    pub next_seq: u64,
    pub pages: Vec<Page>,
    /// 退出时是否正常关闭（用于区分崩溃恢复与普通重开）。
    pub closed_cleanly: bool,
}

impl Default for Session {
    fn default() -> Self {
        Self::new(DEFAULT_DOC_NAME, now_unix())
    }
}

impl Session {
    /// 新建空会话，序号从 1 开始（见命名规则 §4.4）。
    #[must_use]
    pub fn new(doc_name: impl Into<String>, now: u64) -> Self {
        Self {
            version: SESSION_VERSION,
            doc_name: doc_name.into(),
            created_at: now,
            updated_at: now,
            next_seq: 1,
            pages: Vec::new(),
            closed_cleanly: false,
        }
    }

    /// 追加一页；序号必须由调用方通过 `next_seq` 分配。
    pub fn push_page(&mut self, page: Page) {
        if page.seq >= self.next_seq {
            self.next_seq = page.seq.saturating_add(1);
        }
        self.pages.push(page);
        self.updated_at = now_unix();
    }

    /// 撤销最后一页并回收序号。
    ///
    /// 返回被撤销的页面，调用方负责删除其成品文件。
    pub fn undo_last(&mut self) -> Option<Page> {
        let page = self.pages.pop()?;
        // 只有末尾页会被撤销，因此直接把序号回退到该页，供下次复用。
        self.next_seq = self.next_seq.min(page.seq);
        self.updated_at = now_unix();
        Some(page)
    }

    /// 删除指定页。
    ///
    /// 删除最后一页时回收序号；删除中间页只留下空号，不会与后续页面冲突。
    pub fn remove_page(&mut self, index: usize) -> Option<Page> {
        if index >= self.pages.len() {
            return None;
        }
        let page = self.pages.remove(index);
        if page.seq.saturating_add(1) == self.next_seq {
            self.next_seq = page.seq;
        }
        self.updated_at = now_unix();
        Some(page)
    }

    /// 载入后修正可能不一致的序号。
    pub fn normalize(&mut self) {
        let max_seq = self.pages.iter().map(|page| page.seq).max().unwrap_or(0);
        self.next_seq = self.next_seq.max(max_seq.saturating_add(1));
        if self.created_at == 0 {
            self.created_at = self.updated_at;
        }
        self.updated_at = self.updated_at.max(self.created_at);
    }

    /// 序列化为 JSON 文本。
    #[must_use]
    pub fn to_json_string(&self) -> String {
        let mut out = String::with_capacity(256 + self.pages.len() * 160);
        out.push_str("{\n");
        let _ = writeln!(out, "  \"version\": {},", self.version);
        let _ = writeln!(out, "  \"doc_name\": {},", json::quote(&self.doc_name));
        let _ = writeln!(out, "  \"created_at\": {},", self.created_at);
        let _ = writeln!(out, "  \"updated_at\": {},", self.updated_at);
        let _ = writeln!(out, "  \"next_seq\": {},", self.next_seq);
        let _ = writeln!(out, "  \"closed_cleanly\": {},", self.closed_cleanly);
        if self.pages.is_empty() {
            out.push_str("  \"pages\": []\n}\n");
            return out;
        }

        out.push_str("  \"pages\": [\n");
        for (index, page) in self.pages.iter().enumerate() {
            out.push_str("    {\"seq\": ");
            let _ = write!(out, "{}", page.seq);
            out.push_str(", \"path\": ");
            let _ = write!(out, "{}", json::quote(&page.manifest_path()));
            out.push_str(", \"raw\": ");
            match &page.raw {
                Some(raw) => {
                    let _ = write!(out, "{}", json::quote(&path_to_manifest(raw)));
                }
                None => out.push_str("null"),
            }
            let _ = write!(
                out,
                ", \"width\": {}, \"height\": {}, \"captured_at\": {}",
                page.width, page.height, page.captured_at
            );
            out.push_str(", \"device\": ");
            match &page.device {
                Some(device) => {
                    let _ = write!(out, "{}", json::quote(device));
                }
                None => out.push_str("null"),
            }
            out.push('}');
            if index + 1 < self.pages.len() {
                out.push(',');
            }
            out.push('\n');
        }
        out.push_str("  ]\n}\n");
        out
    }

    /// 从 JSON 值还原会话。
    pub fn from_json(value: &Json) -> Result<Self, StoreError> {
        if !matches!(value, Json::Object(_)) {
            return Err(StoreError::Invalid("会话清单顶层不是对象".to_string()));
        }

        let mut session = Self {
            version: value
                .get("version")
                .and_then(Json::as_u64)
                .map_or(SESSION_VERSION, |version| version as u32),
            doc_name: value
                .get("doc_name")
                .and_then(Json::as_str)
                .unwrap_or(DEFAULT_DOC_NAME)
                .to_string(),
            created_at: value.get("created_at").and_then(Json::as_u64).unwrap_or(0),
            updated_at: value.get("updated_at").and_then(Json::as_u64).unwrap_or(0),
            next_seq: value.get("next_seq").and_then(Json::as_u64).unwrap_or(1),
            pages: Vec::new(),
            closed_cleanly: matches!(value.get("closed_cleanly"), Some(Json::Bool(true))),
        };

        if let Some(items) = value.get("pages").and_then(Json::as_array) {
            session.pages.reserve(items.len());
            for (index, item) in items.iter().enumerate() {
                let page = page_from_json(item).map_err(|err| {
                    StoreError::Invalid(format!("第 {} 页无效：{err}", index + 1))
                })?;
                session.pages.push(page);
            }
        }
        session.normalize();
        Ok(session)
    }

    /// 从 JSON 文本还原会话。
    pub fn from_json_str(text: &str) -> Result<Self, StoreError> {
        Self::from_json(&json::parse(text).map_err(StoreError::Json)?)
    }
}

fn page_from_json(value: &Json) -> Result<Page, String> {
    if !matches!(value, Json::Object(_)) {
        return Err("页面不是对象".to_string());
    }
    let path = value
        .get("path")
        .and_then(Json::as_str)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| "缺少 path".to_string())?;
    Ok(Page {
        seq: value.get("seq").and_then(Json::as_u64).unwrap_or(1),
        path: PathBuf::from(path),
        raw: value
            .get("raw")
            .and_then(Json::as_str)
            .filter(|raw| !raw.is_empty())
            .map(PathBuf::from),
        width: value
            .get("width")
            .and_then(Json::as_u64)
            .map_or(0, |width| width as u32),
        height: value
            .get("height")
            .and_then(Json::as_u64)
            .map_or(0, |height| height as u32),
        captured_at: value.get("captured_at").and_then(Json::as_u64).unwrap_or(0),
        device: value
            .get("device")
            .and_then(Json::as_str)
            .map(ToString::to_string),
    })
}

/// 会话清单读写。
#[derive(Debug, Clone)]
pub struct SessionStore {
    dir: PathBuf,
}

impl SessionStore {
    /// 以输出根目录创建清单存储：`<输出根>/.usbscan/session.json`。
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self::in_dir(root.into().join(".usbscan"))
    }

    /// 把清单直接放在指定目录下，与照片目录分离。
    #[must_use]
    pub fn in_dir(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// 会话清单路径。
    #[must_use]
    pub fn manifest_path(&self) -> PathBuf {
        self.dir.join("session.json")
    }

    /// 读取清单；不存在时返回 `Ok(None)`。
    pub fn load(&self) -> Result<Option<Session>, StoreError> {
        let path = self.manifest_path();
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(StoreError::Io(err)),
        };
        let text = String::from_utf8(bytes)
            .map_err(|err| StoreError::Invalid(format!("会话清单不是 UTF-8：{err}")))?;
        // 某些编辑器会写入 UTF-8 BOM，容忍它比让用户重新生成清单友好。
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        Session::from_json_str(text).map(Some)
    }

    /// 原子写入清单：先写临时文件并落盘，再替换目标文件。
    pub fn save(&self, session: &Session) -> Result<(), StoreError> {
        let path = self.manifest_path();
        let dir = path
            .parent()
            .ok_or_else(|| StoreError::Invalid("会话清单路径没有父目录".to_string()))?;
        fs::create_dir_all(dir)?;

        let temp = dir.join(format!(
            "session.json.tmp.{}.{}",
            std::process::id(),
            now_unix()
        ));
        {
            let mut file = fs::File::create(&temp)?;
            file.write_all(session.to_json_string().as_bytes())?;
            file.flush()?;
            file.sync_all()?;
        }
        atomic_replace(&temp, &path).inspect_err(|_| {
            let _ = fs::remove_file(&temp);
        })?;
        Ok(())
    }

    /// 把现有清单归档为 `session-<时间>.json.bak`。
    pub fn archive(&self) -> Result<Option<PathBuf>, StoreError> {
        let path = self.manifest_path();
        if !path.exists() {
            return Ok(None);
        }
        let dir = path
            .parent()
            .ok_or_else(|| StoreError::Invalid("会话清单路径没有父目录".to_string()))?;
        let mut candidate = dir.join(format!("session-{}.json.bak", now_unix()));
        let mut counter = 1u32;
        while candidate.exists() {
            candidate = dir.join(format!("session-{}-{counter}.json.bak", now_unix()));
            counter += 1;
        }
        fs::rename(&path, &candidate)?;
        Ok(Some(candidate))
    }
}

#[cfg(windows)]
fn atomic_replace(from: &Path, to: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let from = wide_path(from);
    let to = wide_path(to);
    // SAFETY: 两个字符串都以 NUL 结尾，并在调用期间保持存活。
    let ok = unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn atomic_replace(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

#[cfg(windows)]
fn wide_path(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain([0]).collect()
}

/// 存储错误。
#[derive(Debug)]
pub enum StoreError {
    /// 文件读写失败。
    Io(io::Error),
    /// JSON 语法错误。
    Json(String),
    /// JSON 结构不符合预期。
    Invalid(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "会话文件读写失败：{err}"),
            Self::Json(err) => write!(f, "会话清单格式错误：{err}"),
            Self::Invalid(err) => write!(f, "会话清单内容无效：{err}"),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Json(_) | Self::Invalid(_) => None,
        }
    }
}

impl From<io::Error> for StoreError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(seq: u64, name: &str) -> Page {
        Page {
            seq,
            path: PathBuf::from(format!("2026/11/07/{name}.jpg")),
            raw: Some(PathBuf::from(format!("raw/{name}.jpg"))),
            width: 1920,
            height: 1080,
            captured_at: 1_700_000_000,
            device: Some("测试 摄像头".to_string()),
        }
    }

    #[test]
    fn push_advances_sequence() {
        let mut session = Session::new(DEFAULT_DOC_NAME, 1);
        session.push_page(page(1, "a"));
        session.push_page(page(2, "b"));
        assert_eq!(session.next_seq, 3);
        assert_eq!(session.pages.len(), 2);
    }

    #[test]
    fn undo_recycles_sequence() {
        let mut session = Session::new(DEFAULT_DOC_NAME, 1);
        session.push_page(page(1, "a"));
        session.push_page(page(2, "b"));
        let undone = session.undo_last().expect("page");
        assert_eq!(undone.seq, 2);
        assert_eq!(session.next_seq, 2);
        session.push_page(page(2, "b"));
        assert_eq!(session.next_seq, 3);
    }

    #[test]
    fn remove_page_recycles_only_last() {
        let mut session = Session::new(DEFAULT_DOC_NAME, 1);
        session.push_page(page(1, "a"));
        session.push_page(page(2, "b"));
        session.push_page(page(3, "c"));

        let middle = session.remove_page(1).expect("middle");
        assert_eq!(middle.seq, 2);
        assert_eq!(session.next_seq, 4);
        assert_eq!(session.pages.len(), 2);

        let last = session.remove_page(1).expect("last");
        assert_eq!(last.seq, 3);
        assert_eq!(session.next_seq, 3);
    }

    #[test]
    fn json_round_trips() {
        let mut session = Session::new("发票\"扫描\"", 10);
        session.push_page(page(1, "扫描_001"));
        session.push_page(Page {
            device: None,
            path: PathBuf::from("目录/文件.jpg"),
            ..page(2, "x")
        });
        session.updated_at = 20;

        let text = session.to_json_string();
        let restored = Session::from_json_str(&text).expect("restore");
        assert_eq!(restored, session);
        assert!(text.contains("\"path\": \"目录/文件.jpg\""));
    }

    #[test]
    fn rejects_missing_path() {
        let error = Session::from_json_str(r#"{"pages": [{"seq": 1}]}"#).unwrap_err();
        assert!(matches!(error, StoreError::Invalid(_)));
    }

    #[test]
    fn normalizes_sequence_on_load() {
        let session =
            Session::from_json_str(r#"{"next_seq": 1, "pages": [{"seq": 7, "path": "a.jpg"}]}"#)
                .expect("load");
        assert_eq!(session.next_seq, 8);
    }

    #[test]
    fn in_dir_keeps_manifest_out_of_the_photo_folder() {
        let store = SessionStore::in_dir(Path::new("C:/Users/me/Documents/usbscan"));
        assert_eq!(
            store.manifest_path(),
            PathBuf::from("C:/Users/me/Documents/usbscan/session.json")
        );
        let legacy = SessionStore::new(Path::new("C:/photos"));
        assert_eq!(
            legacy.manifest_path(),
            PathBuf::from("C:/photos/.usbscan/session.json")
        );
    }

    #[test]
    fn store_replaces_existing_manifest() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/test-tmp")
            .join(format!("usbscan-replace-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let store = SessionStore::new(&root);

        let mut session = Session::new(DEFAULT_DOC_NAME, 1);
        session.push_page(page(1, "a"));
        store.save(&session).expect("first save");
        session.push_page(page(2, "b"));
        store.save(&session).expect("second save");

        let loaded = store.load().expect("load").expect("session");
        assert_eq!(loaded.pages.len(), 2);
        assert_eq!(loaded.next_seq, 3);

        // UTF-8 BOM 不应阻止读取。
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(loaded.to_json_string().as_bytes());
        fs::write(store.manifest_path(), bytes).expect("bom write");
        assert!(store.load().expect("bom load").is_some());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn store_round_trips_and_archives() {
        // 使用 target 目录而不是系统临时目录：部分受管 Windows 环境会拒绝
        // 在 %TEMP% 下创建子目录，但构建目录一定可写。
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/test-tmp")
            .join(format!(
                "usbscan-doc-test-{}-{}",
                std::process::id(),
                now_unix()
            ));
        let store = SessionStore::new(&root);

        assert!(store.load().expect("empty").is_none());
        let mut session = Session::new(DEFAULT_DOC_NAME, 1);
        session.push_page(page(1, "a"));
        store.save(&session).expect("save");
        assert_eq!(store.load().expect("load"), Some(session));

        let archived = store.archive().expect("archive").expect("path");
        assert!(archived.exists());
        assert!(store.load().expect("after archive").is_none());

        let _ = fs::remove_dir_all(&root);
    }
}
