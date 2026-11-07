// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 打印预览窗口。

use std::path::PathBuf;
use std::ptr;

use usbscan_imaging as imaging;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateSolidBrush, DeleteDC,
    DeleteObject, EndPaint, FillRect, FrameRect, InvalidateRect, SelectObject, SetStretchBltMode,
    StretchDIBits, BITMAPINFO, BITMAPINFOHEADER, COLORONCOLOR, DIB_RGB_COLORS, HDC, HGDIOBJ,
    SRCCOPY,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, VK_ESCAPE, VK_LEFT, VK_RIGHT};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    GetWindowLongPtrW, GetWindowRect, IsWindow, MessageBoxW, SendMessageW, SetForegroundWindow,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage, CREATESTRUCTW, GWLP_USERDATA,
    MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MSG, SWP_NOACTIVATE, SWP_NOZORDER, SW_SHOW, WM_CLOSE,
    WM_COMMAND, WM_CREATE, WM_GETFONT, WM_KEYDOWN, WM_NCDESTROY, WM_PAINT, WM_SETFONT, WM_SIZE,
    WS_CAPTION, WS_CLIPCHILDREN, WS_EX_DLGMODALFRAME, WS_POPUP, WS_SYSMENU,
};

use crate::controls;
use crate::dpi;
use crate::text::wide;

/// 窗口类名，由主程序注册。
pub const PRINT_PREVIEW_CLASS: &str = "UsbscanPrintPreviewWindow";

const CMD_PREV: usize = 3701;
const CMD_NEXT: usize = 3702;
const CMD_PRINT: usize = 3703;
const CMD_CLOSE: usize = 3704;
const PAGE_WIDTH: f64 = 595.276;
const PAGE_HEIGHT: f64 = 841.890;

struct PreviewState {
    parent: HWND,
    dark: bool,
    files: Vec<PathBuf>,
    current: usize,
    image: Option<(u32, u32, Vec<u8>)>,
    error: Option<String>,
    label: HWND,
    buttons: [HWND; 4],
}

/// 打开模态打印预览窗口。
pub fn show(parent: HWND, files: Vec<PathBuf>, dark: bool) {
    if files.is_empty() {
        return;
    }
    let state = Box::new(PreviewState {
        parent,
        dark,
        files,
        current: 0,
        image: None,
        error: None,
        label: ptr::null_mut(),
        buttons: [ptr::null_mut(); 4],
    });
    let state_ptr = Box::into_raw(state);

    // SAFETY: 父窗口有效。
    let mut parent_rect: RECT = unsafe { std::mem::zeroed() };
    unsafe {
        GetWindowRect(parent, &mut parent_rect);
    }
    let parent_width = parent_rect.right - parent_rect.left;
    let parent_height = parent_rect.bottom - parent_rect.top;
    let width = (parent_width * 80 / 100).max(760);
    let height = (parent_height * 88 / 100).max(620);
    let x = parent_rect.left + (parent_width - width) / 2;
    let y = parent_rect.top + (parent_height - height) / 2;
    let hinstance = unsafe { GetModuleHandleW(ptr::null()) };
    let class = wide(PRINT_PREVIEW_CLASS);
    let title = wide("打印预览");
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
        return;
    }

    // SAFETY: 标准模态消息循环；父窗口在窗口销毁后恢复。
    unsafe {
        load_page(state_ptr);
        ShowWindow(hwnd, SW_SHOW);
        EnableWindow(parent, 0);
        let mut message: MSG = std::mem::zeroed();
        while IsWindow(hwnd) != 0 {
            let result = GetMessageW(&mut message, ptr::null_mut(), 0, 0);
            if result <= 0 {
                break;
            }
            if message.message == WM_KEYDOWN {
                match message.wParam {
                    value if value == VK_ESCAPE as usize => {
                        DestroyWindow(hwnd);
                        continue;
                    }
                    value if value == VK_LEFT as usize => {
                        navigate(state_ptr, -1);
                        continue;
                    }
                    value if value == VK_RIGHT as usize => {
                        navigate(state_ptr, 1);
                        continue;
                    }
                    _ => {}
                }
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        EnableWindow(parent, 1);
        SetForegroundWindow(parent);
        drop(Box::from_raw(state_ptr));
    }
}

fn load_page(state: *mut PreviewState) {
    // SAFETY: state 有效。
    unsafe {
        let s = &mut *state;
        s.error = None;
        s.image = None;
        let Some(path) = s.files.get(s.current) else {
            return;
        };
        match imaging::load_bgra(path) {
            Ok(image) => s.image = Some(image),
            Err(err) => s.error = Some(format!("{}：{err}", path.display())),
        }
        controls::set_text(
            s.label,
            &format!("第 {} / {} 页", s.current + 1, s.files.len()),
        );
        if !s.buttons[0].is_null() {
            EnableWindow(s.buttons[0], i32::from(s.current > 0));
        }
        if !s.buttons[1].is_null() {
            EnableWindow(s.buttons[1], i32::from(s.current + 1 < s.files.len()));
        }
    }
}

fn navigate(state: *mut PreviewState, delta: i32) {
    if state.is_null() {
        return;
    }
    // SAFETY: state 有效；范围检查后再加载。
    unsafe {
        let s = &mut *state;
        let target = s.current as i32 + delta;
        if target < 0 || target as usize >= s.files.len() {
            return;
        }
        s.current = target as usize;
        load_page(state);
        let hwnd = find_window(state);
        if !hwnd.is_null() {
            InvalidateRect(hwnd, ptr::null(), 0);
        }
    }
}

// 预览窗口句柄保存在 label 所属的顶层窗口中；通过 GetParent 取得。
fn find_window(state: *mut PreviewState) -> HWND {
    // SAFETY: label 是预览窗口的子控件。
    unsafe {
        let label = (*state).label;
        if label.is_null() {
            return ptr::null_mut();
        }
        windows_sys::Win32::UI::WindowsAndMessaging::GetParent(label)
    }
}

/// 预览窗口的消息处理。
///
/// # Safety
///
/// 由系统在窗口消息循环中调用；`hwnd` 必须是本窗口类创建的窗口，`lparam`
/// 在 `WM_CREATE` 时指向框架提供的 `CREATESTRUCTW`。
#[allow(clippy::too_many_lines)]
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
                let state = create.lpCreateParams.cast::<PreviewState>();
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
                let hinstance = create.hInstance;
                (*state).label = controls::create_label(hwnd, hinstance, "");
                let labels = ["上一页", "下一页", "打印…", "关闭"];
                let ids = [CMD_PREV, CMD_NEXT, CMD_PRINT, CMD_CLOSE];
                for index in 0..4 {
                    (*state).buttons[index] =
                        controls::create_push_button(hwnd, hinstance, ids[index], labels[index]);
                }
                let font = SendMessageW((*state).parent, WM_GETFONT, 0, 0) as WPARAM;
                if font != 0 {
                    for control in [(*state).label].into_iter().chain((*state).buttons) {
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
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
                0
            }
            WM_PAINT => {
                paint(hwnd);
                0
            }
            WM_COMMAND => {
                let state = state_ptr(hwnd);
                if !state.is_null() {
                    match wparam & 0xFFFF {
                        CMD_PREV => {
                            navigate(state, -1);
                            InvalidateRect(hwnd, ptr::null(), 0);
                        }
                        CMD_NEXT => {
                            navigate(state, 1);
                            InvalidateRect(hwnd, ptr::null(), 0);
                        }
                        CMD_PRINT => print(state),
                        CMD_CLOSE => {
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

fn state_ptr(hwnd: HWND) -> *mut PreviewState {
    // SAFETY: 读取窗口附加数据。
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PreviewState }
}

fn print(state: *mut PreviewState) {
    if state.is_null() {
        return;
    }
    // SAFETY: state 有效；文件路径为本地副本。
    let files = unsafe { (*state).files.clone() };
    let hwnd = find_window(state);
    match crate::print::print_files(hwnd, &files) {
        Ok(true) => {
            let body = wide(&format!("已发送 {} 页到打印机。", files.len()));
            let title = wide("打印");
            // SAFETY: 字符串在调用期间有效。
            unsafe {
                MessageBoxW(
                    hwnd,
                    body.as_ptr(),
                    title.as_ptr(),
                    MB_OK | MB_ICONINFORMATION,
                );
            }
        }
        Ok(false) => {}
        Err(err) => {
            let body = wide(&format!("打印失败：{err}"));
            let title = wide("打印");
            // SAFETY: 字符串在调用期间有效。
            unsafe {
                MessageBoxW(hwnd, body.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR);
            }
        }
    }
}

fn layout(hwnd: HWND, state: *mut PreviewState) {
    // SAFETY: 标准布局计算。
    unsafe {
        let mut rect: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut rect);
        let dpi = dpi::dpi_for_window(hwnd);
        let margin = dpi::scale(12, dpi);
        let button_width = dpi::scale(92, dpi);
        let button_height = dpi::scale(30, dpi);
        let gap = dpi::scale(8, dpi);
        let label_height = dpi::scale(22, dpi);
        SetWindowPos(
            (*state).label,
            ptr::null_mut(),
            margin,
            margin,
            (rect.right - margin * 2).max(1),
            label_height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let bottom = rect.bottom - margin - button_height;
        let total = button_width * 4 + gap * 3;
        let start = (rect.right - total) / 2;
        for (index, button) in (*state).buttons.iter().enumerate() {
            SetWindowPos(
                *button,
                ptr::null_mut(),
                start + index as i32 * (button_width + gap),
                bottom,
                button_width,
                button_height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}

fn paint(hwnd: HWND) {
    // SAFETY: BeginPaint/EndPaint 成对；GDI 对象成对释放。
    unsafe {
        let mut ps: windows_sys::Win32::Graphics::Gdi::PAINTSTRUCT = std::mem::zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);
        if hdc.is_null() {
            EndPaint(hwnd, &ps);
            return;
        }
        let mut client: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut client);
        let width = client.right.max(1);
        let height = client.bottom.max(1);
        let memory = CreateCompatibleDC(hdc);
        let bitmap = CreateCompatibleBitmap(hdc, width, height);
        let old_bitmap = SelectObject(memory, bitmap);
        let state = state_ptr(hwnd);
        let dark = !state.is_null() && (*state).dark;
        let background = if dark {
            rgb(0x1e, 0x1f, 0x22)
        } else {
            rgb(0x9a, 0x9a, 0x9a)
        };
        let brush = CreateSolidBrush(background);
        FillRect(memory, &client, brush);
        DeleteObject(brush as HGDIOBJ);

        if !state.is_null() {
            draw_page(memory, &client, state, dark);
        }
        BitBlt(hdc, 0, 0, width, height, memory, 0, 0, SRCCOPY);
        SelectObject(memory, old_bitmap);
        DeleteObject(bitmap as HGDIOBJ);
        DeleteDC(memory);
        EndPaint(hwnd, &ps);
    }
}

fn draw_page(hdc: HDC, client: &RECT, state: *mut PreviewState, dark: bool) {
    // SAFETY: state 有效；只读取图像与错误信息。
    unsafe {
        let s = &*state;
        let dpi = dpi::dpi_for_window(find_window(state));
        let margin = f64::from(dpi::scale(12, dpi));
        let top = margin + f64::from(dpi::scale(22, dpi));
        let bottom = f64::from(client.bottom) - margin - f64::from(dpi::scale(30, dpi)) - margin;
        let available_width = f64::from(client.right) - margin * 2.0;
        let available_height = (bottom - top).max(1.0);
        let scale = (available_width / PAGE_WIDTH)
            .min(available_height / PAGE_HEIGHT)
            .max(0.01);
        let page_width = (PAGE_WIDTH * scale).round() as i32;
        let page_height = (PAGE_HEIGHT * scale).round() as i32;
        let left = ((f64::from(client.right) - f64::from(page_width)) / 2.0).round() as i32;
        let top = (top + (available_height - f64::from(page_height)) / 2.0).round() as i32;
        let page = RECT {
            left,
            top,
            right: left + page_width,
            bottom: top + page_height,
        };
        let white = CreateSolidBrush(rgb(0xff, 0xff, 0xff));
        FillRect(hdc, &page, white);
        DeleteObject(white as HGDIOBJ);
        let border = CreateSolidBrush(rgb(0x70, 0x70, 0x70));
        FrameRect(hdc, &page, border);
        DeleteObject(border as HGDIOBJ);

        if let Some((width, height, data)) = &s.image {
            let inner_margin = (f64::from(page_width) * 0.04).round() as i32;
            let max_width = f64::from((page_width - inner_margin * 2).max(1));
            let max_height = f64::from((page_height - inner_margin * 2).max(1));
            let image_width = f64::from(*width);
            let image_height = f64::from(*height);
            let image_scale = (max_width / image_width).min(max_height / image_height);
            let draw_width = (image_width * image_scale).round() as i32;
            let draw_height = (image_height * image_scale).round() as i32;
            let x = left + (page_width - draw_width) / 2;
            let y = top + (page_height - draw_height) / 2;
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>()).unwrap_or(40),
                biWidth: i32::try_from(*width).unwrap_or(1),
                biHeight: -i32::try_from(*height).unwrap_or(1),
                biPlanes: 1,
                biBitCount: 32,
                ..std::mem::zeroed()
            };
            SetStretchBltMode(hdc, COLORONCOLOR);
            StretchDIBits(
                hdc,
                x,
                y,
                draw_width,
                draw_height,
                0,
                0,
                i32::try_from(*width).unwrap_or(1),
                i32::try_from(*height).unwrap_or(1),
                data.as_ptr().cast(),
                &info,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
        } else if let Some(error) = &s.error {
            let text = wide(error);
            let mut text_rect = RECT {
                left: left + 12,
                top: top + 12,
                right: page.right - 12,
                bottom: page.bottom - 12,
            };
            windows_sys::Win32::Graphics::Gdi::DrawTextW(
                hdc,
                text.as_ptr(),
                -1,
                &mut text_rect,
                windows_sys::Win32::Graphics::Gdi::DT_WORDBREAK,
            );
        }
        let _ = dark;
    }
}

const fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}
