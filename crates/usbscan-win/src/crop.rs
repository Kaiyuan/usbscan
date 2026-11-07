// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 手动裁剪窗口。
//!
//! 原生模态窗口：图片按比例显示，拖动四角或框内区域调整裁剪范围，
//! 点“应用”返回以图片像素为单位的裁剪矩形。

use std::ptr;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreatePen, CreateSolidBrush,
    DeleteDC, DeleteObject, Ellipse, EndPaint, FillRect, GetStockObject, InvalidateRect, LineTo,
    MoveToEx, SelectObject, SetStretchBltMode, StretchDIBits, BITMAPINFO, BITMAPINFOHEADER,
    COLORONCOLOR, DIB_RGB_COLORS, HDC, HGDIOBJ, SRCCOPY,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, ReleaseCapture, SetCapture, VK_ESCAPE, VK_RETURN,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    GetWindowLongPtrW, GetWindowRect, IsWindow, SendMessageW, SetForegroundWindow,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage, CREATESTRUCTW, GWLP_USERDATA,
    MSG, SWP_NOACTIVATE, SWP_NOZORDER, SW_SHOW, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_GETFONT,
    WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCDESTROY, WM_PAINT, WM_SETFONT,
    WM_SIZE, WS_CAPTION, WS_CLIPCHILDREN, WS_EX_DLGMODALFRAME, WS_POPUP, WS_SYSMENU,
};

use crate::controls;
use crate::dpi;
use crate::text::wide;

/// 窗口类名，由主程序注册。
pub const CROP_CLASS: &str = "UsbscanCropWindow";

const CMD_APPLY: usize = 3501;
const CMD_RESET: usize = 3502;
const CMD_CANCEL: usize = 3503;
const HANDLE_RADIUS: i32 = 7;
const MINIMUM_SIZE: f64 = 4.0;

#[derive(Clone, Copy)]
enum DragMode {
    None,
    Corner(usize),
    Move {
        start: (f64, f64),
        rect: (f64, f64, f64, f64),
    },
    New {
        anchor: (f64, f64),
    },
}

struct CropState {
    parent: HWND,
    dark: bool,
    image_width: u32,
    image_height: u32,
    bgra: Vec<u8>,
    initial: (f64, f64, f64, f64),
    selection: (f64, f64, f64, f64),
    drag: DragMode,
    buttons: [HWND; 3],
    result: Option<(u32, u32, u32, u32)>,
}

/// 打开模态裁剪窗口，返回 `(left, top, width, height)`。
pub fn edit_crop(
    parent: HWND,
    width: u32,
    height: u32,
    bgra: &[u8],
    dark: bool,
) -> Option<(u32, u32, u32, u32)> {
    let inset_x = f64::from(width) * 0.05;
    let inset_y = f64::from(height) * 0.05;
    let initial = (
        inset_x,
        inset_y,
        f64::from(width) - inset_x,
        f64::from(height) - inset_y,
    );
    let state = Box::new(CropState {
        parent,
        dark,
        image_width: width,
        image_height: height,
        bgra: bgra.to_vec(),
        initial,
        selection: initial,
        drag: DragMode::None,
        buttons: [ptr::null_mut(); 3],
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
    let width = (parent_width * 85 / 100).max(640);
    let height = (parent_height * 85 / 100).max(480);
    let x = parent_rect.left + (parent_width - width) / 2;
    let y = parent_rect.top + (parent_height - height) / 2;

    let hinstance = unsafe { GetModuleHandleW(ptr::null()) };
    let class = wide(CROP_CLASS);
    let title = wide("裁剪 — 拖动边角调整区域");
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
            if message.message == WM_KEYDOWN {
                if message.wParam == VK_ESCAPE as usize {
                    DestroyWindow(hwnd);
                    continue;
                }
                if message.wParam == VK_RETURN as usize {
                    apply(hwnd);
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

/// 裁剪窗口的消息处理。
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
                let state = create.lpCreateParams.cast::<CropState>();
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
                let labels = ["应用", "重置", "取消"];
                let ids = [CMD_APPLY, CMD_RESET, CMD_CANCEL];
                for index in 0..3 {
                    (*state).buttons[index] = controls::create_push_button(
                        hwnd,
                        create.hInstance,
                        ids[index],
                        labels[index],
                    );
                }
                let font = SendMessageW((*state).parent, WM_GETFONT, 0, 0) as WPARAM;
                if font != 0 {
                    for button in (*state).buttons {
                        if !button.is_null() {
                            SendMessageW(button, WM_SETFONT, font, 1);
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
                    let view = view(hwnd);
                    let point = view.to_image(mouse_point(lparam));
                    let selection = (*state).selection;
                    let corner = nearest_corner(selection, point, view.scale, dpi_for_window(hwnd));
                    (*state).drag = if let Some(index) = corner {
                        DragMode::Corner(index)
                    } else if inside(selection, point) {
                        DragMode::Move {
                            start: point,
                            rect: selection,
                        }
                    } else {
                        (*state).selection = (point.0, point.1, point.0, point.1);
                        DragMode::New { anchor: point }
                    };
                    SetCapture(hwnd);
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
                0
            }
            WM_MOUSEMOVE => {
                let state = state_ptr(hwnd);
                if !state.is_null() {
                    let view = view(hwnd);
                    let point = view.to_image(mouse_point(lparam));
                    match (*state).drag {
                        DragMode::None => {}
                        DragMode::Corner(index) => {
                            update_corner(&mut (*state).selection, index, point);
                        }
                        DragMode::Move { start, rect } => {
                            (*state).selection = move_rect(
                                rect,
                                point.0 - start.0,
                                point.1 - start.1,
                                f64::from((*state).image_width),
                                f64::from((*state).image_height),
                            );
                        }
                        DragMode::New { anchor } => {
                            (*state).selection = (
                                anchor.0.min(point.0),
                                anchor.1.min(point.1),
                                anchor.0.max(point.0),
                                anchor.1.max(point.1),
                            );
                        }
                    }
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
                0
            }
            WM_LBUTTONUP => {
                let state = state_ptr(hwnd);
                if !state.is_null() {
                    (*state).drag = DragMode::None;
                    ReleaseCapture();
                    if selection_size((*state).selection) < MINIMUM_SIZE {
                        (*state).selection = (*state).initial;
                    }
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
                0
            }
            WM_COMMAND => {
                match wparam & 0xFFFF {
                    CMD_APPLY => apply(hwnd),
                    CMD_RESET => {
                        let state = state_ptr(hwnd);
                        if !state.is_null() {
                            (*state).selection = (*state).initial;
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
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

fn state_ptr(hwnd: HWND) -> *mut CropState {
    // SAFETY: 读取窗口附加数据。
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut CropState }
}

fn apply(hwnd: HWND) {
    let state = state_ptr(hwnd);
    if state.is_null() {
        return;
    }
    // SAFETY: state 有效；裁剪坐标转换只做纯计算。
    unsafe {
        let (left, top, right, bottom) = (*state).selection;
        let image_width = f64::from((*state).image_width);
        let image_height = f64::from((*state).image_height);
        let left = left.floor().clamp(0.0, image_width - 1.0) as u32;
        let top = top.floor().clamp(0.0, image_height - 1.0) as u32;
        let right = right.round().clamp(f64::from(left + 1), image_width) as u32;
        let bottom = bottom.round().clamp(f64::from(top + 1), image_height) as u32;
        (*state).result = Some((left, top, right - left, bottom - top));
        DestroyWindow(hwnd);
    }
}

fn update_corner(selection: &mut (f64, f64, f64, f64), index: usize, point: (f64, f64)) {
    match index {
        0 => {
            selection.0 = point.0.min(selection.2 - MINIMUM_SIZE);
            selection.1 = point.1.min(selection.3 - MINIMUM_SIZE);
        }
        1 => {
            selection.2 = point.0.max(selection.0 + MINIMUM_SIZE);
            selection.1 = point.1.min(selection.3 - MINIMUM_SIZE);
        }
        2 => {
            selection.2 = point.0.max(selection.0 + MINIMUM_SIZE);
            selection.3 = point.1.max(selection.1 + MINIMUM_SIZE);
        }
        _ => {
            selection.0 = point.0.min(selection.2 - MINIMUM_SIZE);
            selection.3 = point.1.max(selection.1 + MINIMUM_SIZE);
        }
    }
}

fn move_rect(
    rect: (f64, f64, f64, f64),
    dx: f64,
    dy: f64,
    width: f64,
    height: f64,
) -> (f64, f64, f64, f64) {
    let rect_width = rect.2 - rect.0;
    let rect_height = rect.3 - rect.1;
    let left = (rect.0 + dx).clamp(0.0, width - rect_width);
    let top = (rect.1 + dy).clamp(0.0, height - rect_height);
    (left, top, left + rect_width, top + rect_height)
}

fn selection_size(selection: (f64, f64, f64, f64)) -> f64 {
    (selection.2 - selection.0).min(selection.3 - selection.1)
}

fn inside(selection: (f64, f64, f64, f64), point: (f64, f64)) -> bool {
    point.0 >= selection.0
        && point.0 <= selection.2
        && point.1 >= selection.1
        && point.1 <= selection.3
}

fn nearest_corner(
    selection: (f64, f64, f64, f64),
    point: (f64, f64),
    scale: f64,
    dpi: u32,
) -> Option<usize> {
    let corners = [
        (selection.0, selection.1),
        (selection.2, selection.1),
        (selection.2, selection.3),
        (selection.0, selection.3),
    ];
    let radius = f64::from(dpi::scale(HANDLE_RADIUS + 3, dpi)) / scale.max(0.0001);
    let mut best = None;
    let mut best_distance = radius;
    for (index, corner) in corners.iter().enumerate() {
        let distance = ((corner.0 - point.0).powi(2) + (corner.1 - point.1).powi(2)).sqrt();
        if distance <= best_distance {
            best_distance = distance;
            best = Some(index);
        }
    }
    best
}

fn layout(hwnd: HWND, state: *mut CropState) {
    // SAFETY: 标准布局计算。
    unsafe {
        let mut rect: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut rect);
        let dpi = dpi::dpi_for_window(hwnd);
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

struct View {
    left: f64,
    top: f64,
    scale: f64,
    image_width: f64,
    image_height: f64,
}

fn view(hwnd: HWND) -> View {
    // SAFETY: 只查询窗口尺寸与 DPI。
    unsafe {
        let mut client: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut client);
        let dpi = dpi::dpi_for_window(hwnd);
        let margin = f64::from(dpi::scale(16, dpi));
        let bottom = f64::from(dpi::scale(64, dpi));
        let display_width = (f64::from(client.right) - margin * 2.0).max(1.0);
        let display_height = (f64::from(client.bottom) - margin - bottom).max(1.0);
        let state = state_ptr(hwnd);
        let (image_width, image_height) = if state.is_null() {
            (1.0, 1.0)
        } else {
            (
                f64::from((*state).image_width.max(1)),
                f64::from((*state).image_height.max(1)),
            )
        };
        let scale = (display_width / image_width).min(display_height / image_height);
        View {
            left: margin + (display_width - image_width * scale) / 2.0,
            top: margin + (display_height - image_height * scale) / 2.0,
            scale,
            image_width,
            image_height,
        }
    }
}

impl View {
    fn to_display(&self, point: (f64, f64)) -> POINT {
        POINT {
            x: (self.left + point.0 * self.scale).round() as i32,
            y: (self.top + point.1 * self.scale).round() as i32,
        }
    }

    fn to_image(&self, point: POINT) -> (f64, f64) {
        let x = (f64::from(point.x) - self.left) / self.scale;
        let y = (f64::from(point.y) - self.top) / self.scale;
        (
            x.clamp(0.0, self.image_width),
            y.clamp(0.0, self.image_height),
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
            let view = view(hwnd);
            draw_image(
                memory,
                &view,
                (*state).image_width,
                (*state).image_height,
                &(*state).bgra,
            );
            draw_selection(memory, &view, (*state).selection, dark);
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

fn draw_selection(hdc: HDC, view: &View, selection: (f64, f64, f64, f64), dark: bool) {
    let color = if dark {
        rgb(0x5a, 0xb0, 0xff)
    } else {
        rgb(0x00, 0x5f, 0xbb)
    };
    let top_left = view.to_display((selection.0, selection.1));
    let bottom_right = view.to_display((selection.2, selection.3));
    // SAFETY: 纯 GDI 绘制，对象成对释放。
    unsafe {
        let pen = CreatePen(0, 2, color);
        let old_pen = SelectObject(hdc, pen);
        let null_brush = GetStockObject(5); // NULL_BRUSH
        let old_brush = SelectObject(hdc, null_brush);
        let _ = MoveToEx(hdc, top_left.x, top_left.y, ptr::null_mut());
        let _ = LineTo(hdc, bottom_right.x, top_left.y);
        let _ = LineTo(hdc, bottom_right.x, bottom_right.y);
        let _ = LineTo(hdc, top_left.x, bottom_right.y);
        let _ = LineTo(hdc, top_left.x, top_left.y);
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
        DeleteObject(pen as HGDIOBJ);

        let handle_pen = CreatePen(0, 2, color);
        let old_handle_pen = SelectObject(hdc, handle_pen);
        let radius = HANDLE_RADIUS;
        for point in [
            top_left,
            POINT {
                x: bottom_right.x,
                y: top_left.y,
            },
            bottom_right,
            POINT {
                x: top_left.x,
                y: bottom_right.y,
            },
        ] {
            Ellipse(
                hdc,
                point.x - radius,
                point.y - radius,
                point.x + radius,
                point.y + radius,
            );
        }
        SelectObject(hdc, old_handle_pen);
        DeleteObject(handle_pen as HGDIOBJ);
    }
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
    fn move_rect_stays_inside_image() {
        let moved = move_rect((10.0, 10.0, 60.0, 60.0), 100.0, -100.0, 100.0, 80.0);
        assert_eq!(moved, (50.0, 0.0, 100.0, 50.0));
    }

    #[test]
    fn corner_drag_keeps_minimum_size() {
        let mut selection = (10.0, 10.0, 90.0, 90.0);
        update_corner(&mut selection, 1, (5.0, 5.0));
        assert!(selection.2 - selection.0 >= MINIMUM_SIZE);
        assert!(selection.3 - selection.1 >= MINIMUM_SIZE);
    }
}
