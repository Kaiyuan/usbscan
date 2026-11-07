// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 自绘菜单栏。
//!
//! Windows 的 HMENU 菜单栏在系统深色主题下仍按浅色绘制，`SetMenuInfo` 也
//! 不会改变菜单栏本身。这里用一个轻量自绘子窗口代替系统菜单栏，下拉菜单
//! 仍复用原生 HMENU（并做 owner-draw），因此命令、快捷键和勾选状态都不变。

use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect,
    GetStockObject, GetSysColor, GetTextExtentPoint32W, InvalidateRect, RoundRect, SelectObject,
    SetBkMode, SetTextColor, COLOR_BTNTEXT, COLOR_HIGHLIGHTTEXT, DT_CENTER, DT_SINGLELINE,
    DT_VCENTER, HGDIOBJ, NULL_PEN, TRANSPARENT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, GetSubMenu, GetWindowLongPtrW, SendMessageW,
    SetForegroundWindow, SetWindowLongPtrW, TrackPopupMenu, CREATESTRUCTW, GWLP_USERDATA, HMENU,
    TPM_LEFTALIGN, TPM_RIGHTBUTTON, TPM_TOPALIGN, WM_CREATE, WM_DRAWITEM, WM_ERASEBKGND,
    WM_GETFONT, WM_LBUTTONUP, WM_MEASUREITEM, WM_MOUSEMOVE, WM_NCDESTROY, WM_PAINT, WS_CHILD,
    WS_VISIBLE,
};

use crate::dpi;
use crate::text::wide;

/// 菜单栏窗口类名。
pub const MENU_BAR_CLASS: &str = "UsbscanMenuBar";

/// 菜单栏标题，顺序必须与主菜单的顶层顺序一致。
const TITLES: [&str; 6] = ["文件", "编辑", "设备", "视图", "工具", "帮助"];

const DARK_BG: u32 = 0x001e_1c1c;
const DARK_TEXT: u32 = 0x00f7_f5f5;
const DARK_SELECTED: u32 = 0x00ff_840a;

static DARK: AtomicBool = AtomicBool::new(false);

struct MenuBarState {
    menu: HMENU,
    hover: i32,
}

/// 创建菜单栏子窗口；`menu` 是隐藏的主菜单栏句柄。
pub fn create(
    parent: HWND,
    hinstance: windows_sys::Win32::Foundation::HINSTANCE,
    menu: HMENU,
) -> HWND {
    let class = wide(MENU_BAR_CLASS);
    let empty = wide("");
    // SAFETY: 标准窗口创建；menu 通过 WM_CREATE 的 CREATESTRUCT 传入。
    unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            empty.as_ptr(),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            0,
            dpi::scale(28, dpi::BASE_DPI),
            parent,
            ptr::null_mut(),
            hinstance,
            menu as *mut std::ffi::c_void,
        )
    }
}

/// 主题变化时刷新菜单栏。
pub fn set_dark(hwnd: HWND, dark: bool) {
    DARK.store(dark, Ordering::Relaxed);
    if !hwnd.is_null() {
        // SAFETY: hwnd 是本模块创建的菜单栏窗口。
        unsafe {
            InvalidateRect(hwnd, ptr::null(), 1);
        }
    }
}

fn state_ptr(hwnd: HWND) -> *mut MenuBarState {
    // SAFETY: 读取窗口附加数据，未设置时返回 0。
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut MenuBarState }
}

fn item_height(hwnd: HWND) -> i32 {
    dpi::scale(28, dpi::dpi_for_window(hwnd))
}

fn item_padding(hwnd: HWND) -> i32 {
    dpi::scale(14, dpi::dpi_for_window(hwnd))
}

/// 计算每个菜单标题的矩形。
fn item_rects(hwnd: HWND) -> [RECT; 6] {
    let mut rects = [RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    }; 6];
    // SAFETY: 使用屏幕 DC 与父窗口字体测量文字。
    unsafe {
        let parent = windows_sys::Win32::UI::WindowsAndMessaging::GetParent(hwnd);
        let font = SendMessageW(parent, WM_GETFONT, 0, 0) as HGDIOBJ;
        let dc = windows_sys::Win32::Graphics::Gdi::GetDC(hwnd);
        let old = if font.is_null() {
            ptr::null_mut()
        } else {
            SelectObject(dc, font)
        };
        let height = item_height(hwnd);
        let padding = item_padding(hwnd);
        let mut left = 0;
        for (index, title) in TITLES.iter().enumerate() {
            let text = wide(title);
            let mut size: windows_sys::Win32::Foundation::SIZE = std::mem::zeroed();
            GetTextExtentPoint32W(dc, text.as_ptr(), (text.len() - 1) as i32, &mut size);
            let width = size.cx + padding * 2;
            rects[index] = RECT {
                left,
                top: 0,
                right: left + width,
                bottom: height,
            };
            left += width;
        }
        if !old.is_null() {
            SelectObject(dc, old);
        }
        windows_sys::Win32::Graphics::Gdi::ReleaseDC(hwnd, dc);
    }
    rects
}

fn hit_test(hwnd: HWND, x: i32) -> i32 {
    let rects = item_rects(hwnd);
    for (index, rect) in rects.iter().enumerate() {
        if x >= rect.left && x < rect.right {
            return i32::try_from(index).unwrap_or(-1);
        }
    }
    -1
}

fn show_menu(hwnd: HWND, index: usize) {
    let state = state_ptr(hwnd);
    if state.is_null() {
        return;
    }
    // SAFETY: state 有效；菜单句柄由主窗口持有。
    unsafe {
        let state = &mut *state;
        let submenu = GetSubMenu(state.menu, i32::try_from(index).unwrap_or(-1));
        if submenu.is_null() {
            return;
        }
        let rects = item_rects(hwnd);
        let mut point = POINT {
            x: rects[index].left,
            y: rects[index].bottom,
        };
        ClientToScreen(hwnd, &mut point);
        let parent = windows_sys::Win32::UI::WindowsAndMessaging::GetParent(hwnd);
        let _ = SetForegroundWindow(parent);
        TrackPopupMenu(
            submenu,
            TPM_LEFTALIGN | TPM_TOPALIGN | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            parent,
            ptr::null(),
        );
        state.hover = -1;
        InvalidateRect(hwnd, ptr::null(), 1);
    }
}

/// 菜单栏窗口过程。
///
/// # Safety
///
/// `hwnd` 必须是本模块创建的菜单栏窗口；消息参数遵循 Win32 窗口过程契约。
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
                let state = Box::new(MenuBarState {
                    menu: (*cs).lpCreateParams as HMENU,
                    hover: -1,
                });
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
                0
            }
            WM_PAINT => {
                let mut ps: windows_sys::Win32::Graphics::Gdi::PAINTSTRUCT = std::mem::zeroed();
                let hdc = BeginPaint(hwnd, &mut ps);
                let mut client: RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut client);
                let dark = DARK.load(Ordering::Relaxed);
                let (bg, fg) = if dark {
                    (DARK_BG, DARK_TEXT)
                } else {
                    (rgb(0xf2, 0xf2, 0xf7), GetSysColor(COLOR_BTNTEXT))
                };
                let brush = CreateSolidBrush(bg);
                FillRect(hdc, &client, brush);
                DeleteObject(brush as HGDIOBJ);

                let state = state_ptr(hwnd);
                let hover = if state.is_null() { -1 } else { (*state).hover };
                let parent = windows_sys::Win32::UI::WindowsAndMessaging::GetParent(hwnd);
                let font = SendMessageW(parent, WM_GETFONT, 0, 0) as HGDIOBJ;
                let old = if font.is_null() {
                    ptr::null_mut()
                } else {
                    SelectObject(hdc, font)
                };
                SetBkMode(hdc, TRANSPARENT as i32);
                let rects = item_rects(hwnd);
                for (index, title) in TITLES.iter().enumerate() {
                    let mut rect = rects[index];
                    let selected = hover == i32::try_from(index).unwrap_or(-1);
                    if selected {
                        let highlight = if dark {
                            DARK_SELECTED
                        } else {
                            rgb(0x00, 0x7a, 0xff)
                        };
                        let brush = CreateSolidBrush(highlight);
                        let old_brush = SelectObject(hdc, brush);
                        let old_pen = SelectObject(hdc, GetStockObject(NULL_PEN));
                        RoundRect(
                            hdc,
                            rect.left + 4,
                            rect.top + 3,
                            rect.right - 4,
                            rect.bottom - 3,
                            8,
                            8,
                        );
                        SelectObject(hdc, old_pen);
                        SelectObject(hdc, old_brush);
                        DeleteObject(brush as HGDIOBJ);
                    }
                    let color = if selected {
                        if dark {
                            rgb(0xff, 0xff, 0xff)
                        } else {
                            GetSysColor(COLOR_HIGHLIGHTTEXT)
                        }
                    } else {
                        fg
                    };
                    SetTextColor(hdc, color);
                    let text = wide(title);
                    DrawTextW(
                        hdc,
                        text.as_ptr(),
                        (text.len() - 1) as i32,
                        &mut rect,
                        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                    );
                }
                if !old.is_null() {
                    SelectObject(hdc, old);
                }
                EndPaint(hwnd, &ps);
                0
            }
            WM_ERASEBKGND => 1,
            WM_MEASUREITEM => isize::from(crate::menu::measure_item(
                lparam as *mut windows_sys::Win32::UI::Controls::MEASUREITEMSTRUCT,
            )),
            WM_DRAWITEM => {
                let item = lparam as *const windows_sys::Win32::UI::Controls::DRAWITEMSTRUCT;
                isize::from(crate::menu::draw_item(item, DARK.load(Ordering::Relaxed)))
            }
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
            WM_LBUTTONUP => {
                let x = i32::from((lparam & 0xFFFF) as i16);
                let index = hit_test(hwnd, x);
                if index >= 0 {
                    show_menu(hwnd, usize::try_from(index).unwrap_or(0));
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

fn rgb(r: u8, g: u8, b: u8) -> u32 {
    u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16)
}
