// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 用系统默认程序打开文件，或在资源管理器中定位文件。

use std::path::Path;

use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::text::wide;

/// 用文件类型关联的默认程序打开 `path`。
pub fn open_path(path: &Path) {
    let operation = wide("open");
    let file = wide(&path.to_string_lossy());
    // SAFETY: 字符串缓冲区在调用期间有效；ShellExecuteW 不保留指针。
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// 打开资源管理器并选中 `path`。
pub fn reveal_in_explorer(path: &Path) {
    let operation = wide("open");
    let program = wide("explorer.exe");
    let parameters = wide(&format!("/select,\"{}\"", path.display()));
    // SAFETY: 字符串缓冲区在调用期间有效；ShellExecuteW 不保留指针。
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            program.as_ptr(),
            parameters.as_ptr(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}
