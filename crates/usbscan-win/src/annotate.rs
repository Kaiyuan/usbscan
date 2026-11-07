// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 页面批注窗口。
//!
//! 原生工具按钮 + 自绘画布：支持手写笔迹、矩形框、颜色切换、撤销与清空。
//! 应用后把批注扁平化到成品图片，批注本身不单独保存。

use std::ptr;

use usbscan_imaging::{colors, Annotation, AnnotationColor};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreatePen, CreateSolidBrush,
    DeleteDC, DeleteObject, EndPaint, FillRect, GetStockObject, InvalidateRect, Polyline,
    Rectangle, SelectObject, SetStretchBltMode, StretchDIBits, BITMAPINFO, BITMAPINFOHEADER,
    COLORONCOLOR, DIB_RGB_COLORS, HDC, HGDIOBJ, SRCCOPY,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetKeyState, ReleaseCapture, SetCapture, VK_CONTROL, VK_ESCAPE, VK_Z,
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
pub const ANNOTATE_CLASS: &str = "UsbscanAnnotateWindow";

const CMD_PEN: usize = 3201;
const CMD_RECT: usize = 3202;
const CMD_COLOR: usize = 3203;
const CMD_UNDO: usize = 3204;
const CMD_CLEAR: usize = 3205;
const CMD_APPLY: usize = 3206;
const CMD_CANCEL: usize = 3207;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Pen,
    Rect,
}

const PALETTE: [(&str, AnnotationColor); 4] = [
    ("红色", colors::RED),
    ("蓝色", colors::BLUE),
    ("黑色", colors::BLACK),
    ("黄色", colors::YELLOW),
];

struct AnnotateState {
    parent: HWND,
    dark: bool,
    image_width: u32,
    image_height: u32,
    bgra: Vec<u8>,
    annotations: Vec<Annotation>,
    current: Option<Annotation>,
    tool: Tool,
    color_index: usize,
    stroke: u32,
    buttons: [HWND; 7],
    result: Option<Vec<Annotation>>,
}

/// 打开模态批注窗口，返回用户确认的批注列表。
pub fn edit_annotations(
    parent: HWND,
    width: u32,
    height: u32,
    bgra: &[u8],
    dark: bool,
) -> Option<Vec<Annotation>> {
    let stroke = (width.min(height) / 240).clamp(2, 14);
    let state = Box::new(AnnotateState {
        parent,
        dark,
        image_width: width,
        image_height: height,
        bgra: bgra.to_vec(),
        annotations: Vec::new(),
        current: None,
        tool: Tool::Pen,
        color_index: 0,
        stroke,
        buttons: [ptr::null_mut(); 7],
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
    let width = (parent_width * 85 / 100).max(760);
    let height = (parent_height * 85 / 100).max(520);
    let x = parent_rect.left + (parent_width - width) / 2;
    let y = parent_rect.top + (parent_height - height) / 2;

    let hinstance = unsafe { GetModuleHandleW(ptr::null()) };
    let class = wide(ANNOTATE_CLASS);
    let title = wide("批注 — 拖动绘制，Ctrl+Z 撤销");
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
                if message.wParam == VK_Z as usize && GetKeyState(i32::from(VK_CONTROL)) < 0 {
                    undo(hwnd);
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

/// 批注窗口的消息处理。
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
                let state = create.lpCreateParams.cast::<AnnotateState>();
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
                let labels = ["画笔", "矩形", "颜色：红色", "撤销", "清空", "应用", "取消"];
                let ids = [
                    CMD_PEN, CMD_RECT, CMD_COLOR, CMD_UNDO, CMD_CLEAR, CMD_APPLY, CMD_CANCEL,
                ];
                for index in 0..7 {
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
                update_buttons(state);
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
                    (*state).current = match (*state).tool {
                        Tool::Pen => Some(Annotation::Pen {
                            color: PALETTE[(*state).color_index].1,
                            width: (*state).stroke,
                            points: vec![(point.0, point.1)],
                        }),
                        Tool::Rect => Some(Annotation::Rect {
                            color: PALETTE[(*state).color_index].1,
                            width: (*state).stroke,
                            rect: (point.0, point.1, point.0, point.1),
                        }),
                    };
                    SetCapture(hwnd);
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
                0
            }
            WM_MOUSEMOVE => {
                let state = state_ptr(hwnd);
                if !state.is_null() && (*state).current.is_some() {
                    let view = view(hwnd);
                    let point = view.to_image(mouse_point(lparam));
                    match (*state).current.as_mut() {
                        Some(Annotation::Pen { points, width, .. }) => {
                            let minimum = f64::from((*width).max(1)) * 0.35;
                            let last = points.last().copied().unwrap_or((point.0, point.1));
                            if (point.0 - last.0).hypot(point.1 - last.1) >= minimum {
                                points.push((point.0, point.1));
                            }
                        }
                        Some(Annotation::Rect { rect, .. }) => {
                            rect.2 = point.0;
                            rect.3 = point.1;
                        }
                        None => {}
                    }
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
                0
            }
            WM_LBUTTONUP => {
                let state = state_ptr(hwnd);
                if !state.is_null() {
                    ReleaseCapture();
                    if let Some(annotation) = (*state).current.take() {
                        // 太短的点按或过小的矩形不保留，避免误触留下杂点。
                        let keep = match &annotation {
                            Annotation::Pen { points, .. } => points.len() >= 2,
                            Annotation::Rect { rect, .. } => {
                                (rect.2 - rect.0).abs() >= 2.0 && (rect.3 - rect.1).abs() >= 2.0
                            }
                        };
                        if keep {
                            (*state).annotations.push(annotation);
                        }
                    }
                    update_buttons(state);
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
                0
            }
            WM_COMMAND => {
                let state = state_ptr(hwnd);
                if !state.is_null() {
                    match wparam & 0xFFFF {
                        CMD_PEN => {
                            (*state).tool = Tool::Pen;
                            update_buttons(state);
                        }
                        CMD_RECT => {
                            (*state).tool = Tool::Rect;
                            update_buttons(state);
                        }
                        CMD_COLOR => {
                            (*state).color_index = ((*state).color_index + 1) % PALETTE.len();
                            update_buttons(state);
                        }
                        CMD_UNDO => undo(hwnd),
                        CMD_CLEAR => {
                            (*state).annotations.clear();
                            (*state).current = None;
                            update_buttons(state);
                            InvalidateRect(hwnd, ptr::null(), 0);
                        }
                        CMD_APPLY => {
                            (*state).result = Some((*state).annotations.clone());
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

fn state_ptr(hwnd: HWND) -> *mut AnnotateState {
    // SAFETY: 读取窗口附加数据。
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut AnnotateState }
}

fn undo(hwnd: HWND) {
    let state = state_ptr(hwnd);
    if state.is_null() {
        return;
    }
    // SAFETY: state 有效。
    unsafe {
        (*state).annotations.pop();
        (*state).current = None;
        update_buttons(state);
        InvalidateRect(hwnd, ptr::null(), 0);
    }
}

fn update_buttons(state: *mut AnnotateState) {
    // SAFETY: state 与按钮句柄有效。
    unsafe {
        let s = &mut *state;
        controls::set_text(
            s.buttons[0],
            if s.tool == Tool::Pen {
                "● 画笔"
            } else {
                "画笔"
            },
        );
        controls::set_text(
            s.buttons[1],
            if s.tool == Tool::Rect {
                "● 矩形"
            } else {
                "矩形"
            },
        );
        controls::set_text(s.buttons[2], &format!("颜色：{}", PALETTE[s.color_index].0));
        EnableWindow(s.buttons[3], i32::from(!s.annotations.is_empty()));
        EnableWindow(s.buttons[4], i32::from(!s.annotations.is_empty()));
    }
}

fn layout(hwnd: HWND, state: *mut AnnotateState) {
    // SAFETY: 标准布局计算。
    unsafe {
        let mut rect: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut rect);
        let dpi = dpi::dpi_for_window(hwnd);
        let margin = dpi::scale(12, dpi);
        let button_height = dpi::scale(30, dpi);
        let gap = dpi::scale(6, dpi);
        let available = (rect.right - margin * 2).max(1);
        let button_width = ((available - gap * 4) / 5).min(dpi::scale(96, dpi)).max(1);

        for index in 0..5 {
            SetWindowPos(
                (*state).buttons[index],
                ptr::null_mut(),
                margin + index as i32 * (button_width + gap),
                margin,
                button_width,
                button_height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        let bottom = rect.bottom - margin - button_height;
        let action_width = dpi::scale(96, dpi);
        let cancel_x = rect.right - margin - action_width;
        let apply_x = cancel_x - action_width - gap;
        SetWindowPos(
            (*state).buttons[5],
            ptr::null_mut(),
            apply_x,
            bottom,
            action_width,
            button_height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        SetWindowPos(
            (*state).buttons[6],
            ptr::null_mut(),
            cancel_x,
            bottom,
            action_width,
            button_height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
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
        let margin = f64::from(dpi::scale(12, dpi));
        let toolbar = f64::from(dpi::scale(12, dpi) * 2 + dpi::scale(30, dpi));
        let display_left = margin;
        let display_top = toolbar;
        let display_width = (f64::from(client.right) - margin * 2.0).max(1.0);
        let display_height = (f64::from(client.bottom) - toolbar * 2.0).max(1.0);
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
            left: display_left + (display_width - image_width * scale) / 2.0,
            top: display_top + (display_height - image_height * scale) / 2.0,
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
            x.clamp(0.0, self.image_width - 1.0),
            y.clamp(0.0, self.image_height - 1.0),
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
            for annotation in &(*state).annotations {
                draw_annotation(memory, &view, annotation);
            }
            if let Some(annotation) = &(*state).current {
                draw_annotation(memory, &view, annotation);
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

fn draw_annotation(hdc: HDC, view: &View, annotation: &Annotation) {
    let color = annotation.color();
    let colorref = rgb(u32::from(color.r), u32::from(color.g), u32::from(color.b));
    let pen_width = (f64::from(annotation.width()) * view.scale)
        .round()
        .clamp(1.0, 64.0) as i32;
    // SAFETY: 纯 GDI 绘制，对象成对释放。
    unsafe {
        let pen = CreatePen(0, pen_width, colorref);
        let old_pen = SelectObject(hdc, pen);
        match annotation {
            Annotation::Pen { points, .. } => {
                let mapped: Vec<POINT> =
                    points.iter().map(|point| view.to_display(*point)).collect();
                if mapped.len() >= 2 {
                    Polyline(
                        hdc,
                        mapped.as_ptr(),
                        i32::try_from(mapped.len()).unwrap_or(0),
                    );
                }
            }
            Annotation::Rect { rect, .. } => {
                let left = rect.0.min(rect.2);
                let top = rect.1.min(rect.3);
                let right = rect.0.max(rect.2);
                let bottom = rect.1.max(rect.3);
                let top_left = view.to_display((left, top));
                let bottom_right = view.to_display((right, bottom));
                let null_brush = GetStockObject(5); // NULL_BRUSH
                let old_brush = SelectObject(hdc, null_brush);
                Rectangle(hdc, top_left.x, top_left.y, bottom_right.x, bottom_right.y);
                SelectObject(hdc, old_brush);
            }
        }
        SelectObject(hdc, old_pen);
        DeleteObject(pen as HGDIOBJ);
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
