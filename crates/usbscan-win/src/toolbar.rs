// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 自绘工具栏。
//!
//! Windows 原生 Toolbar 在暗色主题下仍使用黑色文字。这里用轻量自绘窗口
//! 绘制图标和文字，保证工具栏文字随亮/暗主题变化，并带 macOS 风格圆角悬停。

use std::ptr;

use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect, GetDC,
    GetStockObject, GetTextExtentPoint32W, InvalidateRect, RoundRect, SelectObject, SetBkMode,
    SetTextColor, DT_LEFT, DT_SINGLELINE, DT_VCENTER, HGDIOBJ, NULL_PEN, TRANSPARENT,
};
use windows_sys::Win32::UI::Controls::{ImageList_Draw, HIMAGELIST, ILD_TRANSPARENT};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, GetParent, GetWindowLongPtrW, PostMessageW,
    SendMessageW, SetWindowLongPtrW, CREATESTRUCTW, GWLP_USERDATA, WM_CAPTURECHANGED, WM_COMMAND,
    WM_CREATE, WM_ERASEBKGND, WM_GETFONT, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCDESTROY,
    WM_PAINT, WS_CHILD, WS_VISIBLE,
};

use crate::controls::ToolButton;
use crate::dpi;
use crate::icons::Icon;
use crate::text::wide;

/// 工具栏窗口类名。
pub const TOOLBAR_CLASS: &str = "UsbscanToolbar";

const DARK_BG: u32 = 0x001e_1c1c;
const DARK_TEXT: u32 = 0x00f7_f5f5;
const DARK_HOVER: u32 = 0x0032_2f2f;
const DARK_PRESSED: u32 = 0x003c_3a3a;
const DARK_SEPARATOR: u32 = 0x003c_3a3a;
const LIGHT_BG: u32 = 0x00f7_f2f2;
const LIGHT_TEXT: u32 = 0x001e_1c1c;
const LIGHT_HOVER: u32 = 0x00ea_e5e5;
const LIGHT_PRESSED: u32 = 0x00d6_d1d1;
const LIGHT_SEPARATOR: u32 = 0x00d6_d1d1;

struct ToolEntry {
    id: usize,
    label: String,
    icon: Option<Icon>,
    separator_before: bool,
}

struct ToolbarState {
    entries: Vec<ToolEntry>,
    images: HIMAGELIST,
    dark: bool,
    hover: i32,
    pressed: i32,
}

/// 创建自绘工具栏。
pub fn create(parent: HWND, hinstance: HINSTANCE, buttons: &[ToolButton]) -> HWND {
    let class = wide(TOOLBAR_CLASS);
    let empty = wide("");
    let entries: Vec<ToolEntry> = buttons
        .iter()
        .map(|button| ToolEntry {
            id: button.id,
            label: button.label.to_string(),
            icon: button.icon,
            separator_before: button.separator_before,
        })
        .collect();
    let state = Box::new(ToolbarState {
        entries,
        images: 0,
        dark: false,
        hover: -1,
        pressed: -1,
    });
    let state = Box::into_raw(state);
    // SAFETY: 状态指针在 WM_CREATE 中接管，失败时手动回收。
    unsafe {
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            empty.as_ptr(),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            0,
            dpi::scale(40, dpi::BASE_DPI),
            parent,
            ptr::null_mut(),
            hinstance,
            state.cast(),
        );
        if hwnd.is_null() {
            drop(Box::from_raw(state));
        }
        hwnd
    }
}

/// 更新主题。
pub fn set_dark(hwnd: HWND, dark: bool) {
    let state = state_ptr(hwnd);
    if state.is_null() {
        return;
    }
    // SAFETY: state 有效。
    unsafe {
        (*state).dark = dark;
        InvalidateRect(hwnd, ptr::null(), 1);
    }
}

/// 更新图标列表。
pub fn set_icons(hwnd: HWND, images: HIMAGELIST) {
    let state = state_ptr(hwnd);
    if state.is_null() {
        return;
    }
    // SAFETY: state 有效；图标列表由调用方持有。
    unsafe {
        (*state).images = images;
        InvalidateRect(hwnd, ptr::null(), 1);
    }
}

fn state_ptr(hwnd: HWND) -> *mut ToolbarState {
    // SAFETY: 读取窗口附加数据，未设置时返回 0。
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ToolbarState }
}

fn icon_size(hwnd: HWND) -> i32 {
    dpi::scale(24, dpi::dpi_for_window(hwnd))
}

fn text_color(dark: bool) -> u32 {
    if dark {
        DARK_TEXT
    } else {
        LIGHT_TEXT
    }
}

/// 计算按钮矩形；返回 `(rect, id)`，分隔线没有 id。
fn layout(hwnd: HWND) -> Vec<(RECT, Option<(usize, usize)>)> {
    let state = state_ptr(hwnd);
    if state.is_null() {
        return Vec::new();
    }
    // SAFETY: state 与 DC 有效。
    unsafe {
        let mut client: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut client);
        let height = client.bottom.max(1);
        let parent = GetParent(hwnd);
        let font = SendMessageW(parent, WM_GETFONT, 0, 0) as HGDIOBJ;
        let dc = GetDC(hwnd);
        let old = if font.is_null() {
            ptr::null_mut()
        } else {
            SelectObject(dc, font)
        };
        let icon = icon_size(hwnd);
        let pad = dpi::scale(10, dpi::dpi_for_window(hwnd));
        let mut result = Vec::new();
        let mut x = dpi::scale(4, dpi::dpi_for_window(hwnd));
        for (entry_index, entry) in (*state).entries.iter().enumerate() {
            if entry.separator_before {
                result.push((
                    RECT {
                        left: x,
                        top: dpi::scale(9, dpi::dpi_for_window(hwnd)),
                        right: x + 1,
                        bottom: height - dpi::scale(9, dpi::dpi_for_window(hwnd)),
                    },
                    None,
                ));
                x += dpi::scale(9, dpi::dpi_for_window(hwnd));
            }
            let label = wide(&entry.label);
            let mut size: windows_sys::Win32::Foundation::SIZE = std::mem::zeroed();
            GetTextExtentPoint32W(
                dc,
                label.as_ptr(),
                i32::try_from(label.len().saturating_sub(1)).unwrap_or(0),
                &mut size,
            );
            let width = pad * 2
                + if entry.icon.is_some() {
                    icon + pad / 2
                } else {
                    0
                }
                + size.cx;
            result.push((
                RECT {
                    left: x,
                    top: dpi::scale(3, dpi::dpi_for_window(hwnd)),
                    right: x + width,
                    bottom: height - dpi::scale(3, dpi::dpi_for_window(hwnd)),
                },
                Some((entry.id, entry_index)),
            ));
            x += width;
        }
        if !old.is_null() {
            SelectObject(dc, old);
        }
        windows_sys::Win32::Graphics::Gdi::ReleaseDC(hwnd, dc);
        result
    }
}

fn hit_test(hwnd: HWND, x: i32) -> i32 {
    for (rect, info) in layout(hwnd) {
        if let Some((_, entry_index)) = info {
            if x >= rect.left && x < rect.right {
                return i32::try_from(entry_index).unwrap_or(-1);
            }
        }
    }
    -1
}

/// 工具栏窗口过程。
///
/// # Safety
///
/// `hwnd` 必须是本模块创建的工具栏窗口；消息参数遵循 Win32 窗口过程契约。
#[allow(clippy::too_many_lines)]
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
                let state = (*cs).lpCreateParams as *mut ToolbarState;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
                0
            }
            WM_PAINT => {
                let mut ps: windows_sys::Win32::Graphics::Gdi::PAINTSTRUCT = std::mem::zeroed();
                let hdc = BeginPaint(hwnd, &mut ps);
                let mut client: RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut client);
                let state = state_ptr(hwnd);
                let dark = !state.is_null() && (*state).dark;
                let (bg, fg) = if dark {
                    (DARK_BG, text_color(true))
                } else {
                    (LIGHT_BG, text_color(false))
                };
                let brush = CreateSolidBrush(bg);
                FillRect(hdc, &client, brush);
                DeleteObject(brush as HGDIOBJ);

                let parent = GetParent(hwnd);
                let font = SendMessageW(parent, WM_GETFONT, 0, 0) as HGDIOBJ;
                let old = if font.is_null() {
                    ptr::null_mut()
                } else {
                    SelectObject(hdc, font)
                };
                SetBkMode(hdc, TRANSPARENT as i32);
                SetTextColor(hdc, fg);
                let hover = if state.is_null() { -1 } else { (*state).hover };
                let pressed = if state.is_null() {
                    -1
                } else {
                    (*state).pressed
                };
                let icon = icon_size(hwnd);
                let pad = dpi::scale(10, dpi::dpi_for_window(hwnd));

                for (rect, info) in layout(hwnd) {
                    let Some((_, entry_index)) = info else {
                        let separator = if dark {
                            DARK_SEPARATOR
                        } else {
                            LIGHT_SEPARATOR
                        };
                        let brush = CreateSolidBrush(separator);
                        FillRect(hdc, &rect, brush);
                        DeleteObject(brush as HGDIOBJ);
                        continue;
                    };
                    let index = i32::try_from(entry_index).unwrap_or(-1);
                    if pressed == index || hover == index {
                        let color = if pressed == index {
                            if dark {
                                DARK_PRESSED
                            } else {
                                LIGHT_PRESSED
                            }
                        } else if dark {
                            DARK_HOVER
                        } else {
                            LIGHT_HOVER
                        };
                        let brush = CreateSolidBrush(color);
                        let old_brush = SelectObject(hdc, brush);
                        let old_pen = SelectObject(hdc, GetStockObject(NULL_PEN));
                        RoundRect(hdc, rect.left, rect.top, rect.right, rect.bottom, 8, 8);
                        SelectObject(hdc, old_pen);
                        SelectObject(hdc, old_brush);
                        DeleteObject(brush as HGDIOBJ);
                    }
                    let state = &*state;
                    let entry = &state.entries[entry_index];
                    let mut text_x = rect.left + pad;
                    if let Some(icon_kind) = entry.icon {
                        if state.images != 0 {
                            let top = (client.bottom - icon) / 2;
                            ImageList_Draw(
                                state.images,
                                i32::try_from(icon_kind.index()).unwrap_or(-1),
                                hdc,
                                text_x,
                                top,
                                ILD_TRANSPARENT,
                            );
                        }
                        text_x += icon + pad / 2;
                    }
                    let label = wide(&entry.label);
                    let mut text_rect = rect;
                    text_rect.left = text_x;
                    text_rect.right = rect.right - pad / 2;
                    DrawTextW(
                        hdc,
                        label.as_ptr(),
                        i32::try_from(label.len().saturating_sub(1)).unwrap_or(0),
                        &mut text_rect,
                        DT_LEFT | DT_VCENTER | DT_SINGLELINE | 0x0800, // DT_NOPREFIX
                    );
                }

                if !old.is_null() {
                    SelectObject(hdc, old);
                }
                EndPaint(hwnd, &ps);
                0
            }
            WM_ERASEBKGND => 1,
            WM_MOUSEMOVE => {
                let x = i32::from((lparam & 0xFFFF) as i16);
                let index = hit_test(hwnd, x);
                let state = state_ptr(hwnd);
                if !state.is_null() && (*state).hover != index {
                    (*state).hover = index;
                    InvalidateRect(hwnd, ptr::null(), 1);
                }
                0
            }
            WM_LBUTTONDOWN => {
                let x = i32::from((lparam & 0xFFFF) as i16);
                let index = hit_test(hwnd, x);
                let state = state_ptr(hwnd);
                if !state.is_null() {
                    (*state).pressed = index;
                    SetCapture(hwnd);
                    InvalidateRect(hwnd, ptr::null(), 1);
                }
                0
            }
            WM_LBUTTONUP => {
                let x = i32::from((lparam & 0xFFFF) as i16);
                let index = hit_test(hwnd, x);
                let state = state_ptr(hwnd);
                if !state.is_null() {
                    let pressed = (*state).pressed;
                    (*state).pressed = -1;
                    ReleaseCapture();
                    InvalidateRect(hwnd, ptr::null(), 1);
                    if pressed == index && index >= 0 {
                        let entries = &(*state).entries;
                        if let Some(entry) =
                            entries.get(usize::try_from(index).unwrap_or(usize::MAX))
                        {
                            PostMessageW(GetParent(hwnd), WM_COMMAND, entry.id, 0);
                        }
                    }
                }
                0
            }
            WM_CAPTURECHANGED => {
                let state = state_ptr(hwnd);
                if !state.is_null() {
                    (*state).pressed = -1;
                    InvalidateRect(hwnd, ptr::null(), 1);
                }
                0
            }
            WM_NCDESTROY => {
                let state = state_ptr(hwnd);
                if !state.is_null() {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    drop(Box::from_raw(state));
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
