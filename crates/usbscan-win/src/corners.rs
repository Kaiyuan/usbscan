// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 四角点手动透视校正窗口。
//!
//! 这是一个原生 Win32 模态窗口：图片和四角叠加层自行绘制，按钮使用标准
//! `BUTTON` 控件。用户拖动四个角点后点“应用”，返回图片坐标中的四个点。

use std::ptr;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreatePen, CreateSolidBrush,
    DeleteDC, DeleteObject, Ellipse, EndPaint, FillRect, GetStockObject, InvalidateRect, LineTo,
    MoveToEx, SelectObject, SetStretchBltMode, StretchDIBits, UpdateWindow, BITMAPINFO,
    BITMAPINFOHEADER, COLORONCOLOR, DIB_RGB_COLORS, HDC, HGDIOBJ, SRCCOPY,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, ReleaseCapture, SetCapture, VK_ESCAPE, VK_RETURN,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    GetWindowLongPtrW, GetWindowRect, IsWindow, MessageBoxW, SendMessageW, SetForegroundWindow,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage, CREATESTRUCTW, GWLP_USERDATA,
    MB_ICONWARNING, MB_OK, MSG, SWP_NOACTIVATE, SWP_NOZORDER, SW_SHOW, WM_CLOSE, WM_COMMAND,
    WM_CREATE, WM_DESTROY, WM_ERASEBKGND, WM_GETFONT, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_NCDESTROY, WM_PAINT, WM_SETFONT, WM_SIZE, WS_CAPTION, WS_CLIPCHILDREN,
    WS_EX_DLGMODALFRAME, WS_POPUP, WS_SYSMENU,
};

use crate::controls;
use crate::dpi;
use crate::text::wide;

/// 窗口类名，由主程序注册。
pub const CORNER_CLASS: &str = "UsbscanCornerWindow";

const CMD_APPLY: usize = 3001;
const CMD_RESET: usize = 3002;
const CMD_CANCEL: usize = 3003;
const HANDLE_RADIUS: i32 = 7;

struct CornerState {
    parent: HWND,
    dark: bool,
    image_width: u32,
    image_height: u32,
    bgra: Vec<u8>,
    initial: [(f64, f64); 4],
    corners: [(f64, f64); 4],
    drag: Option<usize>,
    buttons: [HWND; 3],
    result: Option<[(f64, f64); 4]>,
}

/// 打开模态四角点窗口，返回图片坐标中的四个角点。
pub fn edit_corners(
    parent: HWND,
    width: u32,
    height: u32,
    bgra: &[u8],
    initial: [(f64, f64); 4],
    dark: bool,
) -> Option<[(f64, f64); 4]> {
    let state = Box::new(CornerState {
        parent,
        dark,
        image_width: width,
        image_height: height,
        bgra: bgra.to_vec(),
        initial,
        corners: initial,
        drag: None,
        buttons: [ptr::null_mut(); 3],
        result: None,
    });
    let state_ptr = Box::into_raw(state);

    // 在父窗口上居中，大小约为父窗口的 85%。
    let mut parent_rect: RECT = unsafe { std::mem::zeroed() };
    // SAFETY: 父窗口有效。
    unsafe {
        GetWindowRect(parent, &mut parent_rect);
    }
    let parent_width = parent_rect.right - parent_rect.left;
    let parent_height = parent_rect.bottom - parent_rect.top;
    let width = (parent_width * 85 / 100).max(640);
    let height = (parent_height * 85 / 100).max(480);
    let x = parent_rect.left + (parent_width - width) / 2;
    let y = parent_rect.top + (parent_height - height) / 2;

    let hinstance = unsafe { GetModuleHandleW(ptr::null()) };
    let class = wide(CORNER_CLASS);
    let title = wide("透视校正 — 拖动四个角点");
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
        UpdateWindow(hwnd);
        EnableWindow(parent, 0);
        let mut message: MSG = std::mem::zeroed();
        while IsWindow(hwnd) != 0 {
            let result = GetMessageW(&mut message, ptr::null_mut(), 0, 0);
            if result <= 0 {
                break;
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

/// 四角点窗口的消息处理。
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
                let state = create.lpCreateParams.cast::<CornerState>();
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
                let hinstance = create.hInstance;
                let labels = ["应用", "重置", "取消"];
                let ids = [CMD_APPLY, CMD_RESET, CMD_CANCEL];
                for index in 0..3 {
                    (*state).buttons[index] =
                        controls::create_push_button(hwnd, hinstance, ids[index], labels[index]);
                }
                let font = SendMessageW((*state).parent, WM_GETFONT, 0, 0) as WPARAM;
                if font != 0 {
                    for button in (*state).buttons {
                        if !button.is_null() {
                            SendMessageW(button, WM_SETFONT, font, 1);
                        }
                    }
                }
                layout_buttons(hwnd, state);
                0
            }
            WM_SIZE => {
                let state = state_ptr(hwnd);
                if !state.is_null() {
                    layout_buttons(hwnd, state);
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
                0
            }
            WM_PAINT => {
                paint(hwnd);
                0
            }
            WM_ERASEBKGND => 1,
            WM_LBUTTONDOWN => {
                let state = state_ptr(hwnd);
                if !state.is_null() {
                    let mut rect: RECT = std::mem::zeroed();
                    GetClientRect(hwnd, &mut rect);
                    let display = display_rect(&rect);
                    let mouse = mouse_point(lparam);
                    let mut best = None;
                    let mut best_distance =
                        f64::from(dpi::scale(HANDLE_RADIUS, dpi_for_window(hwnd)) * 2);
                    for (index, corner) in (*state).corners.iter().enumerate() {
                        let point = image_to_display(
                            *corner,
                            &display,
                            (*state).image_width,
                            (*state).image_height,
                        );
                        let dx = (point.0 - f64::from(mouse.x)) as f64;
                        let dy = (point.1 - f64::from(mouse.y)) as f64;
                        let distance = (dx * dx + dy * dy).sqrt();
                        if distance < best_distance {
                            best_distance = distance;
                            best = Some(index);
                        }
                    }
                    if let Some(index) = best {
                        (*state).drag = Some(index);
                        SetCapture(hwnd);
                    }
                }
                0
            }
            WM_MOUSEMOVE => {
                let state = state_ptr(hwnd);
                if !state.is_null() {
                    if let Some(index) = (*state).drag {
                        let mut rect: RECT = std::mem::zeroed();
                        GetClientRect(hwnd, &mut rect);
                        let display = display_rect(&rect);
                        let mouse = mouse_point(lparam);
                        (*state).corners[index] = display_to_image(
                            mouse,
                            &display,
                            (*state).image_width,
                            (*state).image_height,
                        );
                        InvalidateRect(hwnd, ptr::null(), 0);
                    }
                }
                0
            }
            WM_LBUTTONUP => {
                let state = state_ptr(hwnd);
                if !state.is_null() {
                    (*state).drag = None;
                    ReleaseCapture();
                }
                0
            }
            WM_KEYDOWN => {
                if wparam == VK_ESCAPE as usize {
                    DestroyWindow(hwnd);
                } else if wparam == VK_RETURN as usize {
                    apply(hwnd);
                }
                0
            }
            WM_COMMAND => {
                match wparam & 0xFFFF {
                    CMD_APPLY => apply(hwnd),
                    CMD_RESET => {
                        let state = state_ptr(hwnd);
                        if !state.is_null() {
                            (*state).corners = (*state).initial;
                            InvalidateRect(hwnd, ptr::null(), 0);
                        }
                    }
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
            WM_DESTROY => {
                // 模态循环通过 IsWindow 退出，不需要 PostQuitMessage。
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

fn state_ptr(hwnd: HWND) -> *mut CornerState {
    // SAFETY: 读取窗口附加数据。
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut CornerState }
}

fn apply(hwnd: HWND) {
    let state = state_ptr(hwnd);
    if state.is_null() {
        return;
    }
    // SAFETY: state 有效；四边形校验为纯计算。
    unsafe {
        if !valid_quad((*state).corners) {
            let text = wide("四个角点不能重合，且必须构成凸四边形。");
            let title = wide("透视校正");
            MessageBoxW(hwnd, text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONWARNING);
            return;
        }
        (*state).result = Some((*state).corners);
        DestroyWindow(hwnd);
    }
}

fn valid_quad(corners: [(f64, f64); 4]) -> bool {
    let mut area = 0.0;
    let mut sign = 0.0;
    for index in 0..4 {
        let (x1, y1) = corners[index];
        let (x2, y2) = corners[(index + 1) % 4];
        area += x1 * y2 - x2 * y1;
        let (x3, y3) = corners[(index + 2) % 4];
        let cross = (x2 - x1) * (y3 - y2) - (y2 - y1) * (x3 - x2);
        if cross.abs() < 1e-6 {
            return false;
        }
        if sign == 0.0 {
            sign = cross;
        } else if sign.signum() != cross.signum() {
            return false;
        }
    }
    area.abs() > 100.0
}

fn layout_buttons(hwnd: HWND, state: *mut CornerState) {
    // SAFETY: 标准布局计算。
    unsafe {
        let mut rect: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut rect);
        let dpi = dpi_for_window(hwnd);
        let button_width = dpi::scale(88, dpi);
        let button_height = dpi::scale(30, dpi);
        let gap = dpi::scale(10, dpi);
        let total = button_width * 3 + gap * 2;
        let start = (rect.right - total) / 2;
        let y = rect.bottom - button_height - dpi::scale(14, dpi);
        for (index, button) in (*state).buttons.iter().enumerate() {
            if !button.is_null() {
                SetWindowPos(
                    *button,
                    ptr::null_mut(),
                    start + index as i32 * (button_width + gap),
                    y,
                    button_width,
                    button_height,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
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
            windows_sys::Win32::Graphics::Gdi::GetSysColor(
                windows_sys::Win32::Graphics::Gdi::COLOR_BTNFACE,
            )
        };
        let brush = CreateSolidBrush(background);
        FillRect(memory, &client, brush);
        DeleteObject(brush as HGDIOBJ);

        if !state.is_null() {
            let display = display_rect(&client);
            draw_image(
                memory,
                &display,
                (*state).image_width,
                (*state).image_height,
                &(*state).bgra,
            );
            draw_overlay(
                memory,
                &display,
                (*state).corners,
                (*state).image_width,
                (*state).image_height,
                dark,
            );
        }
        BitBlt(hdc, 0, 0, width, height, memory, 0, 0, SRCCOPY);
        SelectObject(memory, old_bitmap);
        DeleteObject(bitmap as HGDIOBJ);
        DeleteDC(memory);
        EndPaint(hwnd, &ps);
    }
}

fn draw_image(hdc: HDC, display: &RECT, width: u32, height: u32, bgra: &[u8]) {
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
    // SAFETY: 像素缓冲区与宽高一致。
    unsafe {
        SetStretchBltMode(hdc, COLORONCOLOR);
        StretchDIBits(
            hdc,
            display.left,
            display.top,
            display.right - display.left,
            display.bottom - display.top,
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

fn draw_overlay(
    hdc: HDC,
    display: &RECT,
    corners: [(f64, f64); 4],
    image_width: u32,
    image_height: u32,
    dark: bool,
) {
    let color = if dark {
        rgb(0x5a, 0xb0, 0xff)
    } else {
        rgb(0x00, 0x5f, 0xbb)
    };
    let mut points = [(0i32, 0i32); 4];
    for (index, corner) in corners.iter().enumerate() {
        let (x, y) = image_to_display(*corner, display, image_width, image_height);
        points[index] = (x as i32, y as i32);
    }
    // SAFETY: 纯 GDI 绘制，对象成对释放。
    unsafe {
        let pen = CreatePen(0, 2, color); // PS_SOLID
        let old_pen = SelectObject(hdc, pen);
        MoveToEx(hdc, points[0].0, points[0].1, ptr::null_mut());
        for point in points.iter().skip(1) {
            LineTo(hdc, point.0, point.1);
        }
        LineTo(hdc, points[0].0, points[0].1);
        SelectObject(hdc, old_pen);
        DeleteObject(pen as HGDIOBJ);

        let null_brush = GetStockObject(5); // NULL_BRUSH
        let old_brush = SelectObject(hdc, null_brush);
        let handle_pen = CreatePen(0, 2, color);
        let old_handle_pen = SelectObject(hdc, handle_pen);
        let radius = HANDLE_RADIUS;
        for point in points {
            Ellipse(
                hdc,
                point.0 - radius,
                point.1 - radius,
                point.0 + radius,
                point.1 + radius,
            );
        }
        SelectObject(hdc, old_handle_pen);
        SelectObject(hdc, old_brush);
        DeleteObject(handle_pen as HGDIOBJ);
    }
}

fn display_rect(client: &RECT) -> RECT {
    let dpi = dpi::BASE_DPI;
    let margin = dpi::scale(16, dpi);
    let bottom = dpi::scale(64, dpi);
    RECT {
        left: client.left + margin,
        top: client.top + margin,
        right: client.right - margin,
        bottom: client.bottom - bottom,
    }
}

fn image_to_display(
    point: (f64, f64),
    display: &RECT,
    image_width: u32,
    image_height: u32,
) -> (f64, f64) {
    let width = f64::from((display.right - display.left).max(1));
    let height = f64::from((display.bottom - display.top).max(1));
    let image_width = f64::from(image_width.max(1));
    let image_height = f64::from(image_height.max(1));
    let scale = (width / image_width).min(height / image_height);
    let draw_width = image_width * scale;
    let draw_height = image_height * scale;
    let left = f64::from(display.left) + (width - draw_width) / 2.0;
    let top = f64::from(display.top) + (height - draw_height) / 2.0;
    (left + point.0 * scale, top + point.1 * scale)
}

fn display_to_image(
    point: POINT,
    display: &RECT,
    image_width: u32,
    image_height: u32,
) -> (f64, f64) {
    let width = f64::from((display.right - display.left).max(1));
    let height = f64::from((display.bottom - display.top).max(1));
    let image_width = f64::from(image_width.max(1));
    let image_height = f64::from(image_height.max(1));
    let scale = (width / image_width).min(height / image_height);
    let draw_width = image_width * scale;
    let draw_height = image_height * scale;
    let left = f64::from(display.left) + (width - draw_width) / 2.0;
    let top = f64::from(display.top) + (height - draw_height) / 2.0;
    let x = (f64::from(point.x) - left) / scale;
    let y = (f64::from(point.y) - top) / scale;
    (
        x.clamp(0.0, image_width - 1.0),
        y.clamp(0.0, image_height - 1.0),
    )
}

fn mouse_point(lparam: LPARAM) -> POINT {
    POINT {
        x: i32::from((lparam & 0xFFFF) as i16),
        y: i32::from(((lparam >> 16) & 0xFFFF) as i16),
    }
}

fn dpi_for_window(hwnd: HWND) -> u32 {
    // SAFETY: 只查询当前窗口 DPI。
    unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }
}

const fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_convex_quad() {
        let corners = [(10.0, 10.0), (90.0, 12.0), (88.0, 92.0), (12.0, 90.0)];
        assert!(valid_quad(corners));
    }

    #[test]
    fn rejects_collinear_and_tiny_quads() {
        let collinear = [(10.0, 10.0), (20.0, 20.0), (30.0, 30.0), (40.0, 40.0)];
        assert!(!valid_quad(collinear));
        let tiny = [(1.0, 1.0), (2.0, 1.0), (2.0, 2.0), (1.0, 2.0)];
        assert!(!valid_quad(tiny));
    }

    #[test]
    fn display_mapping_roundtrips() {
        let display = RECT {
            left: 0,
            top: 0,
            right: 800,
            bottom: 600,
        };
        for corner in [(0.0, 0.0), (159.0, 0.0), (159.0, 119.0), (0.0, 119.0)] {
            let display_point = image_to_display(corner, &display, 160, 120);
            let back = display_to_image(
                POINT {
                    x: display_point.0.round() as i32,
                    y: display_point.1.round() as i32,
                },
                &display,
                160,
                120,
            );
            assert!((back.0 - corner.0).abs() < 1.5, "x {back:?} vs {corner:?}");
            assert!((back.1 - corner.1).abs() < 1.5, "y {back:?} vs {corner:?}");
        }
    }
}
