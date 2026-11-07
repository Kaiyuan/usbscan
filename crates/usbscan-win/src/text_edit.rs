// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! OCR 文本编辑窗口。
//!
//! 原生多行 `EDIT` 控件 + “保存 / 取消”按钮。识别完成后先在这里校对，
//! 再原子写回与页面同名的 `.txt` 文件。

use std::ptr;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetKeyState, VK_CONTROL, VK_ESCAPE, VK_S,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    GetWindowLongPtrW, GetWindowRect, IsWindow, SendMessageW, SetForegroundWindow,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage, CREATESTRUCTW, ES_AUTOHSCROLL,
    ES_AUTOVSCROLL, ES_MULTILINE, ES_WANTRETURN, GWLP_USERDATA, MSG, SWP_NOACTIVATE, SWP_NOZORDER,
    SW_SHOW, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_GETFONT, WM_KEYDOWN, WM_NCDESTROY, WM_SETFONT,
    WM_SIZE, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN, WS_EX_CLIENTEDGE, WS_EX_DLGMODALFRAME,
    WS_HSCROLL, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

use crate::controls;
use crate::dpi;
use crate::text::wide;

/// 窗口类名，由主程序注册。
pub const TEXT_EDIT_CLASS: &str = "UsbscanTextEditWindow";

const CMD_SAVE: usize = 3101;
const CMD_CANCEL: usize = 3102;
const CMD_EDIT: usize = 3100;

struct EditState {
    parent: HWND,
    content: String,
    edit: HWND,
    save: HWND,
    cancel: HWND,
    result: Option<String>,
}

/// 打开模态文本编辑窗口；保存时返回编辑后的文本。
pub fn edit_text(parent: HWND, title: &str, content: &str) -> Option<String> {
    let state = Box::new(EditState {
        parent,
        content: content.to_string(),
        edit: ptr::null_mut(),
        save: ptr::null_mut(),
        cancel: ptr::null_mut(),
        result: None,
    });
    let state_ptr = Box::into_raw(state);

    // SAFETY: 父窗口有效。
    let mut parent_rect: RECT = unsafe { std::mem::zeroed() };
    unsafe {
        GetWindowRect(parent, &mut parent_rect);
    }
    let parent_width = parent_rect.right - parent_rect.left;
    let parent_height = parent_rect.bottom - parent_rect.top;
    let width = (parent_width * 70 / 100).max(560);
    let height = (parent_height * 70 / 100).max(420);
    let x = parent_rect.left + (parent_width - width) / 2;
    let y = parent_rect.top + (parent_height - height) / 2;

    let hinstance = unsafe { GetModuleHandleW(ptr::null()) };
    let class = wide(TEXT_EDIT_CLASS);
    let window_title = wide(title);
    // SAFETY: 标准窗口创建；state_ptr 由模态函数最终回收。
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_DLGMODALFRAME,
            class.as_ptr(),
            window_title.as_ptr(),
            WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN,
            x,
            y,
            width,
            height,
            parent,
            ptr::null_mut(),
            hinstance,
            state_ptr.cast(),
        )
    };
    if hwnd.is_null() {
        // SAFETY: 窗口未创建，state_ptr 仍由本函数独占。
        unsafe { drop(Box::from_raw(state_ptr)) };
        return None;
    }

    // SAFETY: 标准模态消息循环；父窗口在窗口销毁后恢复。
    unsafe {
        ShowWindow(hwnd, SW_SHOW);
        EnableWindow(parent, 0);
        let mut message: MSG = std::mem::zeroed();
        while IsWindow(hwnd) != 0 {
            let result = GetMessageW(&mut message, ptr::null_mut(), 0, 0);
            if result <= 0 {
                break;
            }
            if message.message == WM_KEYDOWN {
                if message.wParam == VK_ESCAPE as usize {
                    DestroyWindow(hwnd);
                    continue;
                }
                if message.wParam == VK_S as usize && GetKeyState(i32::from(VK_CONTROL)) < 0 {
                    save(hwnd);
                    continue;
                }
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        EnableWindow(parent, 1);
        SetForegroundWindow(parent);
        let state = Box::from_raw(state_ptr);
        state.result
    }
}

/// 文本编辑窗口的消息处理。
///
/// # Safety
///
/// 由系统在窗口消息循环中调用；`hwnd` 必须是本窗口类创建的窗口，`lparam`
/// 在 `WM_CREATE` 时指向框架提供的 `CREATESTRUCTW`。
pub unsafe extern "system" fn wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: 标准窗口消息处理；state 由模态函数保证生命周期。
    unsafe {
        match message {
            WM_CREATE => {
                let create = &*(lparam as *const CREATESTRUCTW);
                let state = create.lpCreateParams.cast::<EditState>();
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
                let class = wide("EDIT");
                let content = wide(&(*state).content);
                (*state).edit = CreateWindowExW(
                    WS_EX_CLIENTEDGE,
                    class.as_ptr(),
                    content.as_ptr(),
                    WS_CHILD
                        | WS_VISIBLE
                        | WS_TABSTOP
                        | WS_VSCROLL
                        | WS_HSCROLL
                        | ES_MULTILINE as u32
                        | ES_AUTOVSCROLL as u32
                        | ES_AUTOHSCROLL as u32
                        | ES_WANTRETURN as u32,
                    0,
                    0,
                    0,
                    0,
                    hwnd,
                    CMD_EDIT as usize as _,
                    create.hInstance,
                    ptr::null(),
                );
                (*state).save =
                    controls::create_push_button(hwnd, create.hInstance, CMD_SAVE, "保存");
                (*state).cancel =
                    controls::create_push_button(hwnd, create.hInstance, CMD_CANCEL, "取消");
                let font = SendMessageW((*state).parent, WM_GETFONT, 0, 0) as WPARAM;
                if font != 0 {
                    for control in [(*state).edit, (*state).save, (*state).cancel] {
                        if !control.is_null() {
                            SendMessageW(control, WM_SETFONT, font, 1);
                        }
                    }
                }
                layout(hwnd, state);
                0
            }
            WM_SIZE => {
                let state = state_ptr(hwnd);
                if !state.is_null() {
                    layout(hwnd, state);
                }
                0
            }
            WM_KEYDOWN => {
                if wparam == VK_ESCAPE as usize {
                    DestroyWindow(hwnd);
                }
                0
            }
            WM_COMMAND => {
                match wparam & 0xFFFF {
                    CMD_SAVE => save(hwnd),
                    CMD_CANCEL => {
                        DestroyWindow(hwnd);
                    }
                    _ => {}
                }
                0
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                0
            }
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

fn state_ptr(hwnd: HWND) -> *mut EditState {
    // SAFETY: 读取窗口附加数据。
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut EditState }
}

fn save(hwnd: HWND) {
    let state = state_ptr(hwnd);
    if state.is_null() {
        return;
    }
    // SAFETY: state 有效。
    unsafe {
        (*state).result = Some(controls::get_text((*state).edit));
        DestroyWindow(hwnd);
    }
}

fn layout(hwnd: HWND, state: *mut EditState) {
    // SAFETY: 标准布局计算。
    unsafe {
        let mut rect: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut rect);
        let dpi = dpi::dpi_for_window(hwnd);
        let margin = dpi::scale(12, dpi);
        let button_width = dpi::scale(88, dpi);
        let button_height = dpi::scale(30, dpi);
        let gap = dpi::scale(8, dpi);
        let bottom = rect.bottom - button_height - margin;
        SetWindowPos(
            (*state).edit,
            ptr::null_mut(),
            margin,
            margin,
            (rect.right - margin * 2).max(1),
            (bottom - margin * 2).max(1),
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let save_x = rect.right - margin - button_width * 2 - gap;
        SetWindowPos(
            (*state).save,
            ptr::null_mut(),
            save_x,
            bottom,
            button_width,
            button_height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        SetWindowPos(
            (*state).cancel,
            ptr::null_mut(),
            save_x + button_width + gap,
            bottom,
            button_width,
            button_height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}
