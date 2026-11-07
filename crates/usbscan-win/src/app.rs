// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 主窗口与应用生命周期。
//!
//! M0-1 骨架 + M0-2 采集链路：原生菜单栏、工具栏、状态栏、
//! 缩略图面板、预览区与增强面板，并已接入 Media Foundation 实时取帧。

use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use usbscan_capture as capture;
use usbscan_doc as doc;
use usbscan_imaging as imaging;
use usbscan_naming as naming;

use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontIndirectW, CreatePen, CreateSolidBrush, DeleteObject, DrawTextW,
    EndPaint, FillRect, GetStockObject, GetSysColor, GetSysColorBrush, InvalidateRect, Rectangle,
    SelectObject, SetBkColor, SetBkMode, SetBrushOrgEx, SetStretchBltMode, SetTextColor,
    StretchDIBits, UpdateWindow, BITMAPINFO, BITMAPINFOHEADER, COLOR_APPWORKSPACE, COLOR_BTNFACE,
    COLOR_WINDOWTEXT, DIB_RGB_COLORS, HALFTONE, HBRUSH, HDC, HGDIOBJ, PS_DOT, SRCCOPY,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{
    InitCommonControlsEx, HIMAGELIST, ICC_BAR_CLASSES, ICC_LISTVIEW_CLASSES, ICC_STANDARD_CLASSES,
    INITCOMMONCONTROLSEX,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CheckMenuItem, CreateWindowExW, DefWindowProcW, DestroyAcceleratorTable, DestroyWindow,
    DispatchMessageW, EnableMenuItem, GetClientRect, GetMenu, GetMessageW, GetParent, GetSubMenu,
    GetWindowLongPtrW, GetWindowRect, KillTimer, LoadCursorW, MessageBoxW, PostQuitMessage,
    RegisterClassW, SendMessageW, SetTimer, SetWindowLongPtrW, SetWindowPos, SetWindowTextW,
    ShowWindow, SystemParametersInfoW, TranslateAcceleratorW, TranslateMessage, CREATESTRUCTW,
    CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, IDC_ARROW, MB_ICONERROR, MB_ICONINFORMATION,
    MB_ICONQUESTION, MB_OK, MB_YESNO, MF_BYCOMMAND, MF_CHECKED, MF_ENABLED, MF_GRAYED, MINMAXINFO,
    MSG, NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS, SWP_NOACTIVATE, SWP_NOZORDER, SW_SHOWDEFAULT,
    WM_CLOSE, WM_COMMAND, WM_CREATE, WM_CTLCOLORBTN, WM_CTLCOLORSTATIC, WM_DESTROY,
    WM_DEVICECHANGE, WM_DPICHANGED, WM_ERASEBKGND, WM_GETFONT, WM_GETMINMAXINFO, WM_NCDESTROY,
    WM_PAINT, WM_SETFONT, WM_SETTINGCHANGE, WM_SIZE, WM_TIMER, WNDCLASSW, WS_CHILD,
    WS_CLIPCHILDREN, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

use crate::capture_host::{
    CaptureHost, Status as CameraStatus, WM_APP_DEVICES, WM_APP_FRAME, WM_APP_STATUS,
};
use crate::config::Config;
use crate::controls::{self, ToolButton};
use crate::dpi;
use crate::error::WinError;
use crate::menu;
use crate::text::wide;

const MAIN_CLASS: &str = "UsbscanMainWindow";
const PREVIEW_CLASS: &str = "UsbscanPreviewWindow";

const DT_CENTER: u32 = 0x0000_0001;
const DT_SINGLELINE: u32 = 0x0000_0020;
const DT_NOPREFIX: u32 = 0x0000_0800;

const NULL_BRUSH_INDEX: i32 = 5;

/// 工具栏"拍照"按钮与 F9 加速键共用的命令 ID。
const CMD_CAPTURE: usize = menu::id::CAPTURE;
/// 设备下拉框 ID。
const CMD_DEVICE_COMBO: usize = 2300;

/// 撤销拍摄的计时器 ID。
const UNDO_TIMER: usize = 1;
/// 拍摄后允许撤销的时间窗口（毫秒）。
const UNDO_WINDOW_MS: u32 = 3_000;
/// 缩略图逻辑尺寸（96 DPI）。
const THUMB_WIDTH: i32 = 112;
const THUMB_HEIGHT: i32 = 140;

/// `MessageBoxW` 的“是”返回值。
const IDYES: i32 = 6;

/// 下拉框选择变化的通知码（`CBN_SELCHANGE`）。
const CBN_SELCHANGE: u32 = 1;
/// 设备树发生变化（`DBT_DEVNODES_CHANGED`）。
const DBT_DEVNODES_CHANGED: usize = 0x0007;

/// 供预览子窗口读取的深色模式缓存（注册表读取不宜每帧进行）。
static DARK_MODE: AtomicBool = AtomicBool::new(false);

/// 应用主状态。通过 `GWLP_USERDATA` 与窗口关联。
struct AppState {
    hinstance: HINSTANCE,
    hwnd: HWND,
    dpi: u32,
    dark: bool,
    font: usize,
    toolbar: HWND,
    device_label: HWND,
    device_combo: HWND,
    pages: HWND,
    preview: HWND,
    status: HWND,
    group: HWND,
    checks: [HWND; 4],
    track: HWND,
    track_label: HWND,
    panel_brush: HBRUSH,
    status_msg: String,
    capture: CaptureHost,
    camera_name: Option<String>,
    camera_size: Option<(u32, u32)>,
    /// 最近一次采集错误；`None` 表示正常或尚未开始。
    camera_error: Option<String>,
    /// 已选设备不在线时的名称。
    camera_waiting: Option<String>,
    /// 采集线程实际打开的摄像头 ID。
    active_id: Option<String>,
    /// 用户上次选择的摄像头（来自配置文件）。
    selected_id: Option<String>,
    selected_name: Option<String>,
    devices: Vec<capture::CameraInfo>,
    devices_version: u64,
    naming: naming::NamingEngine,
    rule: naming::NamingRule,
    output_root: PathBuf,
    session: doc::Session,
    store: doc::SessionStore,
    thumbs: HIMAGELIST,
    thumb_width: i32,
    thumb_height: i32,
    /// 最近一次拍摄；非空表示 3 秒撤销窗口内。
    undo_page: Option<doc::Page>,
}

/// 启动应用：创建窗口并进入消息循环。
///
/// 返回时表示用户已关闭主窗口；错误仅在初始化阶段产生。
pub fn run() -> Result<(), WinError> {
    let dark = crate::theme::system_uses_dark_mode();
    DARK_MODE.store(dark, Ordering::Relaxed);
    crate::theme::init_process(dark);

    init_common_controls();
    register_window_classes()?;

    let hinstance = module_handle();
    let menu_bar = menu::build_main_menu()?;
    let accelerators = menu::build_accelerators()?;
    let config = Config::load();
    let output_root = default_output_root();
    let store = doc::SessionStore::new(output_root.clone());

    let state = Box::new(AppState {
        hinstance,
        hwnd: ptr::null_mut(),
        dpi: dpi::BASE_DPI,
        dark,
        font: 0,
        toolbar: ptr::null_mut(),
        device_label: ptr::null_mut(),
        device_combo: ptr::null_mut(),
        pages: ptr::null_mut(),
        preview: ptr::null_mut(),
        status: ptr::null_mut(),
        group: ptr::null_mut(),
        checks: [ptr::null_mut(); 4],
        track: ptr::null_mut(),
        track_label: ptr::null_mut(),
        panel_brush: ptr::null_mut(),
        status_msg: "就绪".to_string(),
        capture: CaptureHost::new(),
        camera_name: None,
        camera_size: None,
        camera_error: None,
        camera_waiting: None,
        active_id: None,
        selected_id: config.last_camera_id,
        selected_name: config.last_camera_name,
        devices: Vec::new(),
        devices_version: 0,
        naming: naming::NamingEngine::default(),
        rule: naming::NamingRule::default(),
        output_root,
        session: doc::Session::default(),
        store,
        thumbs: 0,
        thumb_width: 0,
        thumb_height: 0,
        undo_page: None,
    });
    let state_ptr = Box::into_raw(state);

    let class = wide(MAIN_CLASS);
    let title = wide("usbscan — 未命名文档");
    let initial_dpi = dpi::dpi_for_window(ptr::null_mut());

    // SAFETY: 所有句柄有效；lpparam 是一个堆分配的 AppState 指针，由窗口在 WM_NCDESTROY 时回收。
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            dpi::scale(1280, initial_dpi),
            dpi::scale(800, initial_dpi),
            ptr::null_mut(),
            menu_bar,
            hinstance,
            state_ptr.cast(),
        )
    };

    if hwnd.is_null() {
        // 窗口未创建成功，WM_CREATE 从未执行，需要手动回收。
        // SAFETY: state_ptr 由本函数创建且尚未交出所有权。
        unsafe { drop(Box::from_raw(state_ptr)) };
        return Err(WinError::last());
    }

    // SAFETY: 标准 Win32 消息循环。
    unsafe {
        ShowWindow(hwnd, SW_SHOWDEFAULT);
        UpdateWindow(hwnd);

        let mut msg: MSG = std::mem::zeroed();
        loop {
            let ret = GetMessageW(&mut msg, ptr::null_mut(), 0, 0);
            if ret <= 0 {
                break;
            }
            if TranslateAcceleratorW(hwnd, accelerators, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        DestroyAcceleratorTable(accelerators);
    }

    Ok(())
}

const CW_USEDEFAULT: i32 = 0x8000_0000u32 as i32;

fn module_handle() -> HINSTANCE {
    // SAFETY: 获取当前模块句柄，永不失败。
    unsafe { GetModuleHandleW(ptr::null()) }
}

fn init_common_controls() {
    let icc = INITCOMMONCONTROLSEX {
        dwSize: u32::try_from(std::mem::size_of::<INITCOMMONCONTROLSEX>()).unwrap_or(8),
        dwICC: ICC_BAR_CLASSES | ICC_LISTVIEW_CLASSES | ICC_STANDARD_CLASSES,
    };
    // SAFETY: 结构体大小与类型正确。
    unsafe {
        InitCommonControlsEx(&icc);
    }
}

fn register_window_classes() -> Result<(), WinError> {
    let hinstance = module_handle();
    // SAFETY: 加载系统光标，参数为标准 IDC_ARROW。
    let cursor = unsafe { LoadCursorW(ptr::null_mut(), IDC_ARROW) };

    let classes: [(&str, WndprocAlias); 2] = [(MAIN_CLASS, wndproc), (PREVIEW_CLASS, preview_proc)];

    for (name, proc_fn) in classes {
        let class_name = wide(name);
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(proc_fn),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: ptr::null_mut(),
            hCursor: cursor,
            hbrBackground: (COLOR_BTNFACE + 1) as usize as HBRUSH,
            lpszMenuName: ptr::null(),
            lpszClassName: class_name.as_ptr(),
        };
        // SAFETY: WNDCLASSW 各字段有效，class_name 在调用期间存活。
        let atom = unsafe { RegisterClassW(&wc) };
        if atom == 0 {
            return Err(WinError::last());
        }
    }
    Ok(())
}

type WndprocAlias = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;

// ---------------------------------------------------------------------------
// 主窗口过程
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_lines)]
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: 全部为 Win32 契约内的操作；AppState 指针由本模块独占管理。
    unsafe {
        match msg {
            WM_CREATE => {
                let cs = lparam as *const CREATESTRUCTW;
                let state = (*cs).lpCreateParams as *mut AppState;
                (*state).hwnd = hwnd;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
                create_children(state);
                layout(state);
                0
            }
            WM_SIZE => {
                let state = state_ptr(hwnd);
                if !state.is_null() {
                    layout(state);
                }
                0
            }
            WM_APP_FRAME => {
                let state = state_ptr(hwnd);
                if !state.is_null() {
                    InvalidateRect((*state).preview, ptr::null(), 0);
                }
                0
            }
            WM_APP_STATUS => {
                let state = state_ptr(hwnd);
                if !state.is_null() {
                    update_camera_status(state);
                }
                0
            }
            WM_APP_DEVICES => {
                let state = state_ptr(hwnd);
                if !state.is_null() {
                    update_devices(state);
                }
                0
            }
            WM_TIMER => {
                if wparam == UNDO_TIMER {
                    let state = state_ptr(hwnd);
                    if !state.is_null() {
                        (*state).undo_page = None;
                        set_undo_enabled(hwnd, false);
                    }
                    KillTimer(hwnd, UNDO_TIMER);
                }
                0
            }
            WM_DPICHANGED => {
                let state = state_ptr(hwnd);
                if !state.is_null() {
                    let new_dpi = (wparam & 0xFFFF) as u32;
                    (*state).dpi = new_dpi;
                    rebuild_font(state);
                    let rect = &*(lparam as *const RECT);
                    SetWindowPos(
                        hwnd,
                        ptr::null_mut(),
                        rect.left,
                        rect.top,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                    layout(state);
                }
                0
            }
            WM_GETMINMAXINFO => {
                let state = state_ptr(hwnd);
                let dpi_now = if state.is_null() {
                    dpi::BASE_DPI
                } else {
                    (*state).dpi
                };
                let info = &mut *(lparam as *mut MINMAXINFO);
                info.ptMinTrackSize = POINT {
                    x: dpi::scale(1024, dpi_now),
                    y: dpi::scale(640, dpi_now),
                };
                0
            }
            WM_COMMAND => {
                let id = (wparam & 0xFFFF) as usize;
                let code = ((wparam >> 16) & 0xFFFF) as u32;
                on_command(hwnd, id, code);
                0
            }
            WM_DEVICECHANGE => {
                // 设备树变化只需重新枚举；具体增删由采集线程判断。
                if wparam == DBT_DEVNODES_CHANGED {
                    let state = state_ptr(hwnd);
                    if !state.is_null() {
                        (*state).capture.refresh();
                    }
                }
                0
            }
            WM_SETTINGCHANGE => {
                let state = state_ptr(hwnd);
                if !state.is_null() {
                    let dark = crate::theme::system_uses_dark_mode();
                    if dark != (*state).dark {
                        (*state).dark = dark;
                        apply_theme(state);
                    }
                }
                0
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => control_color(hwnd, wparam),
            WM_ERASEBKGND => {
                erase_background(hwnd, wparam);
                1
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                let state = state_ptr(hwnd);
                if !state.is_null() {
                    // 先停采集线程，确保它不再向即将销毁的窗口发消息。
                    (*state).capture.stop();
                    // 正常退出：标记会话已收尾，下次启动不再提示“未完成会话”。
                    (*state).session.closed_cleanly = true;
                    let _ = (*state).store.save(&(*state).session);
                }
                PostQuitMessage(0);
                0
            }
            WM_NCDESTROY => {
                let state = state_ptr(hwnd);
                if !state.is_null() {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    let brush = (*state).panel_brush;
                    if !brush.is_null() {
                        DeleteObject(brush as HGDIOBJ);
                    }
                    controls::destroy_image_list((*state).thumbs);
                    drop(Box::from_raw(state));
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

fn state_ptr(hwnd: HWND) -> *mut AppState {
    // SAFETY: 读取窗口附加数据，未设置时返回 0。
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut AppState }
}

const WM_CTLCOLORSTATIC_DEFAULT: u32 = 0x0138;

/// 把主窗口背景刷成与主题一致的底色。
fn erase_background(hwnd: HWND, wparam: WPARAM) {
    let state = state_ptr(hwnd);
    // SAFETY: HDC 由系统在 WM_ERASEBKGND 中传入；state 可能为空。
    unsafe {
        let hdc = wparam as HDC;
        let mut rc: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut rc);
        let brush = if state.is_null() {
            GetSysColorBrush(COLOR_BTNFACE)
        } else {
            (*state).panel_brush
        };
        if brush.is_null() {
            DefWindowProcW(hwnd, WM_ERASEBKGND, wparam, 0);
            return;
        }
        FillRect(hdc, &rc, brush);
    }
}

/// 为静态文本、分组框、复选框提供与主题一致的背景刷。
fn control_color(hwnd: HWND, wparam: WPARAM) -> LRESULT {
    let state = state_ptr(hwnd);
    if state.is_null() {
        // SAFETY: 未知状态，交回系统默认处理。
        return unsafe { DefWindowProcW(hwnd, WM_CTLCOLORSTATIC_DEFAULT, wparam, 0) };
    }
    // SAFETY: state 有效；HDC 由系统在 WM_CTLCOLORSTATIC 中传入。
    unsafe {
        let s = &*state;
        let hdc = wparam as HDC;
        let (fg, bg) = if s.dark {
            (rgb(0xe6, 0xe8, 0xeb), rgb(0x1e, 0x1f, 0x22))
        } else {
            (GetSysColor(COLOR_WINDOWTEXT), GetSysColor(COLOR_BTNFACE))
        };
        SetTextColor(hdc, fg);
        SetBkColor(hdc, bg);
        s.panel_brush as isize
    }
}

#[allow(clippy::too_many_lines)]
fn create_children(state: *mut AppState) {
    // SAFETY: state 由 WM_CREATE 传入且有效。
    unsafe {
        let s = &mut *state;
        s.dpi = dpi::dpi_for_window(s.hwnd);
        s.font = create_ui_font(s.dpi);

        let scale = |v: i32| dpi::scale(v, s.dpi);

        s.toolbar = controls::create_toolbar(
            s.hwnd,
            s.hinstance,
            &[
                ToolButton {
                    id: menu::id::CAPTURE,
                    label: "拍照",
                    separator_before: false,
                },
                ToolButton {
                    id: 2002,
                    label: "连拍",
                    separator_before: false,
                },
                ToolButton {
                    id: 2003,
                    label: "定时",
                    separator_before: false,
                },
                ToolButton {
                    id: 2004,
                    label: "录像",
                    separator_before: false,
                },
                ToolButton {
                    id: 2005,
                    label: "裁边",
                    separator_before: true,
                },
                ToolButton {
                    id: 2006,
                    label: "纠偏",
                    separator_before: false,
                },
                ToolButton {
                    id: 2007,
                    label: "识别",
                    separator_before: true,
                },
                ToolButton {
                    id: 2008,
                    label: "导出",
                    separator_before: true,
                },
                ToolButton {
                    id: 2009,
                    label: "打印",
                    separator_before: false,
                },
            ],
            scale(40),
            s.dpi,
        );

        s.device_label = controls::create_label(s.hwnd, s.hinstance, "设备");
        s.device_combo = controls::create_combo_box(s.hwnd, s.hinstance, CMD_DEVICE_COMBO);

        s.pages = controls::create_page_list(s.hwnd, s.hinstance);
        s.thumb_width = scale(THUMB_WIDTH);
        s.thumb_height = scale(THUMB_HEIGHT);
        s.thumbs = controls::create_image_list(s.thumb_width, s.thumb_height);
        if s.thumbs != 0 {
            controls::set_list_image_list(s.pages, s.thumbs);
        }

        let preview_class = wide(PREVIEW_CLASS);
        let empty = wide("");
        s.preview = CreateWindowExW(
            0,
            preview_class.as_ptr(),
            empty.as_ptr(),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            0,
            0,
            s.hwnd,
            ptr::null_mut(),
            s.hinstance,
            ptr::null(),
        );

        s.group = controls::create_group(s.hwnd, s.hinstance, "图像增强");
        let labels = ["自动裁边", "自动纠偏", "去阴影", "白底净化"];
        for (i, label) in labels.iter().enumerate() {
            s.checks[i] = controls::create_checkbox(s.hwnd, s.hinstance, 2100 + i, label);
        }
        s.track_label = controls::create_label(s.hwnd, s.hinstance, "强度");
        s.track = controls::create_trackbar(s.hwnd, s.hinstance, 2200);

        s.status = controls::create_status_bar(s.hwnd, s.hinstance);
        controls::set_status_min_height(s.status, scale(24));
        controls::set_status_text(s.status, 0, "● 未连接设备");
        controls::set_status_text(s.status, 1, "0 页");
        controls::set_status_text(s.status, 2, "就绪");
        controls::set_status_text(s.status, 3, "v0.1.0");

        apply_font_to_children(s);
        apply_theme(state);
        set_undo_enabled(s.hwnd, false);
        restore_session(state);

        // 采集线程在子窗口就绪后启动，避免它的第一条通知落空。
        s.capture
            .select(s.selected_id.clone(), s.selected_name.clone());
        s.capture.start(s.hwnd as isize);
    }
}

fn apply_font_to_children(state: &AppState) {
    if state.font == 0 {
        return;
    }
    let handle = state.font as WPARAM;
    // SAFETY: 句柄来自 CreateFontIndirectW，控件均为本窗口子窗口。
    unsafe {
        SendMessageW(state.hwnd, WM_SETFONT, handle, 1);
        for child in [
            state.toolbar,
            state.device_label,
            state.device_combo,
            state.pages,
            state.status,
            state.group,
            state.checks[0],
            state.checks[1],
            state.checks[2],
            state.checks[3],
            state.track_label,
            state.track,
        ] {
            if !child.is_null() {
                SendMessageW(child, WM_SETFONT, handle, 1);
            }
        }
    }
    controls::autosize_toolbar(state.toolbar);
}

fn apply_theme(state: *mut AppState) {
    // SAFETY: state 有效；主题应用为纯 Win32 调用。
    unsafe {
        let s = &mut *state;
        DARK_MODE.store(s.dark, Ordering::Relaxed);

        if !s.panel_brush.is_null() {
            DeleteObject(s.panel_brush as HGDIOBJ);
        }
        let panel_bg = if s.dark {
            rgb(0x1e, 0x1f, 0x22)
        } else {
            GetSysColor(COLOR_BTNFACE)
        };
        s.panel_brush = CreateSolidBrush(panel_bg);

        crate::theme::apply_window(s.hwnd, s.dark);
        for child in [
            s.toolbar,
            s.device_label,
            s.device_combo,
            s.pages,
            s.status,
            s.group,
            s.checks[0],
            s.checks[1],
            s.checks[2],
            s.checks[3],
            s.track_label,
            s.track,
        ] {
            if !child.is_null() {
                crate::theme::apply_control(child, s.dark);
            }
        }

        let text = if s.dark {
            rgb(0xe6, 0xe8, 0xeb)
        } else {
            GetSysColor(COLOR_WINDOWTEXT)
        };
        controls::set_list_colors(s.pages, panel_bg, text);

        let bar = GetMenu(s.hwnd);
        if !bar.is_null() {
            let view = GetSubMenu(bar, 3);
            if !view.is_null() {
                let check = if s.dark { MF_CHECKED } else { 0 };
                CheckMenuItem(view, menu::id::VIEW_DARK as u32, check);
            }
        }

        InvalidateRect(s.hwnd, ptr::null(), 1);
    }
}

fn rebuild_font(state: *mut AppState) {
    // SAFETY: state 有效。
    unsafe {
        let s = &mut *state;
        if s.font != 0 {
            DeleteObject(s.font as HGDIOBJ);
        }
        s.font = create_ui_font(s.dpi);
        apply_font_to_children(s);
    }
}

fn create_ui_font(dpi: u32) -> usize {
    // SAFETY: 通过系统参数获取字体描述，失败时回退默认字体。
    unsafe {
        let mut ncm: NONCLIENTMETRICSW = std::mem::zeroed();
        ncm.cbSize = u32::try_from(std::mem::size_of::<NONCLIENTMETRICSW>()).unwrap_or(0);
        let ok = SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            ncm.cbSize,
            std::ptr::from_mut(&mut ncm).cast(),
            0,
        );
        if ok == 0 {
            return 0;
        }

        let mut lf = ncm.lfMessageFont;
        let base = lf.lfHeight.abs();
        lf.lfHeight = -dpi::scale(base, dpi);
        CreateFontIndirectW(&lf) as usize
    }
}

// ---------------------------------------------------------------------------
// 布局与命令
// ---------------------------------------------------------------------------

fn layout(state: *mut AppState) {
    // SAFETY: state 有效；所有句柄均为本窗口子控件。
    unsafe {
        let s = &mut *state;
        let mut rc: RECT = std::mem::zeroed();
        GetClientRect(s.hwnd, &mut rc);
        let width = rc.right;
        let height = rc.bottom;
        let scale = |v: i32| dpi::scale(v, s.dpi);

        let toolbar_h = scale(40);
        SetWindowPos(
            s.toolbar,
            ptr::null_mut(),
            0,
            0,
            width,
            toolbar_h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );

        let param_h = scale(32);
        let param_y = toolbar_h + scale(5);
        place(s.device_label, scale(8), param_y, scale(36), scale(22));
        // 下拉列表的高度由窗口高度决定，闭合时只显示一行。
        place(s.device_combo, scale(48), param_y, scale(280), scale(200));

        SendMessageW(s.status, WM_SIZE, 0, 0);
        let status_h = window_height(s.status);
        let body_top = toolbar_h + param_h;
        let body_h = (height - body_top - status_h).max(0);

        let left_w = scale(190);
        let right_w = scale(300);
        let preview_w = (width - left_w - right_w).max(scale(200));

        place(s.pages, 0, body_top, left_w, body_h);
        place(s.preview, left_w, body_top, preview_w, body_h);
        place(s.group, left_w + preview_w, body_top, right_w, body_h);

        let panel_x = left_w + preview_w;
        let pad = scale(16);
        let row = scale(28);
        let mut y = body_top + scale(34);
        for check in s.checks {
            place(check, panel_x + pad, y, right_w - pad * 2, row);
            y += row;
        }
        y += scale(8);
        place(s.track_label, panel_x + pad, y, scale(48), row);
        place(
            s.track,
            panel_x + pad + scale(52),
            y,
            right_w - pad * 2 - scale(52),
            row,
        );

        let parts = [width * 22 / 100, width * 38 / 100, width * 80 / 100, -1];
        controls::set_status_parts(s.status, &parts);
        refresh_status(s);
    }
}

fn refresh_status(s: &AppState) {
    let camera = if let Some(name) = &s.camera_waiting {
        format!("○ 等待 {name}")
    } else if s.camera_error.is_some() {
        "● 摄像头错误".to_string()
    } else {
        match (&s.camera_name, s.camera_size) {
            (Some(name), Some((width, height))) => format!("● {name}  {width}×{height}"),
            (Some(name), None) => format!("● {name}"),
            _ => "● 未连接设备".to_string(),
        }
    };
    controls::set_status_text(s.status, 0, &camera);
    controls::set_status_text(s.status, 1, &format!("{} 页", s.session.pages.len()));
    controls::set_status_text(s.status, 2, &s.status_msg);
    controls::set_status_text(s.status, 3, "v0.1.0");

    let title = match &s.camera_name {
        Some(name) => format!("usbscan — 未命名文档 — {name}"),
        None => "usbscan — 未命名文档".to_string(),
    };
    let title = wide(&title);
    // SAFETY: 标题字符串在调用期间有效。
    unsafe {
        SetWindowTextW(s.hwnd, title.as_ptr());
    }
}

/// 从采集线程的共享状态同步摄像头信息到界面。
fn update_camera_status(state: *mut AppState) {
    let status = {
        // SAFETY: state 由调用方保证有效。
        let s = unsafe { &*state };
        let guard = s.capture.shared();
        guard.status.clone()
    };
    let Some(status) = status else {
        return;
    };

    // SAFETY: state 有效；后续均为字段写入与消息调用。
    unsafe {
        let s = &mut *state;
        match status {
            CameraStatus::Connecting => {
                s.camera_name = None;
                s.camera_size = None;
                s.camera_error = None;
                s.camera_waiting = None;
                s.active_id = None;
                s.status_msg = "正在检测摄像头…".to_string();
                let index = s
                    .selected_id
                    .as_ref()
                    .and_then(|id| s.devices.iter().position(|camera| &camera.id == id));
                controls::set_combo_index(s.device_combo, index);
            }
            CameraStatus::Ready {
                id,
                name,
                width,
                height,
            } => {
                if s.camera_name.as_deref() != Some(name.as_str()) {
                    s.status_msg = format!("已连接 {name}，按 F9 或点击“拍照”");
                }
                let index = s.devices.iter().position(|camera| camera.id == id);
                controls::set_combo_index(s.device_combo, index);
                s.camera_name = Some(name);
                s.camera_size = Some((width, height));
                s.camera_error = None;
                s.camera_waiting = None;
                s.active_id = Some(id);
            }
            CameraStatus::Waiting { name } => {
                s.camera_name = None;
                s.camera_size = None;
                s.camera_error = None;
                s.active_id = None;
                s.camera_waiting = Some(name.clone());
                s.status_msg = format!("等待摄像头：{name}");
            }
            CameraStatus::Error(message) => {
                s.camera_name = None;
                s.camera_size = None;
                s.camera_error = Some(message.clone());
                s.camera_waiting = None;
                s.active_id = None;
                s.status_msg = format!("摄像头错误：{message}");
            }
        }
        refresh_status(s);
        InvalidateRect(s.preview, ptr::null(), 0);
    }
}

/// 设备列表更新后重建下拉框，并保持当前选择。
fn update_devices(state: *mut AppState) {
    let (devices, version) = {
        // SAFETY: state 由调用方保证有效。
        let s = unsafe { &*state };
        let guard = s.capture.shared();
        (guard.devices.clone(), guard.devices_version)
    };

    // SAFETY: state 有效；下拉框为本窗口子控件。
    unsafe {
        let s = &mut *state;
        if version == s.devices_version {
            return;
        }
        s.devices_version = version;
        s.devices = devices;

        let names: Vec<String> = s.devices.iter().map(|camera| camera.name.clone()).collect();
        let index = s
            .active_id
            .as_ref()
            .and_then(|id| s.devices.iter().position(|camera| &camera.id == id))
            .or_else(|| {
                s.selected_id
                    .as_ref()
                    .and_then(|id| s.devices.iter().position(|camera| &camera.id == id))
            });
        controls::set_combo_items(s.device_combo, &names, index);
    }
}

/// 用户在下拉框中切换摄像头。
fn on_device_selected(hwnd: HWND) {
    let state = state_ptr(hwnd);
    if state.is_null() {
        return;
    }
    // SAFETY: state 有效；设备信息为本地副本。
    unsafe {
        let s = &mut *state;
        let Some(index) = controls::combo_selection(s.device_combo) else {
            return;
        };
        let Some(info) = s.devices.get(index).cloned() else {
            return;
        };

        s.selected_id = Some(info.id.clone());
        s.selected_name = Some(info.name.clone());
        s.status_msg = format!("正在切换到 {}…", info.name);
        s.capture
            .select(Some(info.id.clone()), Some(info.name.clone()));
        Config {
            last_camera_id: Some(info.id),
            last_camera_name: Some(info.name),
        }
        .save();
        refresh_status(s);
    }
}

/// 默认输出根目录：`%USERPROFILE%\Pictures\usbscan`。
fn default_output_root() -> PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("PUBLIC").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    home.join("Pictures").join("usbscan")
}

fn place(hwnd: HWND, x: i32, y: i32, w: i32, h: i32) {
    if hwnd.is_null() {
        return;
    }
    // SAFETY: 标准窗口位置设置。
    unsafe {
        SetWindowPos(
            hwnd,
            ptr::null_mut(),
            x,
            y,
            w,
            h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

fn window_height(hwnd: HWND) -> i32 {
    if hwnd.is_null() {
        return 0;
    }
    let mut rc: RECT = unsafe { std::mem::zeroed() };
    // SAFETY: 有效窗口句柄。
    unsafe {
        GetWindowRect(hwnd, &mut rc);
    }
    rc.bottom - rc.top
}

fn on_command(hwnd: HWND, id: usize, code: u32) {
    if id == CMD_DEVICE_COMBO {
        if code == CBN_SELCHANGE {
            on_device_selected(hwnd);
        }
        return;
    }

    if id == menu::id::EDIT_UNDO {
        undo_capture(hwnd);
        return;
    }

    if id == menu::id::DEVICE_REFRESH || id == menu::id::DEVICE_RESET {
        let state = state_ptr(hwnd);
        if !state.is_null() {
            // SAFETY: state 有效；refresh 只写入共享控制位。
            unsafe {
                (*state).capture.refresh();
                (*state).status_msg = "正在重新检测设备…".to_string();
                refresh_status(&*state);
            }
        }
        return;
    }

    if id == menu::id::FILE_EXIT {
        // SAFETY: 标准销毁流程。
        unsafe {
            DestroyWindow(hwnd);
        }
        return;
    }

    if id == menu::id::VIEW_DARK {
        let state = state_ptr(hwnd);
        if !state.is_null() {
            // SAFETY: state 有效。
            unsafe {
                (*state).dark = !(*state).dark;
            }
            apply_theme(state);
        }
        return;
    }

    if id == menu::id::HELP_ABOUT {
        show_about(hwnd);
        return;
    }

    if id == CMD_CAPTURE {
        capture_photo(hwnd);
        return;
    }

    let state = state_ptr(hwnd);
    if state.is_null() {
        return;
    }
    let message = describe(id);
    // SAFETY: state 有效；字符串为副本。
    unsafe {
        (*state).status_msg = message.to_string();
        refresh_status(&*state);
    }
}

fn describe(id: usize) -> &'static str {
    match id {
        menu::id::FILE_NEW => "新建文档（待实现）",
        menu::id::FILE_OPEN => "打开工程（待实现）",
        menu::id::FILE_IMPORT => "导入图片（待实现）",
        menu::id::FILE_EXPORT => "导出（待实现）",
        menu::id::FILE_PRINT => "打印（待实现）",
        menu::id::EDIT_UNDO => "撤销（待实现）",
        menu::id::EDIT_REDO => "重做（待实现）",
        menu::id::DEVICE_REFRESH => "正在重新检测设备…",
        menu::id::DEVICE_RESET => "重置设备（待实现）",
        menu::id::VIEW_THUMBS => "切换页面面板（待实现）",
        menu::id::VIEW_ENHANCE => "切换增强面板（待实现）",
        menu::id::TOOL_OCR => "OCR 识别（待实现）",
        menu::id::TOOL_SETTINGS => "设置（待实现）",
        2001 => "拍照（待实现）",
        2002 => "连拍（待实现）",
        2003 => "定时（待实现）",
        2004 => "录像（待实现）",
        2005 => "自动裁边（待实现）",
        2006 => "自动纠偏（待实现）",
        2007 => "OCR 识别（待实现）",
        2008 => "导出（待实现）",
        2009 => "打印（待实现）",
        _ => "就绪",
    }
}

/// 拍摄当前帧并立即落盘（FR-CAP-01 / FR-CAP-07）。
/// 启用/禁用“撤销拍摄”菜单项。
fn set_undo_enabled(hwnd: HWND, enabled: bool) {
    // SAFETY: 主窗口菜单由系统持有；菜单项 ID 固定。
    unsafe {
        let menu = GetMenu(hwnd);
        if menu.is_null() {
            return;
        }
        let flags = MF_BYCOMMAND | if enabled { MF_ENABLED } else { MF_GRAYED };
        EnableMenuItem(menu, menu::id::EDIT_UNDO as u32, flags);
    }
}

/// 启动时恢复上次会话。
///
/// - 正常退出过的会话直接恢复；
/// - 非正常退出且 24 小时内，询问用户；
/// - 清单损坏时归档后从空会话开始。
fn restore_session(state: *mut AppState) {
    // SAFETY: state 由 WM_CREATE 传入且有效。
    unsafe {
        let s = &mut *state;
        let session = match s.store.load() {
            Ok(Some(session)) => session,
            Ok(None) => return,
            Err(err) => {
                let archived = s.store.archive().ok().flatten().is_some();
                s.status_msg = if archived {
                    format!("会话清单损坏，已归档：{err}")
                } else {
                    format!("会话清单读取失败：{err}")
                };
                refresh_status(s);
                return;
            }
        };

        if session.pages.is_empty() {
            s.session = session;
            return;
        }

        let recent = doc::now_unix().saturating_sub(session.updated_at) <= 24 * 60 * 60;
        let restore = if session.closed_cleanly || !recent {
            true
        } else {
            confirm_restore(s.hwnd, session.pages.len())
        };

        if !restore {
            let _ = s.store.archive();
            s.session = doc::Session::default();
            s.status_msg = "已跳过上次会话".to_string();
            refresh_status(s);
            return;
        }

        s.session = session;
        s.session.closed_cleanly = false;
        rebuild_page_list(s);
        if s.status_msg.is_empty() || s.status_msg == "就绪" {
            s.status_msg = format!("已恢复会话（{} 页），按 F9 继续拍摄", s.session.pages.len());
        }
        refresh_status(s);
    }
}

/// 询问用户是否恢复未正常结束的会话。
fn confirm_restore(hwnd: HWND, pages: usize) -> bool {
    let text = wide(&format!(
        "检测到上次未正常结束的会话（{pages} 页）。\n\n是否恢复？"
    ));
    let title = wide("usbscan");
    // SAFETY: 字符串在调用期间有效；父窗口有效。
    unsafe {
        MessageBoxW(
            hwnd,
            text.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONQUESTION,
        ) == IDYES
    }
}

/// 根据会话重建缩略图列表。
fn rebuild_page_list(s: &mut AppState) {
    controls::clear_page_list(s.pages);
    let thumb_width = u32::try_from(s.thumb_width.max(1)).unwrap_or(1);
    let thumb_height = u32::try_from(s.thumb_height.max(1)).unwrap_or(1);
    let mut missing = 0usize;

    for index in 0..s.session.pages.len() {
        let page = &s.session.pages[index];
        let path = s.output_root.join(&page.path);
        let image = if let Ok((width, height, data)) =
            imaging::image_thumbnail_bgra(&path, thumb_width, thumb_height)
        {
            controls::add_image_bgra(s.thumbs, thumb_width, thumb_height, width, height, &data)
        } else {
            missing += 1;
            None
        };
        let name = page.path.file_name().map_or_else(
            || page.path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        controls::add_page_item(s.pages, index, &name, image);
    }

    if missing > 0 {
        s.status_msg = format!("{missing} 页图片文件缺失，仅恢复索引");
    }
}

/// 从最新一帧生成缩略图并加入图像列表。
fn add_frame_thumbnail(s: &AppState, frame: &capture::Frame) -> Option<usize> {
    let thumb_width = u32::try_from(s.thumb_width.max(1)).ok()?;
    let thumb_height = u32::try_from(s.thumb_height.max(1)).ok()?;
    let (width, height, data) = imaging::thumbnail_bgra(
        frame.width,
        frame.height,
        &frame.data,
        thumb_width,
        thumb_height,
    )
    .ok()?;
    controls::add_image_bgra(s.thumbs, thumb_width, thumb_height, width, height, &data)
}

/// 撤销最近一次拍摄：删除文件、回收序号并更新会话清单。
fn undo_capture(hwnd: HWND) {
    let state = state_ptr(hwnd);
    if state.is_null() {
        return;
    }
    // SAFETY: state 有效；文件操作返回值逐一检查。
    unsafe {
        let s = &mut *state;
        let Some(page) = s.undo_page.take() else {
            s.status_msg = "没有可撤销的拍摄".to_string();
            set_undo_enabled(hwnd, false);
            refresh_status(s);
            return;
        };

        let path = s.output_root.join(&page.path);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                // 删除失败时保留页面，避免界面与磁盘不一致。
                s.undo_page = Some(page);
                let message = format!("无法删除 {}：{err}", path.display());
                s.status_msg.clone_from(&message);
                refresh_status(s);
                show_error(hwnd, &message);
                return;
            }
        }

        if s.session.pages.last().map(|last| last.seq) == Some(page.seq) {
            s.session.undo_last();
        }
        s.session.closed_cleanly = false;
        controls::delete_last_page(s.pages, s.session.pages.len());
        let _ = s.store.save(&s.session);

        KillTimer(hwnd, UNDO_TIMER);
        set_undo_enabled(hwnd, false);

        let name = page.path.file_name().map_or_else(
            || page.path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        s.status_msg = format!("已撤销：{name}（序号 {} 已回收）", page.seq);
        refresh_status(s);
    }
}

fn capture_photo(hwnd: HWND) {
    let state = state_ptr(hwnd);
    if state.is_null() {
        return;
    }
    // SAFETY: state 有效；文件 API 返回值逐一检查。
    unsafe {
        let s = &mut *state;
        let Some(frame) = s.capture.latest_frame() else {
            s.status_msg = "尚未取得画面，请稍候再拍".to_string();
            refresh_status(s);
            return;
        };

        let seq = s.session.next_seq;
        let rendered = {
            let ctx = naming::NamingContext {
                now: naming::TimeStamp::now(),
                device: s.camera_name.as_deref(),
                width: Some(frame.width),
                height: Some(frame.height),
                seq,
                ..naming::NamingContext::default()
            };
            s.naming.render(&s.rule, &ctx)
        };

        let rendered = match rendered {
            Ok(rendered) => rendered,
            Err(err) => {
                let message = format!("命名规则渲染失败：{err}");
                s.status_msg.clone_from(&message);
                refresh_status(s);
                show_error(hwnd, &message);
                return;
            }
        };

        let dir = s.output_root.join(&rendered.folder);
        let reserved =
            match s
                .naming
                .reserve(&dir, &rendered.file_name, "jpg", naming::Collision::Append)
            {
                Ok(Some(reserved)) => reserved,
                Ok(None) => {
                    s.status_msg = "重名策略为跳过，本次未保存".to_string();
                    refresh_status(s);
                    return;
                }
                Err(err) => {
                    let message = format!("无法创建文件：{err}");
                    s.status_msg.clone_from(&message);
                    refresh_status(s);
                    show_error(hwnd, &message);
                    return;
                }
            };

        let path = reserved.path.clone();
        let result = write_jpeg(reserved.file, &frame);
        match result {
            Ok(()) => {
                let name = path.file_name().map_or_else(
                    || path.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                let image = add_frame_thumbnail(s, &frame);
                let relative = path
                    .strip_prefix(&s.output_root)
                    .map_or_else(|_| path.clone(), Path::to_path_buf);
                s.session.push_page(doc::Page {
                    seq,
                    path: relative,
                    width: frame.width,
                    height: frame.height,
                    captured_at: doc::now_unix(),
                    device: s.camera_name.clone(),
                });
                s.session.closed_cleanly = false;
                let item_index = s.session.pages.len() - 1;
                controls::add_page_item(s.pages, item_index, &name, image);
                s.undo_page = s.session.pages.last().cloned();
                if let Err(err) = s.store.save(&s.session) {
                    s.status_msg = format!("已保存 {name}，但会话清单写入失败：{err}");
                } else {
                    s.status_msg = format!("已保存：{name}（3 秒内可撤销）");
                }
                SetTimer(hwnd, UNDO_TIMER, UNDO_WINDOW_MS, None);
                set_undo_enabled(hwnd, true);
                refresh_status(s);
            }
            Err(message) => {
                let _ = fs::remove_file(&path);
                let message = format!("写入失败：{message}");
                s.status_msg.clone_from(&message);
                refresh_status(s);
                show_error(hwnd, &message);
            }
        }
    }
}

/// 把一帧编码为 JPEG 并同步到磁盘。
fn write_jpeg(file: fs::File, frame: &capture::Frame) -> Result<(), String> {
    let mut writer = BufWriter::new(file);
    imaging::encode_jpeg_bgra(frame.width, frame.height, &frame.data, 92, &mut writer)
        .map_err(|err| err.to_string())?;
    writer.flush().map_err(|err| err.to_string())?;
    // 取回文件句柄并落盘，保证进程崩溃/断电后文件仍完整。
    let file = writer
        .into_inner()
        .map_err(|err| err.into_error().to_string())?;
    file.sync_all().map_err(|err| err.to_string())?;
    Ok(())
}

fn show_error(hwnd: HWND, message: &str) {
    let text = wide(message);
    let caption = wide("usbscan");
    // SAFETY: 字符串在调用期间有效。
    unsafe {
        MessageBoxW(hwnd, text.as_ptr(), caption.as_ptr(), MB_OK | MB_ICONERROR);
    }
}

fn show_about(hwnd: HWND) {
    let text = wide(
        "usbscan 0.1.0\n\
         把任意 USB 摄像头变成文档扫描仪\n\n\
         Copyright (C) 2026 usbscan contributors\n\
         许可证：GPL-3.0-or-later\n\
         本程序不提供任何担保。",
    );
    let caption = wide("关于 usbscan");
    // SAFETY: 字符串在调用期间有效。
    unsafe {
        MessageBoxW(
            hwnd,
            text.as_ptr(),
            caption.as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

// ---------------------------------------------------------------------------
// 预览区（自绘占位）
// ---------------------------------------------------------------------------

unsafe extern "system" fn preview_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: 标准绘制流程。
    unsafe {
        match msg {
            WM_PAINT => {
                paint_preview(hwnd);
                0
            }
            WM_ERASEBKGND => 1,
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

fn paint_preview(hwnd: HWND) {
    let dark = DARK_MODE.load(Ordering::Relaxed);
    // SAFETY: GetParent 与读取窗口附加数据均为只读操作。
    let parent = unsafe { GetParent(hwnd) };
    let state = state_ptr(parent);
    // SAFETY: state 可能为空；非空时由主窗口持有，绘制期间保持有效。
    let (frame, dpi_now, has_error, connected) = unsafe {
        if state.is_null() {
            (None, dpi::BASE_DPI, false, false)
        } else {
            let s = &*state;
            (
                s.capture.latest_frame(),
                s.dpi,
                s.camera_error.is_some(),
                s.camera_name.is_some(),
            )
        }
    };

    // SAFETY: BeginPaint/EndPaint 成对使用；GDI 对象成对创建/删除。
    unsafe {
        let mut ps: windows_sys::Win32::Graphics::Gdi::PAINTSTRUCT = std::mem::zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);
        if hdc.is_null() {
            EndPaint(hwnd, &ps);
            return;
        }

        let mut rc: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut rc);

        let background = if dark {
            rgb(0x1e, 0x1f, 0x22)
        } else {
            GetSysColor(COLOR_APPWORKSPACE)
        };
        let brush = CreateSolidBrush(background);
        FillRect(hdc, &rc, brush);
        DeleteObject(brush as HGDIOBJ);

        if let Some(frame) = frame {
            draw_frame(hdc, &rc, &frame, dpi_now);
        } else {
            draw_placeholder(hwnd, hdc, &rc, dark, dpi_now, has_error, connected);
        }

        EndPaint(hwnd, &ps);
    }
}

/// 按"适应窗口"缩放绘制一帧，保持宽高比。
fn draw_frame(hdc: HDC, rc: &RECT, frame: &capture::Frame, dpi_now: u32) {
    let pad = i64::from(dpi::scale(12, dpi_now));
    let area_w = i64::from(rc.right - rc.left) - pad * 2;
    let area_h = i64::from(rc.bottom - rc.top) - pad * 2;
    let frame_w = i64::from(frame.width);
    let frame_h = i64::from(frame.height);
    if area_w <= 0 || area_h <= 0 || frame_w <= 0 || frame_h <= 0 {
        return;
    }

    let (draw_w, draw_h) = if area_w * frame_h <= area_h * frame_w {
        (area_w, (area_w * frame_h / frame_w).max(1))
    } else {
        ((area_h * frame_w / frame_h).max(1), area_h)
    };
    let draw_x = i64::from(rc.left) + (i64::from(rc.right - rc.left) - draw_w) / 2;
    let draw_y = i64::from(rc.top) + (i64::from(rc.bottom - rc.top) - draw_h) / 2;

    // SAFETY: BITMAPINFO 为全零可用的 POD 结构。
    let mut info: BITMAPINFO = unsafe { std::mem::zeroed() };
    info.bmiHeader.biSize = u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>()).unwrap_or(40);
    info.bmiHeader.biWidth = frame.width as i32;
    // 负高度表示 top-down，与采集帧的内存布局一致。
    info.bmiHeader.biHeight = -(frame.height as i32);
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = 0; // BI_RGB

    // SAFETY: 像素缓冲区由 Frame 持有，大小与宽高一致；绘制调用只读取它。
    unsafe {
        SetStretchBltMode(hdc, HALFTONE);
        SetBrushOrgEx(hdc, 0, 0, ptr::null_mut());
        StretchDIBits(
            hdc,
            draw_x as i32,
            draw_y as i32,
            draw_w as i32,
            draw_h as i32,
            0,
            0,
            frame.width as i32,
            frame.height as i32,
            frame.data.as_ptr().cast(),
            &info,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
    }
}

fn draw_placeholder(
    hwnd: HWND,
    hdc: HDC,
    rc: &RECT,
    dark: bool,
    dpi_now: u32,
    has_error: bool,
    connected: bool,
) {
    let (line1, line2) = if has_error {
        ("摄像头不可用", "请检查设备连接，或按 F5 重新检测设备")
    } else if connected {
        ("正在启动预览…", "首帧到达前请稍候")
    } else {
        ("等待摄像头", "插上任一 USB 摄像头即可开始")
    };

    // SAFETY: 绘制期间字体句柄由主窗口持有，SelectObject 恢复原对象。
    unsafe {
        SetBkMode(hdc, 1);
        let text_color = if dark {
            rgb(0xe6, 0xe8, 0xeb)
        } else {
            GetSysColor(COLOR_WINDOWTEXT)
        };
        SetTextColor(hdc, text_color);

        let font = SendMessageW(GetParent(hwnd), WM_GETFONT, 0, 0) as HGDIOBJ;
        let old_font = if font.is_null() {
            ptr::null_mut()
        } else {
            SelectObject(hdc, font)
        };

        let margin = dpi::scale(24, dpi_now);
        let mut inner = *rc;
        inner.left += margin;
        inner.top += margin;
        inner.right -= margin;
        inner.bottom -= margin;
        let pen_color = if dark {
            rgb(0x4a, 0x4d, 0x55)
        } else {
            rgb(0xc4, 0xc7, 0xcd)
        };
        let pen = CreatePen(PS_DOT, 1, pen_color);
        let old_pen = SelectObject(hdc, pen);
        let null_brush = GetStockObject(NULL_BRUSH_INDEX);
        let old_brush = SelectObject(hdc, null_brush);
        Rectangle(hdc, inner.left, inner.top, inner.right, inner.bottom);
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
        DeleteObject(pen as HGDIOBJ);

        let mut text_rect = *rc;
        text_rect.top = rc.top + (rc.bottom - rc.top) / 2 - dpi::scale(24, dpi_now);
        let line1 = wide(line1);
        DrawTextW(
            hdc,
            line1.as_ptr(),
            -1,
            &mut text_rect,
            DT_CENTER | DT_SINGLELINE | DT_NOPREFIX,
        );

        let mut sub_rect = *rc;
        sub_rect.top = text_rect.top + dpi::scale(34, dpi_now);
        let line2 = wide(line2);
        let muted = if dark {
            rgb(0x9a, 0x9f, 0xa8)
        } else {
            rgb(0x60, 0x63, 0x6a)
        };
        SetTextColor(hdc, muted);
        DrawTextW(
            hdc,
            line2.as_ptr(),
            -1,
            &mut sub_rect,
            DT_CENTER | DT_SINGLELINE | DT_NOPREFIX,
        );

        if !old_font.is_null() {
            SelectObject(hdc, old_font);
        }
    }
}

const fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_frame_as_jpeg() {
        let frame = capture::Frame {
            width: 2,
            height: 2,
            format: capture::PixelFormat::Bgra8,
            data: vec![0x40; 2 * 2 * 4],
            timestamp: 0,
        };
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/usbscan-win-tests")
            .join(format!("{}-{}", std::process::id(), frame.timestamp));
        fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("frame.jpg");

        let file = fs::File::create(&path).expect("create file");
        write_jpeg(file, &frame).expect("write jpeg");

        let bytes = fs::read(&path).expect("read back");
        assert!(bytes.starts_with(&[0xFF, 0xD8]), "JPEG SOI");
        assert!(bytes.ends_with(&[0xFF, 0xD9]), "JPEG EOI");
        let _ = fs::remove_dir_all(&dir);
    }
}
