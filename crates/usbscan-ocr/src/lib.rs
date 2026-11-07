// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 离线 OCR：调用本机 Tesseract 5。
//!
//! 不联网、不上传图片。程序只负责定位 `tesseract.exe`、执行识别并返回文本；
//! 语言模型由用户安装的 Tesseract 提供。

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// OCR 默认语言。
pub const DEFAULT_LANGUAGE: &str = "chi_sim+eng";

/// OCR 失败原因。
#[derive(Debug)]
pub enum OcrError {
    /// 找不到 tesseract.exe。
    NotFound,
    /// 无法启动进程或读取输出。
    Io(io::Error),
    /// Tesseract 返回失败。
    Failed(String),
    /// 识别结果为空。
    Empty,
    /// 语言名包含非法字符。
    InvalidLanguage(String),
}

impl std::fmt::Display for OcrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(
                f,
                "未找到 Tesseract 5。请安装 Tesseract 并加入 PATH，或把 tesseract.exe 放到常见安装目录"
            ),
            Self::Io(err) => write!(f, "无法运行 Tesseract：{err}"),
            Self::Failed(message) => write!(f, "Tesseract 识别失败：{message}"),
            Self::Empty => write!(f, "未识别到文字"),
            Self::InvalidLanguage(language) => write!(f, "OCR 语言名无效：{language}"),
        }
    }
}

impl std::error::Error for OcrError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for OcrError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

/// 查找 Tesseract：先看显式配置，再查 PATH 和常见安装目录。
#[must_use]
pub fn find_tesseract(configured: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = configured {
        if path.is_file() {
            return Some(path.to_path_buf());
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            let candidate = directory.join("tesseract.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let mut roots = Vec::new();
    for key in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        if let Some(value) = std::env::var_os(key) {
            roots.push(PathBuf::from(value));
        }
    }
    for root in roots {
        for relative in [
            "Tesseract-OCR\\tesseract.exe",
            "Programs\\Tesseract-OCR\\tesseract.exe",
        ] {
            let candidate = root.join(relative);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// 检查语言名，只允许 Tesseract 语言列表使用的字符。
pub fn validate_language(language: &str) -> Result<(), OcrError> {
    if language.is_empty()
        || !language
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '+' | '-'))
    {
        return Err(OcrError::InvalidLanguage(language.to_string()));
    }
    Ok(())
}

/// 识别单张图片，返回纯文本。
pub fn recognize(executable: &Path, image: &Path, language: &str) -> Result<String, OcrError> {
    validate_language(language)?;
    let mut command = Command::new(executable);
    command
        .arg(image)
        .arg("stdout")
        .arg("-l")
        .arg(language)
        .arg("--psm")
        .arg("3");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let output = command.output()?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(OcrError::Failed(if message.is_empty() {
            format!("退出码 {}", output.status.code().unwrap_or(-1))
        } else {
            message
        }));
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        Err(OcrError::Empty)
    } else {
        Ok(text)
    }
}

/// 依次尝试语言列表，返回第一个成功的结果。
pub fn recognize_with_fallback(
    executable: &Path,
    image: &Path,
    languages: &[&str],
) -> Result<(String, String), OcrError> {
    let mut last_error = None;
    for language in languages {
        match recognize(executable, image, language) {
            Ok(text) => return Ok((text, (*language).to_string())),
            Err(err) => last_error = Some(err),
        }
    }
    Err(last_error.unwrap_or(OcrError::Empty))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_language_names() {
        assert!(validate_language("chi_sim+eng").is_ok());
        assert!(validate_language("eng").is_ok());
        assert!(validate_language("chi_sim").is_ok());
        assert!(validate_language("").is_err());
        assert!(validate_language("eng; rm -rf").is_err());
        assert!(validate_language("eng+chi_sim&calc").is_err());
    }

    #[test]
    fn explicit_missing_path_falls_back_to_none_when_not_installed() {
        // 不假设机器上安装了 Tesseract；只验证传入不存在的路径不会 panic。
        let _ = find_tesseract(Some(Path::new("Z:\\definitely-missing\\tesseract.exe")));
    }
}
