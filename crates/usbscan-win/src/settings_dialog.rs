// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! OCR 设置对话框：语言与 Tesseract 路径。

use std::path::{Path, PathBuf};
use std::ptr;

use usbscan_ocr as ocr;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, VK_ESCAPE};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    GetWindowLongPtrW, GetWindowRect, IsWindow, MessageBoxW, SendMessageW, SetForegroundWindow,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage, CREATESTRUCTW, EN_CHANGE,
    GWLP_USERDATA, MB_ICONWARNING, MB_OK, MSG, SWP_NOACTIVATE, SWP_NOZORDER, SW_SHOW, WM_CLOSE,
    WM_COMMAND, WM_CREATE, WM_GETFONT, WM_NCDESTROY, WM_SETFONT, WM_SIZE, WS_CAPTION,
    WS_CLIPCHILDREN, WS_EX_DLGMODALFRAME, WS_POPUP, WS_SYSMENU,
};

use crate::controls;
use crate::dpi;
use crate::text::wide;

/// 窗口类名，由主程序注册。
pub const SETTINGS_CLASS: &str = "UsbscanSettingsWindow";

const CMD_LANGUAGE: usize = 3400;
const CMD_PATH: usize = 3401;
const CMD_BROWSE: usize = 3402;
const CMD_SAVE: usize = 3403;
const CMD_CANCEL: usize = 3404;
const CMD_TIMER: usize = 3405;
const CMD_BURST: usize = 3406;
const CMD_INTERVAL: usize = 3407;

/// 设置对话框返回的 OCR 与拍摄配置。
pub struct OcrSettings {
    pub language: String,
    pub tesseract_path: Option<PathBuf>,
    pub timer_seconds: u32,
    pub burst_count: u32,
    pub burst_interval_ms: u32,
}

struct SettingsState {
    parent: HWND,
    language_value: String,
    path_value: String,
    timer_value: String,
    burst_value: String,
    interval_value: String,
    labels: [HWND; 5],
    language: HWND,
    path: HWND,
    timer: HWND,
    burst: HWND,
    interval: HWND,
    browse: HWND,
    status: HWND,
    buttons: [HWND; 2],
    result: Option<OcrSettings>,
}

/// 打开模态设置窗口。
pub fn edit_settings(
    parent: HWND,
    language: &str,
    tesseract_path: Option<&Path>,
    timer_seconds: u32,
    burst_count: u32,
    burst_interval_ms: u32,
) -> Option<OcrSettings> {
    let state = Box::new(SettingsState {
        parent,
        language_value: language.to_string(),
        path_value: tesseract_path
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default(),
        timer_value: timer_seconds.to_string(),
        burst_value: burst_count.to_string(),
        interval_value: burst_interval_ms.to_string(),
        labels: [ptr::null_mut(); 5],
        language: ptr::null_mut(),
        path: ptr::null_mut(),
        timer: ptr::null_mut(),
        burst: ptr::null_mut(),
        interval: ptr::null_mut(),
        browse: ptr::null_mut(),
        status: ptr::null_mut(),
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
    let width = dpi::scale(560, dpi).min(parent_width);
    let height = dpi::scale(430, dpi).min(parent_height);
    let x = parent_rect.left + (parent_width - width) / 2;
    let y = parent_rect.top + (parent_height - height) / 2;

    let hinstance = unsafe { GetModuleHandleW(ptr::null()) };
    let class = wide(SETTINGS_CLASS);
    let title = wide("设置 — OCR 与拍摄");
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

/// 设置窗口的消息处理。
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
                let state = create.lpCreateParams.cast::<SettingsState>();
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
                let hinstance = create.hInstance;
                let names = [
                    "OCR 语言",
                    "Tesseract 路径",
                    "定时拍摄秒数",
                    "连拍张数",
                    "连拍间隔 (ms)",
                ];
                for (index, name) in names.iter().enumerate() {
                    (*state).labels[index] = controls::create_label(hwnd, hinstance, name);
                }
                (*state).language = controls::create_edit(hwnd, hinstance, CMD_LANGUAGE);
                (*state).path = controls::create_edit(hwnd, hinstance, CMD_PATH);
                (*state).timer = controls::create_edit(hwnd, hinstance, CMD_TIMER);
                (*state).burst = controls::create_edit(hwnd, hinstance, CMD_BURST);
                (*state).interval = controls::create_edit(hwnd, hinstance, CMD_INTERVAL);
                (*state).browse =
                    controls::create_push_button(hwnd, hinstance, CMD_BROWSE, "浏览…");
                (*state).status = controls::create_label(hwnd, hinstance, "");
                (*state).buttons[0] =
                    controls::create_push_button(hwnd, hinstance, CMD_SAVE, "保存");
                (*state).buttons[1] =
                    controls::create_push_button(hwnd, hinstance, CMD_CANCEL, "取消");

                controls::set_text((*state).language, &(*state).language_value);
                controls::set_text((*state).path, &(*state).path_value);
                controls::set_text((*state).timer, &(*state).timer_value);
                controls::set_text((*state).burst, &(*state).burst_value);
                controls::set_text((*state).interval, &(*state).interval_value);
                let font = SendMessageW((*state).parent, WM_GETFONT, 0, 0) as WPARAM;
                if font != 0 {
                    for control in [
                        (*state).labels[0],
                        (*state).labels[1],
                        (*state).labels[2],
                        (*state).labels[3],
                        (*state).labels[4],
                        (*state).language,
                        (*state).path,
                        (*state).timer,
                        (*state).burst,
                        (*state).interval,
                        (*state).browse,
                        (*state).status,
                        (*state).buttons[0],
                        (*state).buttons[1],
                    ] {
                        if !control.is_null() {
                            SendMessageW(control, WM_SETFONT, font, 1);
                        }
                    }
                }
                layout(hwnd, state);
                refresh_status(state);
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
                    let id = wparam & 0xFFFF;
                    let code = ((wparam >> 16) & 0xFFFF) as u32;
                    if (id == CMD_LANGUAGE || id == CMD_PATH) && code == EN_CHANGE {
                        refresh_status(state);
                    } else {
                        match id {
                            CMD_BROWSE => browse(hwnd, state),
                            CMD_SAVE => save(hwnd, state),
                            CMD_CANCEL => {
                                DestroyWindow(hwnd);
                            }
                            _ => {}
                        }
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

fn state_ptr(hwnd: HWND) -> *mut SettingsState {
    // SAFETY: 读取窗口附加数据。
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SettingsState }
}

fn refresh_status(state: *mut SettingsState) {
    // SAFETY: state 与控件句柄有效。
    unsafe {
        let configured = controls::get_text((*state).path);
        let configured = configured.trim();
        let path = if configured.is_empty() {
            None
        } else {
            Some(Path::new(configured))
        };
        let text = match ocr::find_tesseract(path) {
            Some(found) => format!("已找到：{}", found.display()),
            None if configured.is_empty() => {
                "未指定路径，将在 PATH 和常见安装目录中查找".to_string()
            }
            None => "未找到该文件，将回退到 PATH 搜索".to_string(),
        };
        controls::set_text((*state).status, &text);
    }
}

fn browse(hwnd: HWND, state: *mut SettingsState) {
    let filter: Vec<u16> =
        "Tesseract (tesseract.exe)\0tesseract.exe\0可执行文件 (*.exe)\0*.exe\0所有文件\0*.*\0\0"
            .encode_utf16()
            .collect();
    let title = wide("选择 tesseract.exe");
    let mut buffer = vec![0u16; 1024];
    // SAFETY: 所有指针指向调用期间存活的缓冲区。
    unsafe {
        let mut dialog: OPENFILENAMEW = std::mem::zeroed();
        dialog.lStructSize = u32::try_from(std::mem::size_of::<OPENFILENAMEW>()).unwrap_or(0);
        dialog.hwndOwner = hwnd;
        dialog.lpstrFilter = filter.as_ptr();
        dialog.lpstrFile = buffer.as_mut_ptr();
        dialog.nMaxFile = u32::try_from(buffer.len()).unwrap_or(1024);
        dialog.lpstrTitle = title.as_ptr();
        dialog.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_EXPLORER;
        if GetOpenFileNameW(&mut dialog) == 0 {
            return;
        }
        let len = buffer.iter().position(|&ch| ch == 0).unwrap_or(0);
        let path = String::from_utf16_lossy(&buffer[..len]);
        controls::set_text((*state).path, &path);
        refresh_status(state);
    }
}

fn save(hwnd: HWND, state: *mut SettingsState) {
    // SAFETY: state 与控件句柄有效。
    unsafe {
        let language = controls::get_text((*state).language).trim().to_string();
        let language = if language.is_empty() {
            ocr::DEFAULT_LANGUAGE.to_string()
        } else {
            language
        };
        if let Err(err) = ocr::validate_language(&language) {
            let message = err.to_string();
            let title = wide("设置 — OCR");
            let body = wide(&message);
            MessageBoxW(hwnd, body.as_ptr(), title.as_ptr(), MB_OK | MB_ICONWARNING);
            return;
        }
        let path = controls::get_text((*state).path);
        let path = path.trim();
        let Some(timer_seconds) = parse_number(hwnd, (*state).timer, "定时拍摄秒数", 1, 30)
        else {
            return;
        };
        let Some(burst_count) = parse_number(hwnd, (*state).burst, "连拍张数", 2, 20) else {
            return;
        };
        let Some(burst_interval_ms) = parse_number(hwnd, (*state).interval, "连拍间隔", 200, 5000)
        else {
            return;
        };
        (*state).result = Some(OcrSettings {
            language,
            tesseract_path: if path.is_empty() {
                None
            } else {
                Some(PathBuf::from(path))
            },
            timer_seconds,
            burst_count,
            burst_interval_ms,
        });
        DestroyWindow(hwnd);
    }
}

fn parse_number(hwnd: HWND, control: HWND, name: &str, min: u32, max: u32) -> Option<u32> {
    // SAFETY: control 为设置窗口内的编辑框。
    let text = controls::get_text(control);
    let Ok(value) = text.trim().parse::<u32>() else {
        warn(hwnd, &format!("{name} 请输入 {min}–{max} 之间的整数。"));
        return None;
    };
    if !(min..=max).contains(&value) {
        warn(hwnd, &format!("{name} 必须在 {min}–{max} 之间。"));
        return None;
    }
    Some(value)
}

fn warn(hwnd: HWND, message: &str) {
    let title = wide("设置");
    let body = wide(message);
    // SAFETY: 字符串在调用期间有效。
    unsafe {
        MessageBoxW(hwnd, body.as_ptr(), title.as_ptr(), MB_OK | MB_ICONWARNING);
    }
}

#[allow(clippy::too_many_lines)]
fn layout(hwnd: HWND, state: *mut SettingsState) {
    // SAFETY: 标准布局计算。
    unsafe {
        let mut rect: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut rect);
        let dpi = dpi::dpi_for_window(hwnd);
        let margin = dpi::scale(16, dpi);
        let label_width = dpi::scale(112, dpi);
        let row = dpi::scale(24, dpi);
        let gap = dpi::scale(12, dpi);
        let control_left = margin + label_width;
        let control_width = (rect.right - margin - control_left).max(1);
        let browse_width = dpi::scale(76, dpi);
        let rows = [
            margin,
            margin + row + gap,
            margin + (row + gap) * 2,
            margin + (row + gap) * 3,
            margin + (row + gap) * 4,
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
        SetWindowPos(
            (*state).language,
            ptr::null_mut(),
            control_left,
            rows[0],
            control_width,
            row,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        SetWindowPos(
            (*state).path,
            ptr::null_mut(),
            control_left,
            rows[1],
            (control_width - browse_width - dpi::scale(6, dpi)).max(1),
            row,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        SetWindowPos(
            (*state).browse,
            ptr::null_mut(),
            rect.right - margin - browse_width,
            rows[1],
            browse_width,
            row,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        for (control, top) in [
            ((*state).timer, rows[2]),
            ((*state).burst, rows[3]),
            ((*state).interval, rows[4]),
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
        SetWindowPos(
            (*state).status,
            ptr::null_mut(),
            control_left,
            rows[4] + row + dpi::scale(8, dpi),
            (rect.right - margin - control_left).max(1),
            dpi::scale(32, dpi),
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let button_width = dpi::scale(96, dpi);
        let button_height = dpi::scale(30, dpi);
        let bottom = rect.bottom - margin - button_height;
        let cancel_x = rect.right - margin - button_width;
        let save_x = cancel_x - button_width - gap;
        SetWindowPos(
            (*state).buttons[0],
            ptr::null_mut(),
            save_x,
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
