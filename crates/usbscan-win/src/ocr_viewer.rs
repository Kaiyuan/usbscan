// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! OCR 结果查看/编辑窗口：左侧图片叠加识别行框，右侧可校对文本。

use std::ptr;

use usbscan_ocr::OcrLine;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreatePen, CreateSolidBrush,
    DeleteDC, DeleteObject, EndPaint, FillRect, InvalidateRect, Rectangle, SelectObject,
    SetStretchBltMode, StretchDIBits, BITMAPINFO, BITMAPINFOHEADER, COLORONCOLOR, DIB_RGB_COLORS,
    HDC, HGDIOBJ, SRCCOPY,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, VK_ESCAPE};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    GetWindowLongPtrW, GetWindowRect, IsWindow, SendMessageW, SetForegroundWindow,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage, CREATESTRUCTW, ES_AUTOHSCROLL,
    ES_AUTOVSCROLL, ES_MULTILINE, ES_WANTRETURN, GWLP_USERDATA, MSG, SWP_NOACTIVATE, SWP_NOZORDER,
    SW_SHOW, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_GETFONT, WM_KEYDOWN, WM_LBUTTONDOWN, WM_NCDESTROY,
    WM_PAINT, WM_SETFONT, WM_SIZE, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN, WS_EX_CLIENTEDGE,
    WS_EX_DLGMODALFRAME, WS_HSCROLL, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

use crate::controls;
use crate::dpi;
use crate::text::wide;

/// 窗口类名，由主程序注册。
pub const OCR_VIEWER_CLASS: &str = "UsbscanOcrViewerWindow";

const CMD_SAVE: usize = 3601;
const CMD_CANCEL: usize = 3602;
const CMD_EDIT: usize = 3600;

struct ViewerState {
    parent: HWND,
    dark: bool,
    image_width: u32,
    image_height: u32,
    bgra: Vec<u8>,
    lines: Vec<OcrLine>,
    text: HWND,
    hint: HWND,
    buttons: [HWND; 2],
    selected: Option<usize>,
    result: Option<String>,
}

/// 打开模态 OCR 结果窗口；保存时返回编辑后的文本。
#[allow(clippy::too_many_arguments)]
pub fn edit_result(
    parent: HWND,
    language: &str,
    width: u32,
    height: u32,
    bgra: &[u8],
    lines: &[OcrLine],
    text: &str,
    dark: bool,
) -> Option<String> {
    let state = Box::new(ViewerState {
        parent,
        dark,
        image_width: width,
        image_height: height,
        bgra: bgra.to_vec(),
        lines: lines.to_vec(),
        text: ptr::null_mut(),
        hint: ptr::null_mut(),
        buttons: [ptr::null_mut(); 2],
        selected: None,
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
    let width = (parent_width * 88 / 100).max(900);
    let height = (parent_height * 85 / 100).max(620);
    let x = parent_rect.left + (parent_width - width) / 2;
    let y = parent_rect.top + (parent_height - height) / 2;

    let hinstance = unsafe { GetModuleHandleW(ptr::null()) };
    let class = wide(OCR_VIEWER_CLASS);
    let title = wide(&format!("OCR 结果 — {language}"));
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
        let state = &mut *state_ptr;
        controls::set_text(state.text, text);
        let mut message: MSG = std::mem::zeroed();
        while IsWindow(hwnd) != 0 {
            let result = GetMessageW(&mut message, ptr::null_mut(), 0, 0);
            if result <= 0 {
                break;
            }
            if message.message == WM_KEYDOWN && message.wParam == VK_ESCAPE as usize {
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

/// OCR 结果窗口的消息处理。
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
                let state = create.lpCreateParams.cast::<ViewerState>();
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
                let hinstance = create.hInstance;
                let class = wide("EDIT");
                let empty = wide("");
                (*state).text = CreateWindowExW(
                    WS_EX_CLIENTEDGE,
                    class.as_ptr(),
                    empty.as_ptr(),
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
                    hinstance,
                    ptr::null(),
                );
                let hint = format!("共 {} 行 · 点击左侧识别框可定位", (*state).lines.len());
                (*state).hint = controls::create_label(hwnd, hinstance, &hint);
                (*state).buttons[0] =
                    controls::create_push_button(hwnd, hinstance, CMD_SAVE, "保存");
                (*state).buttons[1] =
                    controls::create_push_button(hwnd, hinstance, CMD_CANCEL, "取消");
                let font = SendMessageW((*state).parent, WM_GETFONT, 0, 0) as WPARAM;
                if font != 0 {
                    for control in [
                        (*state).text,
                        (*state).hint,
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
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
                0
            }
            WM_PAINT => {
                paint(hwnd);
                0
            }
            WM_LBUTTONDOWN => {
                let state = state_ptr(hwnd);
                if !state.is_null() {
                    let view = image_view(hwnd);
                    let point = view.to_image(mouse_point(lparam));
                    if point.0 >= 0.0 && point.1 >= 0.0 {
                        (*state).selected = (*state).lines.iter().position(|line| {
                            point.0 >= f64::from(line.left)
                                && point.0 <= f64::from(line.left + line.width)
                                && point.1 >= f64::from(line.top)
                                && point.1 <= f64::from(line.top + line.height)
                        });
                        InvalidateRect(hwnd, ptr::null(), 0);
                    }
                }
                0
            }
            WM_COMMAND => {
                let state = state_ptr(hwnd);
                if !state.is_null() {
                    match wparam & 0xFFFF {
                        CMD_SAVE => {
                            (*state).result = Some(controls::get_text((*state).text));
                            DestroyWindow(hwnd);
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

fn state_ptr(hwnd: HWND) -> *mut ViewerState {
    // SAFETY: 读取窗口附加数据。
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ViewerState }
}

fn layout(hwnd: HWND, state: *mut ViewerState) {
    // SAFETY: 标准布局计算。
    unsafe {
        let mut rect: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut rect);
        let dpi = dpi::dpi_for_window(hwnd);
        let margin = dpi::scale(12, dpi);
        let gap = dpi::scale(12, dpi);
        let button_height = dpi::scale(30, dpi);
        let hint_height = dpi::scale(22, dpi);
        let bottom = rect.bottom - margin - button_height;
        let content_top = margin + hint_height;
        let content_height = (bottom - gap - content_top).max(1);
        let content_width = (rect.right - margin * 2).max(1);
        let image_width = (content_width * 55 / 100).max(1);
        let text_x = margin + image_width + gap;
        let text_width = (rect.right - margin - text_x).max(1);

        SetWindowPos(
            (*state).hint,
            ptr::null_mut(),
            text_x,
            margin,
            text_width,
            hint_height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        SetWindowPos(
            (*state).text,
            ptr::null_mut(),
            text_x,
            content_top,
            text_width,
            content_height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let button_width = dpi::scale(96, dpi);
        let cancel_x = rect.right - margin - button_width;
        let save_x = cancel_x - button_width - gap;
        for (button, x) in [
            ((*state).buttons[0], save_x),
            ((*state).buttons[1], cancel_x),
        ] {
            SetWindowPos(
                button,
                ptr::null_mut(),
                x,
                bottom,
                button_width,
                button_height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}

struct View {
    left: f64,
    top: f64,
    scale: f64,
}

fn image_view(hwnd: HWND) -> View {
    // SAFETY: 只查询窗口尺寸与 DPI。
    unsafe {
        let mut client: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut client);
        let dpi = dpi::dpi_for_window(hwnd);
        let margin = f64::from(dpi::scale(12, dpi));
        let gap = f64::from(dpi::scale(12, dpi));
        let button_height = f64::from(dpi::scale(30, dpi));
        let hint_height = f64::from(dpi::scale(22, dpi));
        let content_top = margin + hint_height;
        let bottom = f64::from(client.bottom) - margin - button_height - gap;
        let content_width = f64::from(client.right) - margin * 2.0;
        let image_width = content_width * 0.55;
        let display_height = (bottom - content_top).max(1.0);
        let state = state_ptr(hwnd);
        let (width, height) = if state.is_null() {
            (1.0, 1.0)
        } else {
            (
                f64::from((*state).image_width.max(1)),
                f64::from((*state).image_height.max(1)),
            )
        };
        let scale = (image_width / width).min(display_height / height);
        View {
            left: margin + (image_width - width * scale) / 2.0,
            top: content_top + (display_height - height * scale) / 2.0,
            scale,
        }
    }
}

impl View {
    fn to_display(&self, x: f64, y: f64) -> POINT {
        POINT {
            x: (self.left + x * self.scale).round() as i32,
            y: (self.top + y * self.scale).round() as i32,
        }
    }

    fn to_image(&self, point: POINT) -> (f64, f64) {
        (
            (f64::from(point.x) - self.left) / self.scale,
            (f64::from(point.y) - self.top) / self.scale,
        )
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
            windows_sys::Win32::Graphics::Gdi::GetSysColor(
                windows_sys::Win32::Graphics::Gdi::COLOR_BTNFACE,
            )
        };
        let brush = CreateSolidBrush(background);
        FillRect(memory, &client, brush);
        DeleteObject(brush as HGDIOBJ);

        if !state.is_null() {
            let view = image_view(hwnd);
            draw_image(
                memory,
                &view,
                (*state).image_width,
                (*state).image_height,
                &(*state).bgra,
            );
            for (index, line) in (*state).lines.iter().enumerate() {
                let selected = (*state).selected == Some(index);
                let color = if selected {
                    rgb(0xff, 0x8a, 0x00)
                } else if dark {
                    rgb(0x5a, 0xb0, 0xff)
                } else {
                    rgb(0x00, 0x5f, 0xbb)
                };
                let top_left = view.to_display(f64::from(line.left), f64::from(line.top));
                let bottom_right = view.to_display(
                    f64::from(line.left + line.width),
                    f64::from(line.top + line.height),
                );
                let pen = CreatePen(0, if selected { 3 } else { 1 }, color);
                let old_pen = SelectObject(memory, pen);
                let null_brush = windows_sys::Win32::Graphics::Gdi::GetStockObject(5);
                let old_brush = SelectObject(memory, null_brush);
                Rectangle(
                    memory,
                    top_left.x,
                    top_left.y,
                    bottom_right.x,
                    bottom_right.y,
                );
                SelectObject(memory, old_brush);
                SelectObject(memory, old_pen);
                DeleteObject(pen as HGDIOBJ);
            }
        }
        BitBlt(hdc, 0, 0, width, height, memory, 0, 0, SRCCOPY);
        SelectObject(memory, old_bitmap);
        DeleteObject(bitmap as HGDIOBJ);
        DeleteDC(memory);
        EndPaint(hwnd, &ps);
    }
}

fn draw_image(hdc: HDC, view: &View, width: u32, height: u32, bgra: &[u8]) {
    if width == 0 || height == 0 {
        return;
    }
    // SAFETY: 全零 BITMAPINFO 是合法 POD。
    let mut info: BITMAPINFO = unsafe { std::mem::zeroed() };
    info.bmiHeader = BITMAPINFOHEADER {
        biSize: u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>()).unwrap_or(40),
        biWidth: i32::try_from(width).unwrap_or(1),
        biHeight: -i32::try_from(height).unwrap_or(1),
        biPlanes: 1,
        biBitCount: 32,
        ..unsafe { std::mem::zeroed() }
    };
    let left = view.left.round() as i32;
    let top = view.top.round() as i32;
    let draw_width = (f64::from(width) * view.scale).round() as i32;
    let draw_height = (f64::from(height) * view.scale).round() as i32;
    // SAFETY: 像素缓冲区与宽高一致。
    unsafe {
        SetStretchBltMode(hdc, COLORONCOLOR);
        StretchDIBits(
            hdc,
            left,
            top,
            draw_width,
            draw_height,
            0,
            0,
            i32::try_from(width).unwrap_or(1),
            i32::try_from(height).unwrap_or(1),
            bgra.as_ptr().cast(),
            &info,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
    }
}

fn mouse_point(lparam: LPARAM) -> POINT {
    POINT {
        x: i32::from((lparam & 0xFFFF) as i16),
        y: i32::from(((lparam >> 16) & 0xFFFF) as i16),
    }
}

const fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}
