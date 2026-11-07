// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! UTF-16 字符串工具。
//!
//! Win32 的 `W` 系列 API 需要以 NUL 结尾的 UTF-16 序列。

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

/// 把 `&str` 转成以 NUL 结尾的 UTF-16 序列。
///
/// 返回的 `Vec` 必须在调用 Win32 API 期间保持存活。
#[must_use]
pub fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// 把 `&OsStr` 转成以 NUL 结尾的 UTF-16 序列。
#[must_use]
pub fn wide_os(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// 把 UTF-16 缓冲区转成 `String`，截断到第一个 NUL。
#[must_use]
pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_appends_nul() {
        let w = wide("中文abc");
        assert_eq!(w.last(), Some(&0));
        // "中文" 2 个 UTF-16 码元 + "abc" 3 个 + NUL = 6。
        assert_eq!(w.len(), 5 + 1);
    }

    #[test]
    fn from_wide_stops_at_nul() {
        let w = wide("hello");
        assert_eq!(from_wide(&w), "hello");
    }
}
