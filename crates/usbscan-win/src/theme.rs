// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 系统主题探测与深色模式应用（跨平台版本）。
//!
//! **Windows**：读取注册表 `AppsUseLightTheme`；深色模式应用依赖 `uxtheme.dll` 未公开导出。
//!
//! **macOS**：读取 `NSApp.effectiveAppearance`（通过 `defaults read` 命令）。
//!
//! **Linux / UOS**：读取 GTK / GNOME 颜色方案设置（`gsettings`）。
//! UOS 使用 DDE（深度桌面环境），通过 `dbus` 或配置文件探测。
//!
//! 非 Windows 平台上的 `apply_window` / `apply_control` 为空操作：
//! 深色模式由 Slint 框架根据系统主题自动处理。

// ===========================================================================
// Windows 实现（原有代码）
// ===========================================================================

#[cfg(windows)]
mod windows_impl {
    use windows_sys::Win32::Foundation::{HMODULE, HWND};
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE,
        DWMWCP_ROUND,
    };
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    use windows_sys::Win32::UI::Controls::SetWindowTheme;

    use crate::text::wide;

    const PERSONALIZE_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
    const APPS_USE_LIGHT_THEME: &str = "AppsUseLightTheme";

    pub fn system_uses_dark_mode() -> bool {
        let key = wide(PERSONALIZE_KEY);
        let value = wide(APPS_USE_LIGHT_THEME);
        let mut data: u32 = 1;
        let mut size: u32 = 4;
        let rc = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                std::ptr::from_mut(&mut data).cast(),
                &mut size,
            )
        };
        rc == 0 && data == 0
    }

    pub fn init_process(dark: bool) {
        unsafe {
            let module = load_uxtheme();
            if module.is_null() {
                return;
            }
            if let Some(set_mode) = resolve::<unsafe extern "system" fn(i32) -> i32>(module, 135) {
                set_mode(if dark { 2 } else { 3 });
            }
        }
    }

    pub fn apply_window(hwnd: HWND, dark: bool) {
        unsafe {
            let module = load_uxtheme();
            if !module.is_null() {
                if let Some(set_mode) =
                    resolve::<unsafe extern "system" fn(i32) -> i32>(module, 135)
                {
                    set_mode(if dark { 2 } else { 3 });
                }
                if let Some(allow) =
                    resolve::<unsafe extern "system" fn(HWND, i32) -> i32>(module, 133)
                {
                    allow(hwnd, i32::from(dark));
                }
            }
            let value: i32 = i32::from(dark);
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
                std::ptr::from_ref(&value).cast(),
                u32::try_from(std::mem::size_of::<i32>()).unwrap_or(4),
            );
            let name = if dark {
                wide("DarkMode_Explorer")
            } else {
                wide("Explorer")
            };
            SetWindowTheme(hwnd, name.as_ptr(), std::ptr::null());
            windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
                hwnd,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_THEMECHANGED,
                0,
                0,
            );
            let corner = DWMWCP_ROUND;
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                std::ptr::from_ref(&corner).cast(),
                u32::try_from(std::mem::size_of::<i32>()).unwrap_or(4),
            );
        }
    }

    pub fn apply_control(hwnd: HWND, dark: bool) {
        let name = if dark {
            wide("DarkMode_Explorer")
        } else {
            wide("Explorer")
        };
        unsafe {
            SetWindowTheme(hwnd, name.as_ptr(), std::ptr::null());
            windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
                hwnd,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_THEMECHANGED,
                0,
                0,
            );
            windows_sys::Win32::Graphics::Gdi::InvalidateRect(hwnd, std::ptr::null(), 1);
        }
    }

    pub fn apply_named(hwnd: HWND, name: &str) {
        let name = wide(name);
        unsafe {
            SetWindowTheme(hwnd, name.as_ptr(), std::ptr::null());
            windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
                hwnd,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_THEMECHANGED,
                0,
                0,
            );
            windows_sys::Win32::Graphics::Gdi::InvalidateRect(hwnd, std::ptr::null(), 1);
        }
    }

    pub fn apply_plain(hwnd: HWND) {
        let blank = wide(" ");
        unsafe {
            SetWindowTheme(hwnd, blank.as_ptr(), blank.as_ptr());
            windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
                hwnd,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_THEMECHANGED,
                0,
                0,
            );
            windows_sys::Win32::Graphics::Gdi::InvalidateRect(hwnd, std::ptr::null(), 1);
        }
    }

    fn load_uxtheme() -> HMODULE {
        let name = wide("uxtheme.dll");
        unsafe { LoadLibraryW(name.as_ptr()) }
    }

    unsafe fn resolve<T: Copy>(module: HMODULE, ordinal: u16) -> Option<T> {
        debug_assert_eq!(std::mem::size_of::<T>(), std::mem::size_of::<*const ()>());
        unsafe {
            let proc = GetProcAddress(module, ordinal as usize as *const u8)?;
            Some(std::mem::transmute_copy::<usize, T>(&(proc as usize)))
        }
    }
}

// ===========================================================================
// Unix 实现：主题探测
// ===========================================================================

#[cfg(not(windows))]
mod unix_impl {
    /// 探测系统是否使用深色主题。
    ///
    /// **macOS**：读取 `defaults read -g AppleInterfaceStyle`，返回 `"Dark"` 表示深色。
    ///
    /// **Linux / UOS**：
    ///   1. 尝试 `gsettings get org.gnome.desktop.interface color-scheme`（GNOME 42+ / UOS）
    ///   2. 尝试 `gsettings get org.gnome.desktop.interface gtk-theme`（GNOME 旧版）
    ///   3. 失败时默认为浅色。
    pub fn system_uses_dark_mode() -> bool {
        #[cfg(target_os = "macos")]
        {
            if let Ok(output) = std::process::Command::new("defaults")
                .args(["read", "-g", "AppleInterfaceStyle"])
                .output()
            {
                let stdout = String::from_utf8_lossy(&output.stdout);
                return stdout.trim().eq_ignore_ascii_case("dark");
            }
            false
        }
        #[cfg(target_os = "linux")]
        {
            // GNOME 42+ / UOS color-scheme
            if let Ok(output) = std::process::Command::new("gsettings")
                .args(["get", "org.gnome.desktop.interface", "color-scheme"])
                .output()
            {
                let val = String::from_utf8_lossy(&output.stdout);
                if val.to_lowercase().contains("dark") {
                    return true;
                }
            }
            // 旧版 GNOME gtk-theme 名称含 "dark"
            if let Ok(output) = std::process::Command::new("gsettings")
                .args(["get", "org.gnome.desktop.interface", "gtk-theme"])
                .output()
            {
                let val = String::from_utf8_lossy(&output.stdout);
                if val.to_lowercase().contains("dark") {
                    return true;
                }
            }
            false
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            false
        }
    }
}

// ===========================================================================
// 统一导出
// ===========================================================================

/// 系统当前是否使用深色主题。
#[must_use]
pub fn system_uses_dark_mode() -> bool {
    #[cfg(windows)]
    return windows_impl::system_uses_dark_mode();
    #[cfg(not(windows))]
    return unix_impl::system_uses_dark_mode();
}

/// 进程级初始化（仅 Windows 有实际效果）。
pub fn init_process(dark: bool) {
    #[cfg(windows)]
    windows_impl::init_process(dark);
    #[cfg(not(windows))]
    let _ = dark; // Slint 自动处理
}

// 以下函数仅在 Windows 下有意义；非 Windows 编译为空操作。
// 调用方通过 `#[cfg(windows)]` 仅在 Windows 下调用带 HWND 参数的版本。

#[cfg(windows)]
pub use windows_impl::{apply_control, apply_named, apply_plain, apply_window};
