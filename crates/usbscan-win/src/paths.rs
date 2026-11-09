// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 系统已知文件夹与默认输出目录。
//!
//! 使用 `SHGetKnownFolderPath` 读取图片文件夹，这样即使用户把“图片”重定向到
//! `OneDrive` 或其它磁盘，默认输出目录也会跟随系统设置。

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::UI::Shell::{FOLDERID_Pictures, SHGetKnownFolderPath, KF_FLAG_DEFAULT};

/// 返回用户的“图片”文件夹；读取失败时回退到 `%USERPROFILE%\Pictures`。
#[must_use]
pub fn pictures_dir() -> Option<PathBuf> {
    known_folder(&FOLDERID_Pictures).or_else(|| {
        std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .map(|home| home.join("Pictures"))
    })
}

/// 默认的扫描输出目录：`<图片>\usbscan`。
#[must_use]
pub fn default_output_root() -> PathBuf {
    pictures_dir()
        .or_else(|| std::env::var_os("PUBLIC").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("usbscan")
}

fn known_folder(id: &windows_sys::core::GUID) -> Option<PathBuf> {
    let mut raw = std::ptr::null_mut();
    // SAFETY: 标准 Shell API 调用；成功时由系统分配 `raw` 指向的字符串，
    // 下面在复制到 `PathBuf` 后用 `CoTaskMemFree` 释放。
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
    // SAFETY: 上面已确定长度为 `len`。
    let wide = unsafe { std::slice::from_raw_parts(raw, len) };
    let path = PathBuf::from(OsString::from_wide(wide));
    // SAFETY: `raw` 来自 `SHGetKnownFolderPath`，必须用 `CoTaskMemFree` 释放。
    unsafe { CoTaskMemFree(raw.cast()) };
    if path.as_os_str().is_empty() {
        None
    } else {
        Some(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_output_is_absolute() {
        let path = default_output_root();
        assert!(path.is_absolute());
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("usbscan")
        );
    }
}
