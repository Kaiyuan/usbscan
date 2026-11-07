// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 轻量配置读写。
//!
//! 文件仍然是标准的 `%APPDATA%\usbscan\config.toml`，但只实现当前用到的
//! TOML 子集：字符串和整数。后续接入完整 `toml` + `serde` 时可无缝替换。

use std::fs;
use std::path::PathBuf;

/// 应用配置。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    /// 上次使用的摄像头符号链接。
    pub last_camera_id: Option<String>,
    /// 上次使用的摄像头友好名（符号链接失效时的回退线索）。
    pub last_camera_name: Option<String>,
    /// 输出根目录；`None` 使用默认的“图片\usbscan”。
    pub output_root: Option<PathBuf>,
    /// 文件名模板，不含扩展名。
    pub file_template: Option<String>,
    /// 子目录模板。
    pub folder_template: Option<String>,
    /// 重名策略：`append` / `skip` / `overwrite`。
    pub collision: Option<String>,
    /// JPEG 质量 1–100。
    pub jpeg_quality: Option<u8>,
    /// 自动裁边。
    pub auto_crop: Option<bool>,
    /// 自动纠偏。
    pub deskew: Option<bool>,
    /// 自动透视校正。
    pub perspective: Option<bool>,
    /// 去阴影。
    pub remove_shadow: Option<bool>,
    /// 白底净化。
    pub white_background: Option<bool>,
    /// 增强强度 0–100。
    pub enhance_strength: Option<u8>,
    /// 自定义 Tesseract 路径。
    pub tesseract_path: Option<String>,
    /// OCR 语言，例如 `chi_sim+eng`。
    pub ocr_language: Option<String>,
}

impl Config {
    /// 读取配置；文件不存在或损坏时返回默认值。
    #[must_use]
    pub fn load() -> Self {
        let Ok(text) = fs::read_to_string(config_path()) else {
            return Self::default();
        };
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);

        let mut config = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if let Some(value) = toml_string(line, "last_id") {
                config.last_camera_id = Some(value);
            } else if let Some(value) = toml_string(line, "last_name") {
                config.last_camera_name = Some(value);
            } else if let Some(value) = toml_string(line, "root") {
                config.output_root = Some(PathBuf::from(value));
            } else if let Some(value) = toml_string(line, "file_template") {
                config.file_template = Some(value);
            } else if let Some(value) = toml_string(line, "folder_template") {
                config.folder_template = Some(value);
            } else if let Some(value) = toml_string(line, "collision") {
                config.collision = Some(value);
            } else if let Some(value) = toml_u8(line, "jpeg_quality") {
                config.jpeg_quality = Some(value.clamp(1, 100));
            } else if let Some(value) = toml_bool(line, "auto_crop") {
                config.auto_crop = Some(value);
            } else if let Some(value) = toml_bool(line, "deskew") {
                config.deskew = Some(value);
            } else if let Some(value) = toml_bool(line, "perspective") {
                config.perspective = Some(value);
            } else if let Some(value) = toml_bool(line, "remove_shadow") {
                config.remove_shadow = Some(value);
            } else if let Some(value) = toml_bool(line, "white_background") {
                config.white_background = Some(value);
            } else if let Some(value) = toml_u8(line, "enhance_strength") {
                config.enhance_strength = Some(value.min(100));
            } else if let Some(value) = toml_string(line, "tesseract_path") {
                config.tesseract_path = Some(value);
            } else if let Some(value) = toml_string(line, "ocr_language") {
                config.ocr_language = Some(value);
            }
        }
        config
    }

    /// 原子写入配置；失败时静默忽略（配置不是关键路径）。
    pub fn save(&self) {
        let mut text = String::from("# usbscan 配置（自动生成）\n[device]\n");
        push_option_string(&mut text, "last_id", self.last_camera_id.as_deref());
        push_option_string(&mut text, "last_name", self.last_camera_name.as_deref());

        text.push_str("\n[output]\n");
        if let Some(root) = &self.output_root {
            push_string(&mut text, "root", &root.to_string_lossy());
        }

        text.push_str("\n[naming]\n");
        push_option_string(&mut text, "file_template", self.file_template.as_deref());
        push_option_string(
            &mut text,
            "folder_template",
            self.folder_template.as_deref(),
        );
        push_option_string(&mut text, "collision", self.collision.as_deref());
        if let Some(quality) = self.jpeg_quality {
            text.push_str("jpeg_quality = ");
            text.push_str(&quality.to_string());
            text.push('\n');
        }
        for (key, value) in [
            ("auto_crop", self.auto_crop),
            ("deskew", self.deskew),
            ("perspective", self.perspective),
            ("remove_shadow", self.remove_shadow),
            ("white_background", self.white_background),
        ] {
            if let Some(value) = value {
                text.push_str(key);
                text.push_str(" = ");
                text.push_str(if value { "1" } else { "0" });
                text.push('\n');
            }
        }
        if let Some(strength) = self.enhance_strength {
            text.push_str("enhance_strength = ");
            text.push_str(&strength.to_string());
            text.push('\n');
        }

        text.push_str("\n[ocr]\n");
        push_option_string(&mut text, "tesseract_path", self.tesseract_path.as_deref());
        push_option_string(&mut text, "ocr_language", self.ocr_language.as_deref());

        let path = config_path();
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let temp = path.with_extension("toml.tmp");
        if fs::write(&temp, text.as_bytes()).is_ok() {
            let _ = fs::rename(&temp, &path);
        }
    }
}

fn push_option_string(out: &mut String, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        push_string(out, key, value);
    }
}

fn push_string(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push_str(" = \"");
    out.push_str(&escape(value));
    out.push_str("\"\n");
}

/// `%APPDATA%\usbscan\config.toml`。
#[must_use]
pub fn config_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(|home| PathBuf::from(home).join("AppData").join("Roaming"))
        })
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("usbscan").join("config.toml")
}

/// 从一行 `key = "value"` 中取出字符串值。
fn toml_string(line: &str, key: &str) -> Option<String> {
    let (name, value) = line.split_once('=')?;
    if name.trim() != key {
        return None;
    }
    let value = value.trim();
    let inner = value.strip_prefix('"')?.strip_suffix('"')?;
    Some(unescape(inner))
}

/// 从一行 `key = 42` 中取出整数。
fn toml_u8(line: &str, key: &str) -> Option<u8> {
    let (name, value) = line.split_once('=')?;
    if name.trim() != key {
        return None;
    }
    value.trim().parse().ok()
}

fn toml_bool(line: &str, key: &str) -> Option<bool> {
    let (name, value) = line.split_once('=')?;
    if name.trim() != key {
        return None;
    }
    match value.trim() {
        "1" | "true" => Some(true),
        "0" | "false" => Some(false),
        _ => None,
    }
}

fn escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}

fn unescape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some(ch @ ('\\' | '"')) => out.push(ch),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_string_values() {
        assert_eq!(
            toml_string(r#"last_id = "\\\\?\usb#vid_1234""#, "last_id"),
            Some(r"\\?\usb#vid_1234".to_string())
        );
        assert_eq!(toml_string("last_id = \"a\"", "last_name"), None);
        assert_eq!(
            toml_string("last_name = \"a\"", "last_name"),
            Some("a".to_string())
        );
    }

    #[test]
    fn parses_integer_values() {
        assert_eq!(toml_u8("jpeg_quality = 92", "jpeg_quality"), Some(92));
        assert_eq!(toml_u8("jpeg_quality = 999", "jpeg_quality"), None);
        assert_eq!(toml_u8("jpeg_quality = 92", "other"), None);
    }

    #[test]
    fn parses_boolean_values() {
        assert_eq!(toml_bool("auto_crop = 1", "auto_crop"), Some(true));
        assert_eq!(toml_bool("auto_crop = 0", "auto_crop"), Some(false));
        assert_eq!(toml_bool("auto_crop = true", "auto_crop"), Some(true));
        assert_eq!(toml_bool("auto_crop = 2", "auto_crop"), None);
    }

    #[test]
    fn escape_round_trips_windows_paths() {
        let value = r#"\\?\usb#vid_046d&pid_0825#6&2a3b4c5d&0&1"{name}"#;
        assert_eq!(unescape(&escape(value)), value);
    }

    #[test]
    fn parses_escape_sequences() {
        assert_eq!(unescape(r#"a\\b\"c\nd"#), "a\\b\"c\nd");
    }
}
