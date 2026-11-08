// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 调整当前进程主窗口的 Win32 几何。
//!
//! Slint 的 `Window::set_size` 在窗口刚创建时会被首次布局的首选尺寸覆盖，
//! 因此恢复用户窗口大小时直接调用 `SetWindowPos`，按客户端尺寸换算外框。

use windows_sys::Win32::Foundation::{HWND, LPARAM, RECT};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClientRect, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId,
    IsWindowVisible, LoadIconW, SendMessageW, SetWindowPos, ICON_BIG, ICON_SMALL, SWP_NOACTIVATE,
    SWP_NOZORDER, WM_SETICON,
};

struct Search {
    pid: u32,
    found: HWND,
    best_area: i64,
}

unsafe extern "system" fn find_main_window(hwnd: HWND, lparam: LPARAM) -> i32 {
    // SAFETY: `lparam` 由 `set_client_geometry` 传入，指向生命周期内的 `Search`。
    let search = unsafe { &mut *(lparam as *mut Search) };
    let mut pid = 0u32;
    // SAFETY: 标准 Win32 调用，`pid` 是有效输出指针。
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    // SAFETY: 标准 Win32 调用。
    if pid != search.pid || unsafe { IsWindowVisible(hwnd) } == 0 {
        return 1;
    }
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: 输出结构已初始化。
    unsafe { GetWindowRect(hwnd, &mut rect) };
    let area = i64::from(rect.right - rect.left) * i64::from(rect.bottom - rect.top);
    let mut title = [0u16; 160];
    // SAFETY: 缓冲区长度与指针匹配。
    let length = unsafe { GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32) };
    let text = String::from_utf16_lossy(&title[..length.max(0) as usize]);
    if !text.starts_with("UsbScan") {
        return 1;
    }
    if area > search.best_area {
        search.best_area = area;
        search.found = hwnd;
    }
    1
}

/// 返回当前进程的主窗口句柄（按标题匹配 `usbscan` 与“文档扫描”）。
pub fn main_window() -> HWND {
    let mut search = Search {
        pid: std::process::id(),
        found: std::ptr::null_mut(),
        best_area: 0,
    };
    // SAFETY: 回调只访问 `search`，其地址在调用期间有效。
    unsafe {
        EnumWindows(
            Some(find_main_window),
            std::ptr::from_mut(&mut search) as LPARAM,
        );
    }
    search.found
}

/// 把 exe 内嵌的图标设置为窗口图标。
pub fn set_window_icon() {
    let hwnd = main_window();
    if hwnd.is_null() {
        return;
    }
    // SAFETY: 标准 Win32 调用；资源 ID 1 由构建脚本写入。
    unsafe {
        let module = GetModuleHandleW(std::ptr::null());
        if module.is_null() {
            eprintln!("failed to get current module handle");
            return;
        }
        let icon = LoadIconW(module, std::ptr::without_provenance(1));
        if icon.is_null() {
            eprintln!("failed to load embedded window icon");
        } else {
            SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, icon as isize);
            SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, icon as isize);
        }
    }
}

/// 返回主窗口当前客户端尺寸（物理像素）。
pub fn client_size() -> Option<(u32, u32)> {
    let hwnd = main_window();
    if hwnd.is_null() {
        return None;
    }
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: `hwnd` 有效，输出结构已初始化。
    unsafe {
        if GetClientRect(hwnd, &mut rect) == 0 {
            return None;
        }
    }
    Some((
        u32::try_from(rect.right - rect.left).ok()?,
        u32::try_from(rect.bottom - rect.top).ok()?,
    ))
}

/// 按客户端尺寸设置当前进程主窗口的位置与大小。
///
/// 返回是否找到并调整了窗口；窗口尚未创建/显示时返回 `false`，调用方可重试。
#[must_use]
pub fn set_client_geometry(x: i32, y: i32, client_width: u32, client_height: u32) -> bool {
    let hwnd = main_window();
    if hwnd.is_null() {
        return false;
    }
    let mut window_rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let mut client_rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: `hwnd` 是有效窗口句柄，输出结构均已初始化。
    unsafe {
        GetWindowRect(hwnd, &mut window_rect);
        GetClientRect(hwnd, &mut client_rect);
        let frame_width =
            (window_rect.right - window_rect.left) - (client_rect.right - client_rect.left);
        let frame_height =
            (window_rect.bottom - window_rect.top) - (client_rect.bottom - client_rect.top);
        let target_width = client_width as i32 + frame_width;
        let target_height = client_height as i32 + frame_height;
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            x,
            y,
            target_width,
            target_height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        ) != 0
    }
}
