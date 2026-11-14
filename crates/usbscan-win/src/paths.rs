// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 系统已知文件夹与默认输出目录（跨平台版本）。
//!
//! **Windows**：使用 `SHGetKnownFolderPath` 读取图片/文档文件夹。
//!
//! **macOS**：使用 `~/Pictures` 和 `~/Documents`。
//!
//! **Linux / UOS**：优先读取 `XDG_PICTURES_DIR` / `XDG_DOCUMENTS_DIR`；
//! 不存在时退回到 `~/Pictures` 和 `~/Documents`。

use std::path::PathBuf;

// ===========================================================================
// Windows 实现（原有代码，完全不变）
// ===========================================================================

#[cfg(windows)]
mod windows_impl {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;

    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{
        FOLDERID_Documents, FOLDERID_Pictures, SHGetKnownFolderPath, KF_FLAG_DEFAULT,
    };

    /// 返回用户的"图片"文件夹；读取失败时回退到 `%USERPROFILE%\Pictures`。
    #[must_use]
    pub fn pictures_dir() -> Option<PathBuf> {
        known_folder(&FOLDERID_Pictures).or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .map(|home| home.join("Pictures"))
        })
    }

    /// 返回用户的"文档"文件夹；读取失败时回退到 `%USERPROFILE%\Documents`。
    #[must_use]
    pub fn documents_dir() -> Option<PathBuf> {
        known_folder(&FOLDERID_Documents).or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .map(|home| home.join("Documents"))
        })
    }

    fn known_folder(id: &windows_sys::core::GUID) -> Option<PathBuf> {
        let mut raw = std::ptr::null_mut();
        // SAFETY: 标准 Shell API 调用。
        let result = unsafe {
            SHGetKnownFolderPath(
                id,
                KF_FLAG_DEFAULT as u32,
                std::ptr::null_mut(),
                &raw mut raw,
            )
        };
        if result < 0 || raw.is_null() {
            return None;
        }
        // SAFETY: `raw` 是系统返回的 NUL 结尾 UTF-16 字符串。
        let len = unsafe {
            let mut len = 0usize;
            while *raw.add(len) != 0 {
                len += 1;
            }
            len
        };
        let wide = unsafe { std::slice::from_raw_parts(raw, len) };
        let path = PathBuf::from(OsString::from_wide(wide));
        unsafe { CoTaskMemFree(raw.cast()) };
        if path.as_os_str().is_empty() {
            None
        } else {
            Some(path)
        }
    }
}

// ===========================================================================
// Unix 实现（macOS + Linux/UOS）
// ===========================================================================

#[cfg(not(windows))]
mod unix_impl {
    use std::path::PathBuf;

    /// 返回用户的图片目录。
    ///
    /// 查找顺序：
    /// 1. Linux：`$XDG_PICTURES_DIR`（xdg-user-dirs 设置的值，可能指向中文目录）
    /// 2. `$HOME/Pictures`
    #[must_use]
    pub fn pictures_dir() -> Option<PathBuf> {
        // XDG（Linux / UOS）
        #[cfg(target_os = "linux")]
        if let Some(dir) = xdg_user_dir("PICTURES") {
            return Some(dir);
        }
        // 通用 HOME 回退（macOS 也适用）
        home_subdir("Pictures")
    }

    /// 返回用户的文档目录。
    ///
    /// 查找顺序：
    /// 1. Linux：`$XDG_DOCUMENTS_DIR`
    /// 2. `$HOME/Documents`
    #[must_use]
    pub fn documents_dir() -> Option<PathBuf> {
        #[cfg(target_os = "linux")]
        if let Some(dir) = xdg_user_dir("DOCUMENTS") {
            return Some(dir);
        }
        home_subdir("Documents")
    }

    fn home_subdir(name: &str) -> Option<PathBuf> {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(name))
    }

    /// 读取 `~/.config/user-dirs.dirs` 中的 `XDG_<KEY>_DIR`。
    ///
    /// 格式：`XDG_PICTURES_DIR="$HOME/Pictures"` 或绝对路径。
    #[cfg(target_os = "linux")]
    fn xdg_user_dir(key: &str) -> Option<PathBuf> {
        // 先尝试环境变量直接给出的值（xdg-user-dirs-update 有时会设置）
        let env_key = format!("XDG_{key}_DIR");
        if let Some(val) = std::env::var_os(&env_key) {
            let path = PathBuf::from(val);
            if path.is_absolute() {
                return Some(path);
            }
        }
        // 再解析 user-dirs.dirs 文件
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|h| PathBuf::from(h).join(".config"))
            })?;
        let dirs_file = config_home.join("user-dirs.dirs");
        let text = std::fs::read_to_string(dirs_file).ok()?;
        let search = format!("{env_key}=");
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix(&search) {
                // 去掉两端的引号
                let value = rest.trim_matches('"');
                // 替换 $HOME 变量
                let home = std::env::var_os("HOME").unwrap_or_default();
                let resolved = if let Some(suffix) = value.strip_prefix("$HOME") {
                    PathBuf::from(&home).join(suffix.trim_start_matches('/'))
                } else {
                    PathBuf::from(value)
                };
                if resolved.is_absolute() {
                    return Some(resolved);
                }
            }
        }
        None
    }
}

// ===========================================================================
// 统一导出
// ===========================================================================

/// 返回用户的图片目录；失败时返回 `None`。
#[must_use]
pub fn pictures_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    return windows_impl::pictures_dir();
    #[cfg(not(windows))]
    return unix_impl::pictures_dir();
}

/// 返回用户的文档目录；失败时返回 `None`。
#[must_use]
pub fn documents_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    return windows_impl::documents_dir();
    #[cfg(not(windows))]
    return unix_impl::documents_dir();
}

/// 应用配置与会话目录：`<文档>/usbscan`（Windows）或 `~/.local/share/usbscan`（Linux）
/// 或 `~/Library/Application Support/usbscan`（macOS）。
#[must_use]
pub fn app_data_dir() -> PathBuf {
    #[cfg(windows)]
    {
        documents_dir()
            .or_else(|| std::env::var_os("PUBLIC").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("usbscan")
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(|home| {
                PathBuf::from(home)
                    .join("Library")
                    .join("Application Support")
                    .join("usbscan")
            })
            .unwrap_or_else(|| PathBuf::from(".usbscan"))
    }
    #[cfg(target_os = "linux")]
    {
        // XDG_DATA_HOME（通常是 ~/.local/share）
        std::env::var_os("XDG_DATA_HOME")
            .map(|base| PathBuf::from(base).join("usbscan"))
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|home| PathBuf::from(home).join(".local").join("share").join("usbscan"))
            })
            .unwrap_or_else(|| PathBuf::from(".usbscan"))
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        PathBuf::from(".usbscan")
    }
}

/// 默认的扫描输出目录：`<图片>/usbscan`。
#[must_use]
pub fn default_output_root() -> PathBuf {
    pictures_dir()
        .or_else(|| {
            #[cfg(windows)]
            return std::env::var_os("PUBLIC").map(PathBuf::from);
            #[cfg(not(windows))]
            return std::env::var_os("HOME").map(PathBuf::from);
        })
        .unwrap_or_else(|| PathBuf::from("."))
        .join("usbscan")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_output_is_absolute_or_relative() {
        let path = default_output_root();
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("usbscan")
        );
    }

    #[test]
    fn app_data_dir_ends_with_usbscan() {
        let path = app_data_dir();
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("usbscan")
        );
    }

    #[cfg(windows)]
    #[test]
    fn default_output_is_absolute() {
        let path = default_output_root();
        assert!(path.is_absolute());
    }

    #[cfg(windows)]
    #[test]
    fn app_data_lives_in_documents() {
        let path = app_data_dir();
        assert!(path.is_absolute());
        if let Some(documents) = documents_dir() {
            assert_eq!(path.parent(), Some(documents.as_path()));
        }
    }
}
