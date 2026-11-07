// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 自绘静态标签。
//!
//! Windows 主题化 STATIC 控件会忽略 `WM_CTLCOLORSTATIC` 的文字颜色，暗色
//! 主题下仍是黑字。这里用轻量自绘窗口保证标签文字随主题变化。

use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect, GetSysColor,
    SelectObject, SetBkMode, SetTextColor, COLOR_WINDOWTEXT, DT_LEFT, DT_NOPREFIX, HGDIOBJ,
    TRANSPARENT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, GetParent, GetWindowTextLengthW,
    GetWindowTextW, SendMessageW, SetWindowLongPtrW, CREATESTRUCTW, GWLP_USERDATA, WM_CREATE,
    WM_ERASEBKGND, WM_GETFONT, WM_NCDESTROY, WM_PAINT, WM_SETFONT, WM_SETTEXT, WS_CHILD,
    WS_VISIBLE,
};

use crate::text::wide;

/// 标签窗口类名。
pub const LABEL_CLASS: &str = "UsbscanLabel";

static DARK: AtomicBool = AtomicBool::new(false);

/// 创建自绘标签。
pub fn create(parent: HWND, hinstance: HINSTANCE, text: &str) -> HWND {
    let class = wide(LABEL_CLASS);
    let text = wide(text);
    // SAFETY: 标准窗口创建。
    unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            text.as_ptr(),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            0,
            0,
            parent,
            ptr::null_mut(),
            hinstance,
            ptr::null(),
        )
    }
}

/// 设置标签主题并重绘。
pub fn set_dark(hwnd: HWND, dark: bool) {
    DARK.store(dark, Ordering::Relaxed);
    if !hwnd.is_null() {
        // SAFETY: hwnd 是本模块创建的标签窗口。
        unsafe {
            windows_sys::Win32::Graphics::Gdi::InvalidateRect(hwnd, ptr::null(), 1);
        }
    }
}

/// 标签窗口过程。
///
/// # Safety
///
/// `hwnd` 必须是本模块创建的标签窗口；消息参数遵循 Win32 窗口过程契约。
pub unsafe extern "system" fn wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: 所有句柄与结构体均按 Win32 契约使用。
    unsafe {
        match msg {
            WM_CREATE => {
                let cs = lparam as *const CREATESTRUCTW;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*cs).lpCreateParams as isize);
                0
            }
            WM_PAINT => {
                let mut ps: windows_sys::Win32::Graphics::Gdi::PAINTSTRUCT = std::mem::zeroed();
                let hdc = BeginPaint(hwnd, &mut ps);
                let mut rc: RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut rc);
                let dark = DARK.load(Ordering::Relaxed);
                let (bg, fg) = if dark {
                    (rgb(0x1c, 0x1c, 0x1e), rgb(0xf5, 0xf5, 0xf7))
                } else {
                    (rgb(0xec, 0xec, 0xf0), GetSysColor(COLOR_WINDOWTEXT))
                };
                let brush = CreateSolidBrush(bg);
                FillRect(hdc, &rc, brush);
                DeleteObject(brush as HGDIOBJ);

                let parent = GetParent(hwnd);
                let font = SendMessageW(parent, WM_GETFONT, 0, 0) as HGDIOBJ;
                let old_font = if font.is_null() {
                    ptr::null_mut()
                } else {
                    SelectObject(hdc, font)
                };
                SetBkMode(hdc, TRANSPARENT as i32);
                SetTextColor(hdc, fg);

                let len = GetWindowTextLengthW(hwnd);
                let mut text = vec![0u16; len.max(0) as usize + 1];
                let copied = GetWindowTextW(hwnd, text.as_mut_ptr(), text.len() as i32);
                let _ = copied;
                let text = wide(&String::from_utf16_lossy(
                    &text[..usize::try_from(copied.max(0)).unwrap_or(0)],
                ));
                DrawTextW(
                    hdc,
                    text.as_ptr(),
                    i32::try_from(text.len().saturating_sub(1)).unwrap_or(0),
                    &mut rc,
                    DT_LEFT | 0x0010 | DT_NOPREFIX, // DT_WORDBREAK
                );
                if !old_font.is_null() {
                    SelectObject(hdc, old_font);
                }
                EndPaint(hwnd, &ps);
                0
            }
            WM_ERASEBKGND => 1,
            WM_SETTEXT => {
                let result = DefWindowProcW(hwnd, msg, wparam, lparam);
                windows_sys::Win32::Graphics::Gdi::InvalidateRect(hwnd, ptr::null(), 1);
                result
            }
            WM_SETFONT => {
                // 标签始终使用父窗口字体，忽略单独设置。
                windows_sys::Win32::Graphics::Gdi::InvalidateRect(hwnd, ptr::null(), 1);
                0
            }
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

fn rgb(r: u8, g: u8, b: u8) -> u32 {
    u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16)
}
