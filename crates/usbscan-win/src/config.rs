// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 轻量配置读写。
//!
//! 当前只需要记住"上次使用的摄像头"，因此先实现一个最小 TOML 子集读写器：
//! 文件仍然是标准的 `%APPDATA%\usbscan\config.toml`，后续接入完整配置系统时
//! 可以直接用 `toml` + `serde` 替换本模块，而不用迁移用户数据。

use std::fs;
use std::path::PathBuf;

/// 应用配置。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    /// 上次使用的摄像头符号链接。
    pub last_camera_id: Option<String>,
    /// 上次使用的摄像头友好名（符号链接失效时的回退线索）。
    pub last_camera_name: Option<String>,
}

impl Config {
    /// 读取配置；文件不存在或损坏时返回默认值。
    #[must_use]
    pub fn load() -> Self {
        let Ok(text) = fs::read_to_string(config_path()) else {
            return Self::default();
        };

        let mut config = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if config.last_camera_id.is_none() {
                if let Some(value) = toml_string(line, "last_id") {
                    config.last_camera_id = Some(value);
                    continue;
                }
            }
            if config.last_camera_name.is_none() {
                if let Some(value) = toml_string(line, "last_name") {
                    config.last_camera_name = Some(value);
                }
            }
        }
        config
    }

    /// 原子写入配置；失败时静默忽略（配置不是关键路径）。
    pub fn save(&self) {
        let mut text = String::from("# usbscan 配置（自动生成）\n[device]\n");
        if let Some(id) = &self.last_camera_id {
            text.push_str("last_id = \"");
            text.push_str(&escape(id));
            text.push_str("\"\n");
        }
        if let Some(name) = &self.last_camera_name {
            text.push_str("last_name = \"");
            text.push_str(&escape(name));
            text.push_str("\"\n");
        }

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
    fn escape_round_trips_windows_paths() {
        let value = r#"\\?\usb#vid_046d&pid_0825#6&2a3b4c5d&0&1"{name}"#;
        assert_eq!(unescape(&escape(value)), value);
    }

    #[test]
    fn parses_escape_sequences() {
        assert_eq!(unescape(r#"a\\b\"c\nd"#), "a\\b\"c\nd");
    }
}
