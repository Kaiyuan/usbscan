// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 系统主题探测与深色模式应用。
//!
//! 深色模式依赖 `uxtheme.dll` 的未公开导出（序号 133 / 135），
//! 属于"尽力而为"：探测失败时静默退回浅色，不报错、不阻塞。
//! 参见 `docs/06` 与 ADR-009。

use windows_sys::Win32::Foundation::{HMODULE, HWND};
use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows_sys::Win32::UI::Controls::SetWindowTheme;

use crate::text::wide;

const PERSONALIZE_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
const APPS_USE_LIGHT_THEME: &str = "AppsUseLightTheme";

/// 系统当前是否使用深色主题。
///
/// 读取 `HKCU\...\Personalize\AppsUseLightTheme`；读不到时按浅色处理。
#[must_use]
pub fn system_uses_dark_mode() -> bool {
    let key = wide(PERSONALIZE_KEY);
    let value = wide(APPS_USE_LIGHT_THEME);
    let mut data: u32 = 1;
    let mut size: u32 = 4;

    // SAFETY: data/size 类型与 RRF_RT_REG_DWORD 匹配，指针在调用期间有效。
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

/// 进程级初始化：允许窗口使用深色模式。
///
/// 必须在创建任何窗口之前调用一次。
pub fn init_process(dark: bool) {
    unsafe {
        let module = load_uxtheme();
        if module.is_null() {
            return;
        }
        // PreferredAppMode: 1 = AllowDark, 2 = ForceDark, 3 = ForceLight
        if let Some(set_mode) = resolve::<unsafe extern "system" fn(i32) -> i32>(module, 135) {
            set_mode(if dark { 2 } else { 3 });
        }
    }
}

/// 把深色模式应用到单个窗口（标题栏 + 菜单 + 非客户区）。
pub fn apply_window(hwnd: HWND, dark: bool) {
    unsafe {
        let module = load_uxtheme();
        if !module.is_null() {
            if let Some(set_mode) = resolve::<unsafe extern "system" fn(i32) -> i32>(module, 135) {
                set_mode(if dark { 2 } else { 3 });
            }
            // AllowDarkModeForWindow(HWND, BOOL)
            if let Some(allow) = resolve::<unsafe extern "system" fn(HWND, i32) -> i32>(module, 133)
            {
                allow(hwnd, i32::from(dark));
            }
        }

        let value: i32 = i32::from(dark);
        // 失败无妨：旧版系统不认识该属性。
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            std::ptr::from_ref(&value).cast(),
            u32::try_from(std::mem::size_of::<i32>()).unwrap_or(4),
        );

        // 让非客户区与菜单栏使用与系统一致的深色/浅色主题。
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
    }
}

/// 把深色主题应用到某个标准控件。
pub fn apply_control(hwnd: HWND, dark: bool) {
    let name = if dark {
        wide("DarkMode_Explorer")
    } else {
        wide("Explorer")
    };
    // SAFETY: 字符串在调用期间有效。
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

/// 把指定主题名应用到控件（用于编辑框等需要特殊暗色主题的控件）。
pub fn apply_named(hwnd: HWND, name: &str) {
    let name = wide(name);
    // SAFETY: 字符串在调用期间有效。
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

/// 去掉控件的视觉样式，让 `WM_CTLCOLOREDIT` 的颜色生效。
pub fn apply_plain(hwnd: HWND) {
    // 传一个空格（而不是空串）才会真正禁用该控件的视觉样式。
    let blank = wide(" ");
    // SAFETY: 字符串在调用期间有效。
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
    // 已加载的模块会被引用计数复用，无需释放。
    let name = wide("uxtheme.dll");
    unsafe { LoadLibraryW(name.as_ptr()) }
}

/// 按序号解析未公开导出。
unsafe fn resolve<T: Copy>(module: HMODULE, ordinal: u16) -> Option<T> {
    debug_assert_eq!(std::mem::size_of::<T>(), std::mem::size_of::<*const ()>());
    unsafe {
        let proc = GetProcAddress(module, ordinal as usize as *const u8)?;
        Some(std::mem::transmute_copy::<usize, T>(&(proc as usize)))
    }
}
