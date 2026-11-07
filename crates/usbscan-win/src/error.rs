// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! Win32 错误码封装。

use std::fmt;

use windows_sys::Win32::Foundation::GetLastError;

/// 一个 Win32 错误码（`GetLastError` 的返回值）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WinError(pub u32);

impl WinError {
    /// 取当前线程的最后一个错误码。
    #[must_use]
    pub fn last() -> Self {
        // SAFETY: GetLastError 只读取线程局部状态，无副作用。
        Self(unsafe { GetLastError() })
    }

    /// 是否表示成功（错误码 0）。
    #[must_use]
    pub fn is_ok(self) -> bool {
        self.0 == 0
    }
}

impl fmt::Display for WinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Win32 错误 {}", self.0)
    }
}

impl std::error::Error for WinError {}
