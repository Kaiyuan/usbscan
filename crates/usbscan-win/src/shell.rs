// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 用系统默认程序打开文件、在资源管理器中定位文件，或以管理员权限运行程序。

use std::path::Path;

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_CANCELLED, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
use windows_sys::Win32::UI::Shell::{
    ShellExecuteExW, ShellExecuteW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::text::wide;

/// 提权启动失败的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElevatedError {
    /// 用户在 UAC 提示中取消。
    Cancelled,
    /// `ShellExecuteExW` 失败，附 Win32 错误码。
    Launch(u32),
    /// 系统没有返回进程句柄。
    NoProcess,
    /// 等待安装进程失败，附 Win32 错误码。
    Wait(u32),
}

impl std::fmt::Display for ElevatedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => write!(f, "elevation was cancelled"),
            Self::Launch(code) => write!(f, "ShellExecuteEx failed with error {code}"),
            Self::NoProcess => write!(f, "the elevated process was not created"),
            Self::Wait(code) => write!(
                f,
                "waiting for the elevated process failed with error {code}"
            ),
        }
    }
}

/// 以管理员权限启动程序并等待它退出，返回退出码。
///
/// 使用 `runas` 动词触发 UAC；用户拒绝时返回 [`ElevatedError::Cancelled`]。
pub fn run_elevated_and_wait(path: &Path, arguments: &str) -> Result<u32, ElevatedError> {
    let operation = wide("runas");
    let file = wide(&path.to_string_lossy());
    let parameters = wide(arguments);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: operation.as_ptr(),
        lpFile: file.as_ptr(),
        lpParameters: parameters.as_ptr(),
        nShow: SW_SHOWNORMAL,
        ..Default::default()
    };

    // SAFETY: 结构体与缓冲区在调用期间有效；成功后 hProcess 由我们负责关闭。
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        // SAFETY: 立即读取上一个 Win32 错误码。
        let code = unsafe { GetLastError() };
        return Err(if code == ERROR_CANCELLED {
            ElevatedError::Cancelled
        } else {
            ElevatedError::Launch(code)
        });
    }
    if info.hProcess.is_null() {
        return Err(ElevatedError::NoProcess);
    }

    // SAFETY: hProcess 是 ShellExecuteExW 返回的有效进程句柄。
    let waited = unsafe { WaitForSingleObject(info.hProcess, INFINITE) };
    let mut exit_code = 0u32;
    // SAFETY: 同上；exit_code 是有效的输出指针。
    let read = unsafe { GetExitCodeProcess(info.hProcess, &mut exit_code) };
    // SAFETY: 句柄由本次调用创建，只关闭一次。
    unsafe { CloseHandle(info.hProcess) };
    if waited != WAIT_OBJECT_0 {
        return Err(ElevatedError::Wait(waited));
    }
    if read == 0 {
        // SAFETY: 立即读取上一个 Win32 错误码。
        return Err(ElevatedError::Wait(unsafe { GetLastError() }));
    }
    Ok(exit_code)
}

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
