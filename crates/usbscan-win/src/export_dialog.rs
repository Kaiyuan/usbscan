// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! PDF 导出选项对话框。

use std::ptr;

use usbscan_export::{FitMode, Orientation, PageSize, PdfOptions};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, VK_ESCAPE};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    GetWindowLongPtrW, GetWindowRect, IsWindow, MessageBoxW, SendMessageW, SetForegroundWindow,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage, CREATESTRUCTW, GWLP_USERDATA,
    MB_ICONWARNING, MB_OK, MSG, SWP_NOACTIVATE, SWP_NOZORDER, SW_SHOW, WM_CLOSE, WM_COMMAND,
    WM_CREATE, WM_GETFONT, WM_NCDESTROY, WM_SETFONT, WM_SIZE, WS_CAPTION, WS_CLIPCHILDREN,
    WS_EX_DLGMODALFRAME, WS_POPUP, WS_SYSMENU,
};

use crate::controls;
use crate::dpi;
use crate::text::wide;

/// 窗口类名，由主程序注册。
pub const EXPORT_CLASS: &str = "UsbscanExportWindow";

const CMD_APPLY: usize = 3301;
const CMD_CANCEL: usize = 3302;

struct ExportState {
    parent: HWND,
    labels: [HWND; 4],
    page_size: HWND,
    orientation: HWND,
    margin: HWND,
    fit: HWND,
    buttons: [HWND; 2],
    result: Option<PdfOptions>,
}

/// 打开模态 PDF 选项窗口。
pub fn edit_options(parent: HWND) -> Option<PdfOptions> {
    let state = Box::new(ExportState {
        parent,
        labels: [ptr::null_mut(); 4],
        page_size: ptr::null_mut(),
        orientation: ptr::null_mut(),
        margin: ptr::null_mut(),
        fit: ptr::null_mut(),
        buttons: [ptr::null_mut(); 2],
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
    let dpi = dpi::dpi_for_window(parent);
    let width = dpi::scale(460, dpi).min(parent_width);
    let height = dpi::scale(310, dpi).min(parent_height);
    let x = parent_rect.left + (parent_width - width) / 2;
    let y = parent_rect.top + (parent_height - height) / 2;

    let hinstance = unsafe { GetModuleHandleW(ptr::null()) };
    let class = wide(EXPORT_CLASS);
    let title = wide("导出 PDF 选项");
    // SAFETY: 标准窗口创建；state_ptr 由模态函数最终回收。
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_DLGMODALFRAME,
            class.as_ptr(),
            title.as_ptr(),
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
            if message.message == windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN
                && message.wParam == VK_ESCAPE as usize
            {
                DestroyWindow(hwnd);
                continue;
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

/// 导出选项窗口的消息处理。
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
                let state = create.lpCreateParams.cast::<ExportState>();
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
                let hinstance = create.hInstance;
                let names = ["纸张", "方向", "页边距 (mm)", "缩放"];
                for (index, name) in names.iter().enumerate() {
                    (*state).labels[index] = controls::create_label(hwnd, hinstance, name);
                }
                (*state).page_size = controls::create_combo_box(hwnd, hinstance, 3310);
                (*state).orientation = controls::create_combo_box(hwnd, hinstance, 3311);
                (*state).margin = controls::create_edit(hwnd, hinstance, 3312);
                (*state).fit = controls::create_combo_box(hwnd, hinstance, 3313);

                controls::set_combo_items(
                    (*state).page_size,
                    &[
                        "A4".to_string(),
                        "US Letter".to_string(),
                        "跟随图片尺寸".to_string(),
                    ],
                    Some(0),
                );
                controls::set_combo_items(
                    (*state).orientation,
                    &["自动".to_string(), "纵向".to_string(), "横向".to_string()],
                    Some(0),
                );
                controls::set_text((*state).margin, "12.7");
                controls::set_combo_items(
                    (*state).fit,
                    &["适应页面".to_string(), "填满页面".to_string()],
                    Some(0),
                );

                (*state).buttons[0] =
                    controls::create_push_button(hwnd, hinstance, CMD_APPLY, "导出…");
                (*state).buttons[1] =
                    controls::create_push_button(hwnd, hinstance, CMD_CANCEL, "取消");

                let font = SendMessageW((*state).parent, WM_GETFONT, 0, 0) as WPARAM;
                if font != 0 {
                    for label in (*state).labels {
                        if !label.is_null() {
                            SendMessageW(label, WM_SETFONT, font, 1);
                        }
                    }
                    for control in [
                        (*state).page_size,
                        (*state).orientation,
                        (*state).margin,
                        (*state).fit,
                        (*state).buttons[0],
                        (*state).buttons[1],
                    ] {
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
            WM_COMMAND => {
                let state = state_ptr(hwnd);
                if !state.is_null() {
                    match wparam & 0xFFFF {
                        CMD_APPLY => {
                            if let Some(options) = collect(hwnd, state) {
                                (*state).result = Some(options);
                                DestroyWindow(hwnd);
                            }
                        }
                        CMD_CANCEL => {
                            DestroyWindow(hwnd);
                        }
                        _ => {}
                    }
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

fn state_ptr(hwnd: HWND) -> *mut ExportState {
    // SAFETY: 读取窗口附加数据。
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ExportState }
}

fn collect(hwnd: HWND, state: *mut ExportState) -> Option<PdfOptions> {
    // SAFETY: state 与控件句柄有效。
    unsafe {
        let text = controls::get_text((*state).margin).replace(',', ".");
        let margin = match text.trim().parse::<f64>() {
            Ok(value) if value.is_finite() && (0.0..=50.0).contains(&value) => value,
            _ => {
                let body = wide("页边距请输入 0–50 之间的毫米数。");
                let title = wide("导出 PDF 选项");
                MessageBoxW(hwnd, body.as_ptr(), title.as_ptr(), MB_OK | MB_ICONWARNING);
                return None;
            }
        };
        let page_size = match controls::combo_selection((*state).page_size) {
            Some(1) => PageSize::Letter,
            Some(2) => PageSize::FitImage,
            _ => PageSize::A4,
        };
        let orientation = match controls::combo_selection((*state).orientation) {
            Some(1) => Orientation::Portrait,
            Some(2) => Orientation::Landscape,
            _ => Orientation::Auto,
        };
        let fit = match controls::combo_selection((*state).fit) {
            Some(1) => FitMode::Fill,
            _ => FitMode::Contain,
        };
        Some(PdfOptions {
            page_size,
            orientation,
            margin_mm: margin,
            fit,
        })
    }
}

fn layout(hwnd: HWND, state: *mut ExportState) {
    // SAFETY: 标准布局计算。
    unsafe {
        let mut rect: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut rect);
        let dpi = dpi::dpi_for_window(hwnd);
        let margin = dpi::scale(16, dpi);
        let label_width = dpi::scale(92, dpi);
        let row = dpi::scale(26, dpi);
        let gap = dpi::scale(12, dpi);
        let control_left = margin + label_width;
        let control_width = (rect.right - margin - control_left).max(1);
        let rows = [
            margin,
            margin + row + gap,
            margin + (row + gap) * 2,
            margin + (row + gap) * 3,
        ];
        for (index, label) in (*state).labels.iter().enumerate() {
            SetWindowPos(
                *label,
                ptr::null_mut(),
                margin,
                rows[index] + dpi::scale(4, dpi),
                label_width,
                row,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        for (control, top) in [
            ((*state).page_size, rows[0]),
            ((*state).orientation, rows[1]),
            ((*state).margin, rows[2]),
            ((*state).fit, rows[3]),
        ] {
            SetWindowPos(
                control,
                ptr::null_mut(),
                control_left,
                top,
                control_width,
                row,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        let button_width = dpi::scale(96, dpi);
        let button_height = dpi::scale(30, dpi);
        let bottom = rect.bottom - margin - button_height;
        let cancel_x = rect.right - margin - button_width;
        let apply_x = cancel_x - button_width - gap;
        SetWindowPos(
            (*state).buttons[0],
            ptr::null_mut(),
            apply_x,
            bottom,
            button_width,
            button_height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        SetWindowPos(
            (*state).buttons[1],
            ptr::null_mut(),
            cancel_x,
            bottom,
            button_width,
            button_height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}
