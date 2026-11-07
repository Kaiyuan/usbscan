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
    // 便携版：与 usbscan.exe 同目录的 tesseract\tesseract.exe。
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            for relative in ["tesseract\\tesseract.exe", "tesseract\\bin\\tesseract.exe"] {
                let candidate = directory.join(relative);
                if candidate.is_file() {
                    return Some(candidate);
                }
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

/// 便携版 Tesseract 的 tessdata 目录。
fn apply_portable_tessdata(command: &mut Command, executable: &Path) {
    let Some(directory) = executable.parent() else {
        return;
    };
    for relative in ["tessdata", "..\\tessdata"] {
        let candidate = directory.join(relative);
        if candidate.is_dir() {
            command.env("TESSDATA_PREFIX", candidate);
            return;
        }
    }
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
    apply_portable_tessdata(&mut command, executable);
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

/// OCR 识别出的一行文字及其像素包围盒。
#[derive(Debug, Clone, PartialEq)]
pub struct OcrLine {
    pub text: String,
    pub left: u32,
    pub top: u32,
    pub width: u32,
    pub height: u32,
    pub confidence: f32,
}

/// 一次 OCR 的完整结果。
#[derive(Debug, Clone, PartialEq)]
pub struct OcrDocument {
    pub text: String,
    pub lines: Vec<OcrLine>,
    pub language: String,
}

/// 识别图片并返回带坐标的文本行。
pub fn recognize_document(
    executable: &Path,
    image: &Path,
    language: &str,
) -> Result<OcrDocument, OcrError> {
    validate_language(language)?;
    let mut command = Command::new(executable);
    apply_portable_tessdata(&mut command, executable);
    command
        .arg(image)
        .arg("stdout")
        .arg("-l")
        .arg(language)
        .arg("--psm")
        .arg("3")
        .arg("tsv");
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
    let tsv = String::from_utf8_lossy(&output.stdout);
    let lines = parse_tsv(&tsv);
    let text = lines
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\r\n");
    if text.trim().is_empty() {
        return Err(OcrError::Empty);
    }
    Ok(OcrDocument {
        text,
        lines,
        language: language.to_string(),
    })
}

/// 依次尝试语言列表，返回第一个成功的带坐标结果。
pub fn recognize_document_with_fallback(
    executable: &Path,
    image: &Path,
    languages: &[&str],
) -> Result<OcrDocument, OcrError> {
    let mut last_error = None;
    for language in languages {
        match recognize_document(executable, image, language) {
            Ok(document) => return Ok(document),
            Err(err) => last_error = Some(err),
        }
    }
    Err(last_error.unwrap_or(OcrError::Empty))
}

/// 解析 Tesseract TSV 输出，把单词合并为文本行。
#[must_use]
pub fn parse_tsv(text: &str) -> Vec<OcrLine> {
    struct Accumulator {
        text: String,
        left: u32,
        top: u32,
        right: u32,
        bottom: u32,
        confidence_sum: f64,
        words: u32,
    }

    let mut groups: Vec<Accumulator> = Vec::new();
    let mut keys: Vec<(u32, u32, u32)> = Vec::new();
    for row in text.lines().skip(1) {
        let columns: Vec<&str> = row.split('\t').collect();
        if columns.len() < 12 || columns[0].trim() != "5" {
            continue;
        }
        let word = columns[11].trim();
        if word.is_empty() {
            continue;
        }
        let Ok(left) = columns[6].trim().parse::<u32>() else {
            continue;
        };
        let Ok(top) = columns[7].trim().parse::<u32>() else {
            continue;
        };
        let Ok(width) = columns[8].trim().parse::<u32>() else {
            continue;
        };
        let Ok(height) = columns[9].trim().parse::<u32>() else {
            continue;
        };
        let confidence = columns[10].trim().parse::<f64>().unwrap_or(0.0);
        let key = (
            columns[2].trim().parse::<u32>().unwrap_or(0),
            columns[3].trim().parse::<u32>().unwrap_or(0),
            columns[4].trim().parse::<u32>().unwrap_or(0),
        );
        let right = left.saturating_add(width);
        let bottom = top.saturating_add(height);
        if let Some(index) = keys.iter().position(|existing| *existing == key) {
            let group = &mut groups[index];
            append_word(&mut group.text, word);
            group.left = group.left.min(left);
            group.top = group.top.min(top);
            group.right = group.right.max(right);
            group.bottom = group.bottom.max(bottom);
            group.confidence_sum += confidence;
            group.words += 1;
        } else {
            keys.push(key);
            groups.push(Accumulator {
                text: word.to_string(),
                left,
                top,
                right,
                bottom,
                confidence_sum: confidence,
                words: 1,
            });
        }
    }

    groups
        .into_iter()
        .map(|group| OcrLine {
            text: group.text,
            left: group.left,
            top: group.top,
            width: group.right.saturating_sub(group.left),
            height: group.bottom.saturating_sub(group.top),
            confidence: if group.words == 0 {
                0.0
            } else {
                (group.confidence_sum / f64::from(group.words)) as f32
            },
        })
        .collect()
}

fn append_word(target: &mut String, word: &str) {
    if target.is_empty() {
        target.push_str(word);
        return;
    }
    let previous = target.chars().last();
    let next = word.chars().next();
    if !previous.is_some_and(is_cjk) && !next.is_some_and(is_cjk) {
        target.push(' ');
    }
    target.push_str(word);
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32, 0x3000..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF)
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

    #[test]
    fn parses_tsv_lines_and_bounding_boxes() {
        let tsv = concat!(
            "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n",
            "5\t1\t1\t1\t1\t1\t10\t20\t40\t18\t95.5\tHello\n",
            "5\t1\t1\t1\t1\t2\t60\t20\t40\t18\t90.0\tWorld\n",
            "5\t1\t1\t1\t2\t1\t10\t50\t40\t20\t88.0\t中文\n",
            "5\t1\t1\t1\t2\t2\t52\t50\t40\t20\t92.0\t字符\n",
        );
        let lines = parse_tsv(tsv);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Hello World");
        assert_eq!((lines[0].left, lines[0].top), (10, 20));
        assert_eq!((lines[0].width, lines[0].height), (90, 18));
        assert_eq!(lines[1].text, "中文字符");
        assert_eq!(lines[1].width, 82);
        assert!(lines[0].confidence > 90.0 && lines[0].confidence < 96.0);
    }

    #[test]
    fn ignores_non_word_rows_and_bad_numbers() {
        let tsv = concat!(
            "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n",
            "4\t1\t1\t1\t1\t0\t0\t0\t100\t50\t-1\t\n",
            "5\t1\t1\t1\t1\t1\tbad\t20\t40\t18\t95\tWord\n",
            "5\t1\t1\t1\t1\t1\t10\t20\t40\t18\t95\tReal\n",
        );
        let lines = parse_tsv(tsv);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "Real");
    }
}
