// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 用系统默认程序打开文件、在文件管理器中定位文件，或以特权运行程序。
//!
//! **Windows**：使用 `ShellExecuteW` / `ShellExecuteExW`（UAC 提权）。
//!
//! **macOS**：使用 `open` 命令；提权使用 `osascript do shell script ... with administrator privileges`。
//!
//! **Linux / UOS**：使用 `xdg-open`；提权使用 `pkexec`（UOS 预装）或 `sudo -A`。

use std::path::Path;

// ===========================================================================
// 提权错误（跨平台统一）
// ===========================================================================

/// 提权启动失败的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElevatedError {
    /// 用户在权限提示中取消。
    Cancelled,
    /// 启动命令失败，附错误信息。
    Launch(String),
    /// 等待进程失败。
    Wait(String),
    /// 此平台不支持自动提权，需要用户手动操作。
    NotSupported,
}

impl std::fmt::Display for ElevatedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => write!(f, "elevation was cancelled"),
            Self::Launch(msg) => write!(f, "failed to launch elevated process: {msg}"),
            Self::Wait(msg) => write!(f, "waiting for elevated process failed: {msg}"),
            Self::NotSupported => write!(
                f,
                "automatic elevation is not supported on this platform; please install manually"
            ),
        }
    }
}

impl std::error::Error for ElevatedError {}

// ===========================================================================
// Windows 实现
// ===========================================================================

#[cfg(windows)]
mod windows_impl {
    use std::path::Path;

    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_CANCELLED, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, WaitForSingleObject, INFINITE,
    };
    use windows_sys::Win32::UI::Shell::{
        ShellExecuteExW, ShellExecuteW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    use crate::text::wide;

    use super::ElevatedError;

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
        if unsafe { ShellExecuteExW(&mut info) } == 0 {
            let code = unsafe { GetLastError() };
            return Err(if code == ERROR_CANCELLED {
                ElevatedError::Cancelled
            } else {
                ElevatedError::Launch(format!("Win32 error {code}"))
            });
        }
        if info.hProcess.is_null() {
            return Err(ElevatedError::Launch("no process handle".into()));
        }
        let waited = unsafe { WaitForSingleObject(info.hProcess, INFINITE) };
        let mut exit_code = 0u32;
        let read = unsafe { GetExitCodeProcess(info.hProcess, &mut exit_code) };
        unsafe { CloseHandle(info.hProcess) };
        if waited != WAIT_OBJECT_0 {
            return Err(ElevatedError::Wait(format!("WaitForSingleObject={waited}")));
        }
        if read == 0 {
            let code = unsafe { GetLastError() };
            return Err(ElevatedError::Wait(format!(
                "GetExitCodeProcess error {code}"
            )));
        }
        Ok(exit_code)
    }

    pub fn open_path(path: &Path) {
        let operation = wide("open");
        let file = wide(&path.to_string_lossy());
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

    pub fn reveal_in_explorer(path: &Path) {
        let operation = wide("open");
        let program = wide("explorer.exe");
        let parameters = wide(&format!("/select,\"{}\"", path.display()));
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
}

// ===========================================================================
// Unix 实现（macOS + Linux/UOS）
// ===========================================================================

#[cfg(not(windows))]
mod unix_impl {
    use std::path::Path;
    use std::process::Command;

    use super::ElevatedError;

    /// 以特权运行安装脚本并等待退出。
    ///
    /// **macOS**：使用 `osascript` `AppleScript` `do shell script ... with administrator privileges`。
    ///
    /// **Linux / UOS**：优先使用 `pkexec`（Polkit 图形授权，UOS 预装）；
    /// `pkexec` 不可用时返回 [`ElevatedError::NotSupported`]，
    /// 由上层提示用户手动运行 `sudo apt install tesseract-ocr`。
    pub fn run_elevated_and_wait(path: &Path, arguments: &str) -> Result<u32, ElevatedError> {
        #[cfg(target_os = "macos")]
        {
            // osascript 形式：do shell script "path args" with administrator privileges
            let script = format!(
                r#"do shell script "{} {}" with administrator privileges"#,
                shell_escape(path.to_string_lossy().as_ref()),
                arguments,
            );
            let status = Command::new("osascript")
                .arg("-e")
                .arg(&script)
                .status()
                .map_err(|err| ElevatedError::Launch(err.to_string()))?;
            Ok(status.code().unwrap_or(1) as u32)
        }
        #[cfg(target_os = "linux")]
        {
            // 尝试 pkexec（Polkit 图形授权，UOS / GNOME / KDE 均预装）
            let pkexec = which_command("pkexec");
            if let Some(pkexec) = pkexec {
                let mut args_vec: Vec<&str> = vec![path.to_str().unwrap_or("")];
                args_vec.extend(arguments.split_whitespace());
                let status = Command::new(pkexec)
                    .args(&args_vec)
                    .status()
                    .map_err(|err| ElevatedError::Launch(err.to_string()))?;
                let code = status.code().unwrap_or(1);
                // pkexec 返回 126 / 127 表示用户取消或未授权
                if code == 126 || code == 127 {
                    return Err(ElevatedError::Cancelled);
                }
                return Ok(code as u32);
            }
            Err(ElevatedError::NotSupported)
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            Err(ElevatedError::NotSupported)
        }
    }

    /// 用系统默认程序打开文件或 URL。
    pub fn open_path(path: &Path) {
        #[cfg(target_os = "macos")]
        {
            let _ = Command::new("open").arg(path).spawn();
        }
        #[cfg(target_os = "linux")]
        {
            // xdg-open 是标准；UOS/GNOME 还支持 gio open
            if which_command("xdg-open").is_some() {
                let _ = Command::new("xdg-open").arg(path).spawn();
            } else if which_command("gio").is_some() {
                let _ = Command::new("gio").arg("open").arg(path).spawn();
            }
        }
    }

    /// 在文件管理器中定位并选中文件。
    pub fn reveal_in_explorer(path: &Path) {
        #[cfg(target_os = "macos")]
        {
            let _ = Command::new("open").arg("-R").arg(path).spawn();
        }
        #[cfg(target_os = "linux")]
        {
            // UOS / Deepin 的 dde-file-manager 支持 --show-item（选中文件）。
            if which_command("dde-file-manager").is_some() {
                let _ = Command::new("dde-file-manager")
                    .arg("--show-item")
                    .arg(path)
                    .spawn();
            } else if which_command("nautilus").is_some() {
                // 尝试 Nautilus（GNOME）的 --select 选项
                let _ = Command::new("nautilus").arg("--select").arg(path).spawn();
            } else if which_command("dolphin").is_some() {
                let _ = Command::new("dolphin").arg("--select").arg(path).spawn();
            } else {
                // 退化：打开父目录
                if let Some(parent) = path.parent() {
                    open_path(parent);
                }
            }
        }
    }

    /// 检查命令是否存在于 PATH 中。
    #[cfg(target_os = "linux")]
    fn which_command(name: &str) -> Option<std::ffi::OsString> {
        if let Some(path_var) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path_var) {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Some(candidate.into_os_string());
                }
            }
        }
        None
    }

    /// 简单的 shell 路径转义（单引号包裹，内部单引号用 '\'' 替换）。
    #[cfg(target_os = "macos")]
    fn shell_escape(s: &str) -> String {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

// ===========================================================================
// 统一导出
// ===========================================================================

/// 以提升权限运行程序并等待退出，返回进程退出码。
///
/// 用户取消权限提示时返回 [`ElevatedError::Cancelled`]。
/// 平台不支持自动提权时返回 [`ElevatedError::NotSupported`]。
pub fn run_elevated_and_wait(path: &Path, arguments: &str) -> Result<u32, ElevatedError> {
    #[cfg(windows)]
    return windows_impl::run_elevated_and_wait(path, arguments);
    #[cfg(not(windows))]
    return unix_impl::run_elevated_and_wait(path, arguments);
}

/// 用文件类型关联的默认程序打开 `path`。
pub fn open_path(path: &Path) {
    #[cfg(windows)]
    windows_impl::open_path(path);
    #[cfg(not(windows))]
    unix_impl::open_path(path);
}

/// 在文件管理器中定位并选中 `path`。
pub fn reveal_in_explorer(path: &Path) {
    #[cfg(windows)]
    windows_impl::reveal_in_explorer(path);
    #[cfg(not(windows))]
    unix_impl::reveal_in_explorer(path);
}
