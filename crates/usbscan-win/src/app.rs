// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 主窗口与应用生命周期。
//!
//! M0-1 骨架 + M0-2 采集链路：原生菜单栏、工具栏、状态栏、
//! 缩略图面板、预览区与增强面板，并已接入 Media Foundation 实时取帧。

use std::fmt::Write as _;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use usbscan_capture as capture;
use usbscan_doc as doc;
use usbscan_export as export;
use usbscan_imaging as imaging;
use usbscan_naming as naming;
use usbscan_ocr as ocr;

use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontIndirectW, CreatePen,
    CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect, GetStockObject,
    GetSysColor, GetSysColorBrush, InvalidateRect, Rectangle, SelectObject, SetBkColor, SetBkMode,
    SetStretchBltMode, SetTextColor, StretchDIBits, UpdateWindow, BITMAPINFO, BITMAPINFOHEADER,
    COLORONCOLOR, COLOR_APPWORKSPACE, COLOR_BTNFACE, COLOR_BTNTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT,
    DIB_RGB_COLORS, HBITMAP, HBRUSH, HDC, HGDIOBJ, PS_DOT, SRCCOPY,
};
use windows_sys::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_APARTMENTTHREADED};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, GetSaveFileNameW, OFN_ALLOWMULTISELECT, OFN_EXPLORER, OFN_FILEMUSTEXIST,
    OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::Controls::{
    GetWindowTheme, InitCommonControlsEx, CDDS_ITEMPREPAINT, CDDS_PREPAINT, CDRF_NEWFONT,
    CDRF_NOTIFYITEMDRAW, HIMAGELIST, ICC_BAR_CLASSES, ICC_LISTVIEW_CLASSES, ICC_STANDARD_CLASSES,
    INITCOMMONCONTROLSEX, NMCUSTOMDRAW, NMHDR, NMLVKEYDOWN, NM_CUSTOMDRAW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, VK_DELETE};
use windows_sys::Win32::UI::Shell::{
    SHBrowseForFolderW, SHGetPathFromIDListW, ShellExecuteW, BIF_NEWDIALOGSTYLE,
    BIF_RETURNONLYFSDIRS, BROWSEINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CheckMenuItem, CreateWindowExW, DefWindowProcW, DestroyAcceleratorTable, DestroyWindow,
    DispatchMessageW, EnableMenuItem, GetClassNameW, GetClientRect, GetMenu, GetMessageW,
    GetParent, GetSubMenu, GetWindowLongPtrW, GetWindowRect, KillTimer, LoadCursorW, MessageBoxW,
    PostQuitMessage, RegisterClassW, SendMessageW, SetTimer, SetWindowLongPtrW, SetWindowPos,
    SetWindowTextW, ShowWindow, SystemParametersInfoW, TranslateAcceleratorW, TranslateMessage,
    CREATESTRUCTW, CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, GWL_STYLE, IDC_ARROW,
    MB_ICONERROR, MB_ICONINFORMATION, MB_ICONQUESTION, MB_ICONWARNING, MB_OK, MB_YESNO,
    MF_BYCOMMAND, MF_CHECKED, MF_ENABLED, MF_GRAYED, MINMAXINFO, MSG, NONCLIENTMETRICSW,
    SPI_GETNONCLIENTMETRICS, SWP_NOACTIVATE, SWP_NOZORDER, SW_SHOWDEFAULT, SW_SHOWNORMAL, WM_CLOSE,
    WM_COMMAND, WM_CREATE, WM_CTLCOLORBTN, WM_CTLCOLOREDIT, WM_CTLCOLORLISTBOX, WM_CTLCOLORSTATIC,
    WM_DESTROY, WM_DEVICECHANGE, WM_DPICHANGED, WM_ERASEBKGND, WM_GETFONT, WM_GETMINMAXINFO,
    WM_HSCROLL, WM_NCDESTROY, WM_NOTIFY, WM_PAINT, WM_SETFONT, WM_SETTINGCHANGE, WM_SIZE, WM_TIMER,
    WNDCLASSW, WS_CHILD, WS_CLIPCHILDREN, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

use crate::capture_host::{
    CaptureHost, Status as CameraStatus, WM_APP_DEVICES, WM_APP_FRAME, WM_APP_STATUS,
};
use crate::config::Config;
use crate::controls::{self, ToolButton};
use crate::dpi;
use crate::error::WinError;
use crate::icons::{self, Icon};
use crate::menu;
use crate::text::wide;

const MAIN_CLASS: &str = "UsbscanMainWindow";
const PREVIEW_CLASS: &str = "UsbscanPreviewWindow";

const DT_CENTER: u32 = 0x0000_0001;
const DT_SINGLELINE: u32 = 0x0000_0020;
const DT_NOPREFIX: u32 = 0x0000_0800;
const DT_END_ELLIPSIS: u32 = 0x0000_8000;

const NULL_BRUSH_INDEX: i32 = 5;

/// 工具栏"拍照"按钮与 F9 加速键共用的命令 ID。
const CMD_CAPTURE: usize = menu::id::CAPTURE;
/// 工具栏"导出"按钮的命令 ID。
const CMD_EXPORT: usize = 2008;
/// 设备下拉框 ID。
const CMD_DEVICE_COMBO: usize = 2300;

/// 扫描设置控件 ID。
const CMD_SETTINGS_OUTPUT: usize = 2400;
const CMD_SETTINGS_BROWSE: usize = 2401;
const CMD_SETTINGS_FILE: usize = 2402;
const CMD_SETTINGS_FOLDER: usize = 2403;
const CMD_SETTINGS_COLLISION: usize = 2404;
const CMD_SETTINGS_QUALITY: usize = 2405;
const CMD_SETTINGS_APPLY: usize = 2406;
/// 增强复选框起始 ID（2700..2703）。
const CMD_ENHANCE_CHECK: usize = 2700;
/// 增强强度轨道条 ID。
const CMD_ENHANCE_TRACK: usize = 2800;

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
/// 编辑框内容变化（`EN_CHANGE`）。
const EN_CHANGE: u32 = 0x0300;
/// 设备树发生变化（`DBT_DEVNODES_CHANGED`）。
const DBT_DEVNODES_CHANGED: usize = 0x0007;
/// 列表选择变化（`LVN_ITEMCHANGED`）。
const LVN_ITEMCHANGED: i32 = -101;
/// 列表双击（`NM_DBLCLK`）。
const NM_DBLCLK: i32 = -3;
/// 列表按键（`LVN_KEYDOWN`）。
const LVN_KEYDOWN: i32 = -155;

/// 供预览子窗口读取的深色模式缓存（注册表读取不宜每帧进行）。
static DARK_MODE: AtomicBool = AtomicBool::new(false);

/// 预览区离屏缓冲，避免每帧先擦背景再绘制造成闪烁。
struct PreviewBuffer {
    dc: HDC,
    bitmap: HBITMAP,
    old_bitmap: HGDIOBJ,
    width: i32,
    height: i32,
}

impl Default for PreviewBuffer {
    fn default() -> Self {
        Self {
            dc: ptr::null_mut(),
            bitmap: ptr::null_mut(),
            old_bitmap: ptr::null_mut(),
            width: 0,
            height: 0,
        }
    }
}

impl PreviewBuffer {
    /// 确保离屏缓冲尺寸与窗口一致；尺寸未变时复用。
    fn ensure(&mut self, reference: HDC, width: i32, height: i32) -> bool {
        if width <= 0 || height <= 0 {
            return false;
        }
        if !self.dc.is_null() && self.width == width && self.height == height {
            return true;
        }
        self.release();

        // SAFETY: reference 是有效窗口 DC；返回的 GDI 对象由 release 释放。
        unsafe {
            let dc = CreateCompatibleDC(reference);
            let bitmap = CreateCompatibleBitmap(reference, width, height);
            if dc.is_null() || bitmap.is_null() {
                if !dc.is_null() {
                    DeleteDC(dc);
                }
                if !bitmap.is_null() {
                    DeleteObject(bitmap as HGDIOBJ);
                }
                return false;
            }
            self.old_bitmap = SelectObject(dc, bitmap);
            self.dc = dc;
            self.bitmap = bitmap;
            self.width = width;
            self.height = height;
        }
        true
    }

    /// 释放离屏缓冲。
    fn release(&mut self) {
        // SAFETY: 句柄只由本结构创建，且释放后立即置空。
        unsafe {
            if !self.dc.is_null() {
                if !self.old_bitmap.is_null() {
                    SelectObject(self.dc, self.old_bitmap);
                }
                DeleteDC(self.dc);
            }
            if !self.bitmap.is_null() {
                DeleteObject(self.bitmap as HGDIOBJ);
            }
        }
        *self = Self::default();
    }
}

/// 应用主状态。通过 `GWLP_USERDATA` 与窗口关联。
struct AppState {
    hinstance: HINSTANCE,
    hwnd: HWND,
    dpi: u32,
    dark: bool,
    font: usize,
    toolbar: HWND,
    toolbar_icons: HIMAGELIST,
    device_label: HWND,
    device_combo: HWND,
    pages: HWND,
    preview: HWND,
    preview_buffer: PreviewBuffer,
    status: HWND,
    settings_group: HWND,
    settings_heading: HWND,
    settings_output_label: HWND,
    settings_output: HWND,
    settings_browse: HWND,
    settings_file_label: HWND,
    settings_file: HWND,
    settings_folder_label: HWND,
    settings_folder: HWND,
    settings_collision_label: HWND,
    settings_collision: HWND,
    settings_quality_label: HWND,
    settings_quality_value: HWND,
    settings_track: HWND,
    settings_preview: HWND,
    settings_apply: HWND,
    enhance_group: HWND,
    enhance_heading: HWND,
    enhance_checks: [HWND; 5],
    enhance_label: HWND,
    enhance_value: HWND,
    enhance_track: HWND,
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
    collision: naming::Collision,
    jpeg_quality: u8,
    enhance: imaging::EnhanceOptions,
    ocr_language: String,
    tesseract_path: Option<PathBuf>,
    output_root: PathBuf,
    session: doc::Session,
    store: doc::SessionStore,
    thumbs: HIMAGELIST,
    thumb_width: i32,
    thumb_height: i32,
    /// 页面列表中选中的页；`None` 表示未选中。
    selected_page: Option<usize>,
    /// 最近一次拍摄；非空表示 3 秒撤销窗口内。
    undo_page: Option<doc::Page>,
}

/// 启动应用：创建窗口并进入消息循环。
///
/// 返回时表示用户已关闭主窗口；错误仅在初始化阶段产生。
#[allow(clippy::too_many_lines)]
pub fn run() -> Result<(), WinError> {
    // UI 线程使用 STA：系统文件夹选择框和 Shell 对话框都依赖它。
    // SAFETY: 每个线程只初始化一次；返回 RPC_E_CHANGED_MODE 时也无妨。
    unsafe {
        let _ = CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32);
    }

    let dark = crate::theme::system_uses_dark_mode();
    DARK_MODE.store(dark, Ordering::Relaxed);
    crate::theme::init_process(dark);

    init_common_controls();
    register_window_classes()?;

    let hinstance = module_handle();
    let menu_bar = menu::build_main_menu()?;
    let accelerators = menu::build_accelerators()?;
    let config = Config::load();
    let output_root = config
        .output_root
        .clone()
        .filter(|path| path.is_absolute())
        .unwrap_or_else(default_output_root);
    let store = doc::SessionStore::new(output_root.clone());
    let mut rule = naming::NamingRule::default();
    if let Some(template) = &config.file_template {
        rule.template.clone_from(template);
    }
    if rule.template.trim().is_empty() {
        rule.template = naming::NamingRule::default().template;
    }
    if let Some(folder) = &config.folder_template {
        rule.folder.clone_from(folder);
    }
    let collision = match config.collision.as_deref() {
        Some("skip") => naming::Collision::Skip,
        Some("overwrite") => naming::Collision::Overwrite,
        _ => naming::Collision::Append,
    };
    let jpeg_quality = config.jpeg_quality.unwrap_or(92).clamp(1, 100);
    let enhance = imaging::EnhanceOptions {
        auto_crop: config.auto_crop.unwrap_or(false),
        perspective: config.perspective.unwrap_or(false),
        deskew: config.deskew.unwrap_or(false),
        remove_shadow: config.remove_shadow.unwrap_or(false),
        white_background: config.white_background.unwrap_or(false),
        strength: config.enhance_strength.unwrap_or(60).min(100),
    };
    let ocr_language = config
        .ocr_language
        .clone()
        .unwrap_or_else(|| ocr::DEFAULT_LANGUAGE.to_string());
    let tesseract_path = config.tesseract_path.clone().map(PathBuf::from);

    let state = Box::new(AppState {
        hinstance,
        hwnd: ptr::null_mut(),
        dpi: dpi::BASE_DPI,
        dark,
        font: 0,
        toolbar: ptr::null_mut(),
        toolbar_icons: 0,
        device_label: ptr::null_mut(),
        device_combo: ptr::null_mut(),
        pages: ptr::null_mut(),
        preview: ptr::null_mut(),
        preview_buffer: PreviewBuffer::default(),
        status: ptr::null_mut(),
        settings_group: ptr::null_mut(),
        settings_heading: ptr::null_mut(),
        settings_output_label: ptr::null_mut(),
        settings_output: ptr::null_mut(),
        settings_browse: ptr::null_mut(),
        settings_file_label: ptr::null_mut(),
        settings_file: ptr::null_mut(),
        settings_folder_label: ptr::null_mut(),
        settings_folder: ptr::null_mut(),
        settings_collision_label: ptr::null_mut(),
        settings_collision: ptr::null_mut(),
        settings_quality_label: ptr::null_mut(),
        settings_quality_value: ptr::null_mut(),
        settings_track: ptr::null_mut(),
        settings_preview: ptr::null_mut(),
        settings_apply: ptr::null_mut(),
        enhance_group: ptr::null_mut(),
        enhance_heading: ptr::null_mut(),
        enhance_checks: [ptr::null_mut(); 5],
        enhance_label: ptr::null_mut(),
        enhance_value: ptr::null_mut(),
        enhance_track: ptr::null_mut(),
        panel_brush: ptr::null_mut(),
        status_msg: "就绪".to_string(),
        capture: CaptureHost::new(),
        camera_name: None,
        camera_size: None,
        camera_error: None,
        camera_waiting: None,
        active_id: None,
        selected_id: config.last_camera_id.clone(),
        selected_name: config.last_camera_name.clone(),
        devices: Vec::new(),
        devices_version: 0,
        naming: naming::NamingEngine::default(),
        rule,
        collision,
        jpeg_quality,
        enhance,
        ocr_language,
        tesseract_path,
        output_root,
        session: doc::Session::default(),
        store,
        thumbs: 0,
        thumb_width: 0,
        thumb_height: 0,
        selected_page: None,
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
            // 焦点在编辑框/下拉框时，把按键交给控件本身，
            // 避免 Ctrl+Z、Delete 等全局快捷键干扰文本编辑。
            let focus = GetFocus();
            let mut editing = false;
            if !focus.is_null() {
                let mut class_name = [0u16; 32];
                let len = GetClassNameW(focus, class_name.as_mut_ptr(), 32);
                if len > 0 {
                    let name = String::from_utf16_lossy(&class_name[..len as usize]);
                    editing = name == "Edit" || name == "ComboBox";
                }
            }
            if editing || TranslateAcceleratorW(hwnd, accelerators, &msg) == 0 {
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

    let classes: [(&str, WndprocAlias); 5] = [
        (MAIN_CLASS, wndproc),
        (PREVIEW_CLASS, preview_proc),
        (crate::corners::CORNER_CLASS, crate::corners::wndproc),
        (crate::text_edit::TEXT_EDIT_CLASS, crate::text_edit::wndproc),
        (crate::annotate::ANNOTATE_CLASS, crate::annotate::wndproc),
    ];

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
                    (*state).capture.shared().frame_pending = false;
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
            WM_HSCROLL => {
                let state = state_ptr(hwnd);
                if !state.is_null() {
                    let s = &mut *state;
                    if lparam == s.settings_track as isize {
                        let value = controls::track_position(s.settings_track);
                        controls::set_text(s.settings_quality_value, &value.to_string());
                    } else if lparam == s.enhance_track as isize {
                        let value = controls::track_position(s.enhance_track);
                        controls::set_text(s.enhance_value, &value.to_string());
                    }
                }
                0
            }
            WM_NOTIFY => {
                let state = state_ptr(hwnd);
                if !state.is_null() {
                    let s = &mut *state;
                    let header = &*(lparam as *const NMHDR);
                    if header.hwndFrom == s.status && header.code == NM_CUSTOMDRAW {
                        let custom = &mut *(lparam as *mut NMCUSTOMDRAW);
                        if custom.dwDrawStage == CDDS_PREPAINT {
                            let brush = if s.dark {
                                s.panel_brush
                            } else {
                                GetSysColorBrush(COLOR_BTNFACE)
                            };
                            FillRect(custom.hdc, &custom.rc, brush);
                            return CDRF_NOTIFYITEMDRAW as LRESULT;
                        }
                        if custom.dwDrawStage == CDDS_ITEMPREPAINT {
                            let (fg, bg, brush) = if s.dark {
                                (rgb(0xe6, 0xe8, 0xeb), rgb(0x1e, 0x1f, 0x22), s.panel_brush)
                            } else {
                                (
                                    GetSysColor(COLOR_BTNTEXT),
                                    GetSysColor(COLOR_BTNFACE),
                                    GetSysColorBrush(COLOR_BTNFACE),
                                )
                            };
                            FillRect(custom.hdc, &custom.rc, brush);
                            SetTextColor(custom.hdc, fg);
                            SetBkColor(custom.hdc, bg);
                            return CDRF_NEWFONT as LRESULT;
                        }
                        return 0;
                    }
                    if header.hwndFrom == s.pages {
                        match header.code as i32 {
                            LVN_ITEMCHANGED => update_page_selection(s),
                            NM_DBLCLK => {
                                update_page_selection(s);
                                open_selected_page(hwnd);
                            }
                            LVN_KEYDOWN => {
                                let key = &*(lparam as *const NMLVKEYDOWN);
                                if key.wVKey == VK_DELETE {
                                    delete_selected_page(hwnd);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                0
            }
            WM_DPICHANGED => {
                let state = state_ptr(hwnd);
                if !state.is_null() {
                    let new_dpi = (wparam & 0xFFFF) as u32;
                    (*state).dpi = new_dpi;
                    rebuild_font(state);
                    rebuild_toolbar_icons(&mut *state);
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
            WM_CTLCOLORSTATIC => {
                let control = lparam as HWND;
                if is_combo(control) || is_control_class(control, "Edit") {
                    edit_color(hwnd, wparam, control)
                } else {
                    control_color(hwnd, wparam)
                }
            }
            WM_CTLCOLORBTN => button_color(hwnd, wparam, lparam as HWND),
            WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => edit_color(hwnd, wparam, lparam as HWND),
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
                    controls::destroy_image_list((*state).toolbar_icons);
                    (*state).preview_buffer.release();
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
/// 为编辑框/下拉框提供颜色。
///
/// 主题化控件由 Windows 同时绘制文字和背景；这时不要只改文字颜色，
/// 否则在部分 Win10 版本上会出现“白字白底”。只有控件没有主题时才使用
/// 我们自己的深色配色。
fn edit_color(hwnd: HWND, wparam: WPARAM, control: HWND) -> LRESULT {
    let state = state_ptr(hwnd);
    if state.is_null() {
        return unsafe { GetSysColorBrush(COLOR_WINDOW) as isize };
    }
    // SAFETY: state 有效；HDC 由系统在 WM_CTLCOLOR* 中传入。
    unsafe {
        if !control.is_null() && GetWindowTheme(control) != 0 {
            return GetSysColorBrush(COLOR_WINDOW) as isize;
        }
        let s = &*state;
        let hdc = wparam as HDC;
        let (fg, bg) = if s.dark {
            (rgb(0xe6, 0xe8, 0xeb), rgb(0x1e, 0x1f, 0x22))
        } else {
            (GetSysColor(COLOR_WINDOWTEXT), GetSysColor(COLOR_WINDOW))
        };
        SetTextColor(hdc, fg);
        SetBkColor(hdc, bg);
        if s.dark {
            s.panel_brush as isize
        } else {
            GetSysColorBrush(COLOR_WINDOW) as isize
        }
    }
}

/// 按钮/分组框颜色。按钮交给主题绘制；分组框标题跟随深色面板。
fn button_color(hwnd: HWND, wparam: WPARAM, control: HWND) -> LRESULT {
    let state = state_ptr(hwnd);
    if state.is_null() {
        return unsafe { GetSysColorBrush(COLOR_BTNFACE) as isize };
    }
    // SAFETY: state 有效；HDC 由系统在 WM_CTLCOLORBTN 中传入。
    unsafe {
        let s = &*state;
        if !control.is_null() {
            let style = GetWindowLongPtrW(control, GWL_STYLE) as u32;
            if style & 0x0F == 0x07 {
                // BS_GROUPBOX：标题需要和深色面板形成对比。
                return control_color(hwnd, wparam);
            }
            if GetWindowTheme(control) != 0 {
                // 主题按钮自行绘制文字和背景。
                return GetSysColorBrush(COLOR_BTNFACE) as isize;
            }
        }
        let hdc = wparam as HDC;
        let (fg, bg) = if s.dark {
            (rgb(0xe6, 0xe8, 0xeb), rgb(0x1e, 0x1f, 0x22))
        } else {
            (GetSysColor(COLOR_BTNTEXT), GetSysColor(COLOR_BTNFACE))
        };
        SetTextColor(hdc, fg);
        SetBkColor(hdc, bg);
        if s.dark {
            s.panel_brush as isize
        } else {
            GetSysColorBrush(COLOR_BTNFACE) as isize
        }
    }
}

fn is_control_class(control: HWND, expected: &str) -> bool {
    if control.is_null() {
        return false;
    }
    let mut buffer = [0u16; 32];
    // SAFETY: 缓冲区与窗口句柄有效。
    let len = unsafe { GetClassNameW(control, buffer.as_mut_ptr(), 32) };
    len > 0 && String::from_utf16_lossy(&buffer[..len as usize]) == expected
}

fn is_combo(control: HWND) -> bool {
    is_control_class(control, "ComboBox")
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
                    id: menu::id::FILE_NEW,
                    label: "新建",
                    separator_before: false,
                    icon: Some(Icon::New),
                },
                ToolButton {
                    id: menu::id::CAPTURE,
                    label: "拍照",
                    separator_before: true,
                    icon: Some(Icon::Capture),
                },
                ToolButton {
                    id: 2002,
                    label: "连拍",
                    separator_before: false,
                    icon: Some(Icon::Burst),
                },
                ToolButton {
                    id: 2003,
                    label: "定时",
                    separator_before: false,
                    icon: Some(Icon::Delay),
                },
                ToolButton {
                    id: 2004,
                    label: "录像",
                    separator_before: false,
                    icon: Some(Icon::Video),
                },
                ToolButton {
                    id: 2005,
                    label: "裁边",
                    separator_before: true,
                    icon: Some(Icon::Crop),
                },
                ToolButton {
                    id: 2006,
                    label: "纠偏",
                    separator_before: false,
                    icon: Some(Icon::Deskew),
                },
                ToolButton {
                    id: 2007,
                    label: "识别",
                    separator_before: true,
                    icon: Some(Icon::Ocr),
                },
                ToolButton {
                    id: 2008,
                    label: "导出",
                    separator_before: true,
                    icon: Some(Icon::Export),
                },
                ToolButton {
                    id: menu::id::FILE_REVEAL,
                    label: "位置",
                    separator_before: false,
                    icon: Some(Icon::Reveal),
                },
                ToolButton {
                    id: 2009,
                    label: "打印",
                    separator_before: true,
                    icon: Some(Icon::Print),
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

        s.settings_group = controls::create_group(s.hwnd, s.hinstance, "");
        s.settings_heading = controls::create_label(s.hwnd, s.hinstance, "扫描设置");
        s.settings_output_label = controls::create_label(s.hwnd, s.hinstance, "输出目录");
        s.settings_output = controls::create_edit(s.hwnd, s.hinstance, CMD_SETTINGS_OUTPUT);
        s.settings_browse =
            controls::create_push_button(s.hwnd, s.hinstance, CMD_SETTINGS_BROWSE, "浏览…");
        s.settings_file_label = controls::create_label(s.hwnd, s.hinstance, "文件名模板");
        s.settings_file = controls::create_edit(s.hwnd, s.hinstance, CMD_SETTINGS_FILE);
        s.settings_folder_label = controls::create_label(s.hwnd, s.hinstance, "子目录模板");
        s.settings_folder = controls::create_edit(s.hwnd, s.hinstance, CMD_SETTINGS_FOLDER);
        s.settings_collision_label = controls::create_label(s.hwnd, s.hinstance, "重名策略");
        s.settings_collision =
            controls::create_combo_box(s.hwnd, s.hinstance, CMD_SETTINGS_COLLISION);
        let collision_items = [
            "追加序号".to_string(),
            "跳过".to_string(),
            "覆盖".to_string(),
        ];
        let collision_index = match s.collision {
            naming::Collision::Skip => 1,
            naming::Collision::Overwrite => 2,
            _ => 0,
        };
        controls::set_combo_items(
            s.settings_collision,
            &collision_items,
            Some(collision_index),
        );
        s.settings_quality_label = controls::create_label(s.hwnd, s.hinstance, "JPEG 质量");
        s.settings_quality_value = controls::create_label(s.hwnd, s.hinstance, "92");
        s.settings_track = controls::create_trackbar(s.hwnd, s.hinstance, CMD_SETTINGS_QUALITY);
        controls::set_track_position(s.settings_track, i32::from(s.jpeg_quality));
        s.settings_preview = controls::create_label(s.hwnd, s.hinstance, "示例：生成中…");
        s.settings_apply =
            controls::create_push_button(s.hwnd, s.hinstance, CMD_SETTINGS_APPLY, "应用设置");

        s.enhance_group = controls::create_group(s.hwnd, s.hinstance, "");
        s.enhance_heading = controls::create_label(s.hwnd, s.hinstance, "图像增强");
        let enhance_labels = ["自动裁边", "自动纠偏", "透视校正", "去阴影", "白底净化"];
        for (index, label) in enhance_labels.iter().enumerate() {
            s.enhance_checks[index] =
                controls::create_checkbox(s.hwnd, s.hinstance, CMD_ENHANCE_CHECK + index, label);
        }
        s.enhance_label = controls::create_label(s.hwnd, s.hinstance, "强度");
        s.enhance_value = controls::create_label(s.hwnd, s.hinstance, "60");
        s.enhance_track = controls::create_trackbar(s.hwnd, s.hinstance, CMD_ENHANCE_TRACK);
        controls::set_track_position(s.enhance_track, i32::from(s.enhance.strength));
        controls::set_text(s.enhance_value, &s.enhance.strength.to_string());
        for (control, checked) in s.enhance_checks.iter().zip([
            s.enhance.auto_crop,
            s.enhance.deskew,
            s.enhance.perspective,
            s.enhance.remove_shadow,
            s.enhance.white_background,
        ]) {
            controls::set_checkbox(*control, checked);
        }

        controls::set_text(s.settings_output, &s.output_root.to_string_lossy());
        controls::set_text(s.settings_file, &s.rule.template);
        controls::set_text(s.settings_folder, &s.rule.folder);
        controls::set_text(s.settings_quality_value, &s.jpeg_quality.to_string());
        refresh_settings_preview(s);

        s.status = controls::create_status_bar(s.hwnd, s.hinstance);
        controls::set_status_min_height(s.status, scale(24));
        controls::set_status_text(s.status, 0, "● 未连接设备");
        controls::set_status_text(s.status, 1, "0 页");
        controls::set_status_text(s.status, 2, "就绪");
        controls::set_status_text(s.status, 3, "v0.1.0");

        apply_font_to_children(s);
        apply_theme(state);
        set_undo_enabled(s.hwnd, false);
        set_delete_page_enabled(s.hwnd, false);
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
            state.settings_group,
            state.settings_heading,
            state.settings_output_label,
            state.settings_output,
            state.settings_browse,
            state.settings_file_label,
            state.settings_file,
            state.settings_folder_label,
            state.settings_folder,
            state.settings_collision_label,
            state.settings_collision,
            state.settings_quality_label,
            state.settings_quality_value,
            state.settings_track,
            state.settings_preview,
            state.settings_apply,
            state.enhance_group,
            state.enhance_heading,
            state.enhance_checks[0],
            state.enhance_checks[1],
            state.enhance_checks[2],
            state.enhance_checks[4],
            state.enhance_label,
            state.enhance_value,
            state.enhance_track,
        ] {
            if !child.is_null() {
                SendMessageW(child, WM_SETFONT, handle, 1);
            }
        }
    }
    controls::autosize_toolbar(state.toolbar);
}

/// 按当前主题和 DPI 重新生成工具栏图标。
fn rebuild_toolbar_icons(s: &mut AppState) {
    controls::destroy_image_list(s.toolbar_icons);
    let color = if s.dark {
        rgb(0xe6, 0xe8, 0xeb)
    } else {
        // SAFETY: GetSysColor 是纯查询调用。
        unsafe { GetSysColor(COLOR_WINDOWTEXT) }
    };
    s.toolbar_icons = icons::create_toolbar_icons(dpi::scale(24, s.dpi), color);
    controls::set_toolbar_images(s.toolbar, s.toolbar_icons);
    controls::autosize_toolbar(s.toolbar);
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
            s.settings_group,
            s.settings_heading,
            s.settings_output_label,
            s.settings_output,
            s.settings_browse,
            s.settings_file_label,
            s.settings_file,
            s.settings_folder_label,
            s.settings_folder,
            s.settings_collision_label,
            s.settings_collision,
            s.settings_quality_label,
            s.settings_quality_value,
            s.settings_track,
            s.settings_preview,
            s.settings_apply,
            s.enhance_group,
            s.enhance_heading,
            s.enhance_checks[0],
            s.enhance_checks[1],
            s.enhance_checks[2],
            s.enhance_checks[4],
            s.enhance_label,
            s.enhance_value,
            s.enhance_track,
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
        controls::set_status_background(s.status, panel_bg);
        rebuild_toolbar_icons(s);

        if s.dark {
            // 编辑框/下拉框使用专门的暗色子主题；不同 Win10 版本对
            // DarkMode_Explorer 的支持不一致，CFD 主题更稳定。
            for control in [
                s.device_combo,
                s.settings_output,
                s.settings_file,
                s.settings_folder,
                s.settings_collision,
            ] {
                crate::theme::apply_named(control, "DarkMode_CFD");
            }
        }

        let bar = GetMenu(s.hwnd);
        if !bar.is_null() {
            let view = GetSubMenu(bar, 3);
            if !view.is_null() {
                let check = if s.dark { MF_CHECKED } else { 0 };
                CheckMenuItem(view, menu::id::VIEW_DARK as u32, check);
            }
        }

        InvalidateRect(s.hwnd, ptr::null(), 1);
        if !s.preview.is_null() {
            InvalidateRect(s.preview, ptr::null(), 1);
        }
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

#[allow(clippy::too_many_lines)]
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
        let panel_x = left_w + preview_w;
        let panel_top = body_top + scale(10);
        place(
            s.settings_group,
            panel_x,
            panel_top,
            right_w,
            (body_h - scale(10)).max(0),
        );

        let pad = scale(16);
        let x = panel_x + pad;
        let inner_w = right_w - pad * 2;
        let label_h = scale(18);
        let edit_h = scale(24);
        let gap = scale(8);
        let heading_y = panel_top + scale(8);
        place(s.settings_heading, x, heading_y, inner_w, scale(20));
        let mut y = heading_y + scale(28);

        place(s.settings_output_label, x, y, inner_w, label_h);
        y += label_h;
        place(s.settings_output, x, y, inner_w - scale(48), edit_h);
        place(
            s.settings_browse,
            x + inner_w - scale(44),
            y,
            scale(44),
            edit_h,
        );
        y += edit_h + gap;

        place(s.settings_file_label, x, y, inner_w, label_h);
        y += label_h;
        place(s.settings_file, x, y, inner_w, edit_h);
        y += edit_h + gap;

        place(s.settings_folder_label, x, y, inner_w, label_h);
        y += label_h;
        place(s.settings_folder, x, y, inner_w, edit_h);
        y += edit_h + gap;

        place(s.settings_collision_label, x, y, scale(80), label_h);
        place(
            s.settings_collision,
            x + scale(86),
            y - scale(2),
            scale(140),
            scale(200),
        );
        y += label_h + scale(4);

        place(s.settings_quality_label, x, y, scale(90), label_h);
        place(
            s.settings_quality_value,
            x + scale(96),
            y,
            scale(40),
            label_h,
        );
        y += label_h;
        place(s.settings_track, x, y, inner_w, scale(28));
        y += scale(30) + gap;

        place(s.settings_preview, x, y, inner_w, scale(38));
        y += scale(42);
        place(s.settings_apply, x, y, scale(100), scale(28));

        let enhance_y = y + scale(40);
        let enhance_h = ((body_top + body_h) - enhance_y).max(0);
        place(s.enhance_group, panel_x, enhance_y, right_w, enhance_h);
        place(
            s.enhance_heading,
            x,
            enhance_y + scale(8),
            inner_w,
            scale(20),
        );
        let mut checkbox_y = enhance_y + scale(30);
        for check in s.enhance_checks {
            place(check, x, checkbox_y, inner_w, scale(22));
            checkbox_y += scale(22);
        }
        place(s.enhance_label, x, checkbox_y, scale(40), label_h);
        place(
            s.enhance_value,
            x + scale(44),
            checkbox_y,
            scale(40),
            label_h,
        );
        checkbox_y += label_h;
        place(s.enhance_track, x, checkbox_y, inner_w, scale(28));

        let parts = [width * 22 / 100, width * 38 / 100, width * 80 / 100, -1];
        controls::set_status_parts(s.status, &parts);
        refresh_status(s);
    }
}

/// 当前设置控件对应的命名规则。
fn settings_rule(s: &AppState) -> naming::NamingRule {
    naming::NamingRule {
        template: controls::get_text(s.settings_file).trim().to_string(),
        folder: controls::get_text(s.settings_folder).trim().to_string(),
    }
}

/// 刷新设置面板里的文件名示例。
fn refresh_settings_preview(s: &AppState) {
    let rule = settings_rule(s);
    let ctx = naming::NamingContext {
        now: naming::TimeStamp::now(),
        device: s.camera_name.as_deref(),
        seq: s.session.next_seq,
        ..naming::NamingContext::default()
    };
    let text = match s.naming.render(&rule, &ctx) {
        Ok(rendered) => {
            let folder = rendered.folder.to_string_lossy();
            let name = format!("{}.jpg", rendered.file_name);
            if folder.is_empty() {
                format!("示例：{name}")
            } else {
                format!("示例：{folder}/{name}")
            }
        }
        Err(err) => format!("模板错误：{err}"),
    };
    controls::set_text(s.settings_preview, &text);
}

/// 打开系统文件夹选择框，把结果写入输出目录编辑框。
fn browse_output_dir(hwnd: HWND) {
    if let Some(folder) = choose_folder(hwnd, "选择输出目录") {
        let state = state_ptr(hwnd);
        if !state.is_null() {
            // SAFETY: state 有效；仅更新设置控件。
            unsafe {
                controls::set_text((*state).settings_output, &folder.to_string_lossy());
                refresh_settings_preview(&*state);
            }
        }
    }
}

/// 弹出系统文件夹选择框，返回用户选中的目录。
fn choose_folder(hwnd: HWND, title_text: &str) -> Option<PathBuf> {
    let title = wide(title_text);
    let mut display = [0u16; 260];
    // SAFETY: 结构体与缓冲区在调用期间有效。
    unsafe {
        let mut info: BROWSEINFOW = std::mem::zeroed();
        info.hwndOwner = hwnd;
        info.pszDisplayName = display.as_mut_ptr();
        info.lpszTitle = title.as_ptr();
        info.ulFlags = BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE;

        let pidl = SHBrowseForFolderW(&info);
        if pidl.is_null() {
            return None;
        }
        let mut buffer = [0u16; 260];
        let selected = if SHGetPathFromIDListW(pidl, buffer.as_mut_ptr()) != 0 {
            let len = buffer
                .iter()
                .position(|&ch| ch == 0)
                .unwrap_or(buffer.len());
            Some(PathBuf::from(String::from_utf16_lossy(&buffer[..len])))
        } else {
            None
        };
        CoTaskMemFree(pidl.cast());
        selected
    }
}

fn collision_from_index(index: Option<usize>) -> naming::Collision {
    match index {
        Some(1) => naming::Collision::Skip,
        Some(2) => naming::Collision::Overwrite,
        _ => naming::Collision::Append,
    }
}

fn collision_key(collision: naming::Collision) -> &'static str {
    match collision {
        naming::Collision::Skip => "skip",
        naming::Collision::Overwrite => "overwrite",
        _ => "append",
    }
}

/// 把当前内存中的设置写回 `config.toml`。
fn persist_config(s: &AppState) {
    Config {
        last_camera_id: s.selected_id.clone(),
        last_camera_name: s.selected_name.clone(),
        output_root: Some(s.output_root.clone()),
        file_template: Some(s.rule.template.clone()),
        folder_template: Some(s.rule.folder.clone()),
        collision: Some(collision_key(s.collision).to_string()),
        jpeg_quality: Some(s.jpeg_quality),
        auto_crop: Some(s.enhance.auto_crop),
        deskew: Some(s.enhance.deskew),
        perspective: Some(s.enhance.perspective),
        remove_shadow: Some(s.enhance.remove_shadow),
        white_background: Some(s.enhance.white_background),
        enhance_strength: Some(s.enhance.strength),
        tesseract_path: s
            .tesseract_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        ocr_language: Some(s.ocr_language.clone()),
    }
    .save();
}

/// 重置页面列表与缩略图图像列表。
fn reset_page_list(s: &mut AppState) {
    controls::clear_page_list(s.pages);
    controls::destroy_image_list(s.thumbs);
    s.thumbs = controls::create_image_list(s.thumb_width, s.thumb_height);
    if s.thumbs != 0 {
        controls::set_list_image_list(s.pages, s.thumbs);
    }
    s.selected_page = None;
    s.undo_page = None;
    // SAFETY: 主窗口句柄有效；撤销菜单项固定。
    unsafe {
        KillTimer(s.hwnd, UNDO_TIMER);
    }
    set_undo_enabled(s.hwnd, false);
}

/// 读取设置控件、校验并应用。
fn apply_settings(hwnd: HWND) {
    let state = state_ptr(hwnd);
    if state.is_null() {
        return;
    }
    // SAFETY: state 有效；控件与文件调用均在本线程。
    unsafe {
        let s = &mut *state;
        let root_text = controls::get_text(s.settings_output).trim().to_string();
        if root_text.is_empty() {
            show_error(hwnd, "输出目录不能为空");
            return;
        }
        let root = PathBuf::from(root_text);
        if !root.is_absolute() {
            show_error(hwnd, "输出目录必须是绝对路径");
            return;
        }

        let rule = settings_rule(s);
        let ctx = naming::NamingContext {
            now: naming::TimeStamp::now(),
            device: s.camera_name.as_deref(),
            seq: s.session.next_seq,
            ..naming::NamingContext::default()
        };
        let diagnostics = s.naming.validate(&rule, &ctx, Some(&root));
        let errors: Vec<String> = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == naming::Severity::Error)
            .map(|diagnostic| format!("{} {}", diagnostic.code, diagnostic.message))
            .collect();
        if !errors.is_empty() {
            s.status_msg = "命名规则有错误，设置未应用".to_string();
            refresh_status(s);
            show_error(hwnd, &errors.join("\n"));
            return;
        }

        let collision = collision_from_index(controls::combo_selection(s.settings_collision));
        if collision == naming::Collision::Overwrite {
            let text = wide("“覆盖”策略会直接替换同名文件，旧文件无法恢复。\n\n确定使用覆盖策略？");
            let title = wide("确认重名策略");
            if MessageBoxW(
                hwnd,
                text.as_ptr(),
                title.as_ptr(),
                MB_YESNO | MB_ICONWARNING,
            ) != IDYES
            {
                return;
            }
        }
        let quality =
            u8::try_from(controls::track_position(s.settings_track).clamp(50, 100)).unwrap_or(92);
        let enhance = imaging::EnhanceOptions {
            auto_crop: controls::checkbox_checked(s.enhance_checks[0]),
            deskew: controls::checkbox_checked(s.enhance_checks[1]),
            perspective: controls::checkbox_checked(s.enhance_checks[2]),
            remove_shadow: controls::checkbox_checked(s.enhance_checks[3]),
            white_background: controls::checkbox_checked(s.enhance_checks[4]),
            strength: u8::try_from(controls::track_position(s.enhance_track).clamp(0, 100))
                .unwrap_or(60),
        };
        let root_changed = root != s.output_root;

        if root_changed && !s.session.pages.is_empty() {
            let text = wide(
                "更改输出目录将开始一个新文档。\n\n现有图片不会被删除，仍保留在原目录。是否继续？",
            );
            let title = wide("更改输出目录");
            if MessageBoxW(
                hwnd,
                text.as_ptr(),
                title.as_ptr(),
                MB_YESNO | MB_ICONQUESTION,
            ) != IDYES
            {
                return;
            }
            let _ = s.store.archive();
            s.session = doc::Session::default();
            reset_page_list(s);
        }

        s.store = doc::SessionStore::new(root.clone());
        s.output_root = root;
        s.rule = rule;
        s.collision = collision;
        s.jpeg_quality = quality;
        s.enhance = enhance;
        s.session.closed_cleanly = false;
        let _ = s.store.save(&s.session);

        persist_config(s);

        s.status_msg = "设置已应用".to_string();
        refresh_settings_preview(s);
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
            CameraStatus::NoDevices => {
                s.camera_name = None;
                s.camera_size = None;
                s.camera_error = None;
                s.camera_waiting = None;
                s.active_id = None;
                s.status_msg = "未检测到摄像头，请插入 USB 摄像头或按 F5 重新检测".to_string();
            }
            CameraStatus::Ready {
                id,
                name,
                width,
                height,
            } => {
                if s.camera_name.as_deref() != Some(name.as_str())
                    || s.camera_error.is_some()
                    || s.camera_waiting.is_some()
                {
                    s.status_msg = format!("已连接 {name}，按 F9 或点击“拍照”");
                }
                // 实际打开的可能是按名称找回的设备，或自动选择的第一台；
                // 把纠正后的稳定 ID 记住，下次启动直接命中。
                if s.selected_id.as_deref() != Some(id.as_str()) {
                    s.selected_id = Some(id.clone());
                    s.selected_name = Some(name.clone());
                    persist_config(s);
                }
                let index = s.devices.iter().position(|camera| camera.id == id);
                controls::set_combo_index(s.device_combo, index);
                s.camera_name = Some(name);
                s.camera_size = Some((width, height));
                s.camera_error = None;
                s.camera_waiting = None;
                s.active_id = Some(id);
                refresh_settings_preview(s);
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
        persist_config(s);
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

#[allow(clippy::too_many_lines)]
fn on_command(hwnd: HWND, id: usize, code: u32) {
    if id == CMD_DEVICE_COMBO {
        if code == CBN_SELCHANGE {
            on_device_selected(hwnd);
        }
        return;
    }

    if id == CMD_SETTINGS_APPLY {
        apply_settings(hwnd);
        return;
    }
    if id == CMD_SETTINGS_BROWSE {
        browse_output_dir(hwnd);
        return;
    }
    if (id == CMD_SETTINGS_FILE || id == CMD_SETTINGS_FOLDER) && code == EN_CHANGE {
        let state = state_ptr(hwnd);
        if !state.is_null() {
            // SAFETY: state 有效；仅读取控件并更新预览标签。
            refresh_settings_preview(unsafe { &*state });
        }
        return;
    }
    if id == CMD_SETTINGS_OUTPUT || id == CMD_SETTINGS_COLLISION || id == CMD_SETTINGS_QUALITY {
        return;
    }

    if id == menu::id::EDIT_UNDO {
        undo_capture(hwnd);
        return;
    }

    if id == menu::id::EDIT_DELETE_PAGE {
        delete_selected_page(hwnd);
        return;
    }

    if id == menu::id::EDIT_ROTATE_RIGHT {
        transform_selected_page(hwnd, PageTransform::RotateRight);
        return;
    }
    if id == menu::id::EDIT_ROTATE_LEFT {
        transform_selected_page(hwnd, PageTransform::RotateLeft);
        return;
    }
    if id == menu::id::EDIT_MIRROR {
        transform_selected_page(hwnd, PageTransform::Mirror);
        return;
    }
    if id == menu::id::EDIT_REPROCESS {
        reprocess_selected_page(hwnd);
        return;
    }
    if id == menu::id::EDIT_PERSPECTIVE {
        manual_perspective_selected(hwnd);
        return;
    }
    if id == menu::id::EDIT_ANNOTATE {
        annotate_selected_page(hwnd);
        return;
    }

    if id == menu::id::FILE_NEW {
        new_document(hwnd);
        return;
    }

    if id == menu::id::FILE_REVEAL {
        reveal_selected_page(hwnd);
        return;
    }

    if id == CMD_EXPORT || id == menu::id::FILE_EXPORT {
        export_pdf(hwnd);
        return;
    }

    if id == menu::id::FILE_EXPORT_JPG {
        export_selected_page(hwnd);
        return;
    }

    if id == menu::id::FILE_EXPORT_TEXT {
        export_all_text(hwnd);
        return;
    }

    if id == menu::id::TOOL_OCR_EDIT {
        edit_ocr_text(hwnd);
        return;
    }

    if id == menu::id::FILE_IMPORT {
        import_images(hwnd);
        return;
    }

    if id == menu::id::FILE_PRINT || id == 2009 {
        print_selected_page(hwnd);
        return;
    }

    if id == 2007 || id == menu::id::TOOL_OCR {
        ocr_selected_page(hwnd);
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
    set_menu_enabled(hwnd, menu::id::EDIT_UNDO, enabled);
}

/// 启用/禁用“删除当前页”菜单项。
fn set_delete_page_enabled(hwnd: HWND, enabled: bool) {
    set_menu_enabled(hwnd, menu::id::EDIT_DELETE_PAGE, enabled);
}

fn set_menu_enabled(hwnd: HWND, id: usize, enabled: bool) {
    // SAFETY: 主窗口菜单由系统持有；菜单项 ID 固定。
    unsafe {
        let menu = GetMenu(hwnd);
        if menu.is_null() {
            return;
        }
        let flags = MF_BYCOMMAND | if enabled { MF_ENABLED } else { MF_GRAYED };
        EnableMenuItem(menu, id as u32, flags);
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
        refresh_settings_preview(s);
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
fn add_frame_thumbnail(s: &AppState, width: u32, height: u32, data: &[u8]) -> Option<usize> {
    let thumb_width = u32::try_from(s.thumb_width.max(1)).ok()?;
    let thumb_height = u32::try_from(s.thumb_height.max(1)).ok()?;
    let (width, height, data) =
        imaging::thumbnail_bgra(width, height, data, thumb_width, thumb_height).ok()?;
    controls::add_image_bgra(s.thumbs, thumb_width, thumb_height, width, height, &data)
}

/// 刷新页面列表选中项并同步菜单状态。
fn update_page_selection(s: &mut AppState) {
    s.selected_page = controls::list_selected(s.pages);
    set_delete_page_enabled(s.hwnd, s.selected_page.is_some());
}

/// 用系统默认程序打开选中的页面。
fn open_selected_page(hwnd: HWND) {
    let state = state_ptr(hwnd);
    if state.is_null() {
        return;
    }
    // SAFETY: state 有效；路径复制后立即使用。
    unsafe {
        let s = &mut *state;
        let Some(index) = s.selected_page else {
            return;
        };
        let Some(page) = s.session.pages.get(index) else {
            return;
        };
        let path = s.output_root.join(&page.path);
        if !path.exists() {
            show_error(hwnd, "图片文件不存在或已被移动");
            return;
        }
        let operation = wide("open");
        let file = wide(&path.to_string_lossy());
        ShellExecuteW(
            hwnd,
            operation.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// 在资源管理器中定位选中的页面文件。
fn reveal_selected_page(hwnd: HWND) {
    let state = state_ptr(hwnd);
    if state.is_null() {
        return;
    }
    // SAFETY: state 有效；参数缓冲区在调用期间保持存活。
    unsafe {
        let s = &mut *state;
        let Some(index) = s.selected_page else {
            s.status_msg = "请先在左侧选择页面".to_string();
            refresh_status(s);
            return;
        };
        let Some(page) = s.session.pages.get(index) else {
            return;
        };
        let path = s.output_root.join(&page.path);
        if !path.exists() {
            s.status_msg = "图片文件不存在或已被移动".to_string();
            refresh_status(s);
            return;
        }
        let operation = wide("open");
        let program = wide("explorer.exe");
        let parameters = wide(&format!("/select,\"{}\"", path.display()));
        ShellExecuteW(
            hwnd,
            operation.as_ptr(),
            program.as_ptr(),
            parameters.as_ptr(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// 生成并保存整份会话的 PDF。
fn export_pdf(hwnd: HWND) {
    let state = state_ptr(hwnd);
    if state.is_null() {
        return;
    }
    // SAFETY: state 有效；文件与对话框调用逐项检查。
    unsafe {
        let s = &mut *state;
        if s.session.pages.is_empty() {
            s.status_msg = "当前文档没有页面".to_string();
            refresh_status(s);
            return;
        }
        let default_name = format!(
            "usbscan_{}.pdf",
            naming::TimeStamp::now().format("yyyyMMdd_HHmmss")
        );
        let Some(path) = save_file_dialog(hwnd, "导出 PDF", &default_name, "pdf") else {
            return;
        };

        let mut jpeg_pages = Vec::with_capacity(s.session.pages.len());
        for page in &s.session.pages {
            let source = s.output_root.join(&page.path);
            match fs::read(&source) {
                Ok(bytes) => jpeg_pages.push(bytes),
                Err(err) => {
                    let message = format!("读取 {} 失败：{err}", source.display());
                    s.status_msg.clone_from(&message);
                    refresh_status(s);
                    show_error(hwnd, &message);
                    return;
                }
            }
        }
        let images: Vec<export::PdfImage<'_>> = jpeg_pages
            .iter()
            .zip(&s.session.pages)
            .map(|(jpeg, page)| export::PdfImage {
                jpeg,
                width: page.width,
                height: page.height,
            })
            .collect();

        let temp = path.with_extension("pdf.tmp");
        let result = fs::File::create(&temp).and_then(|mut file| {
            export::write_pdf(&mut file, &images)?;
            file.sync_all()?;
            fs::rename(&temp, &path)
        });
        match result {
            Ok(()) => {
                s.status_msg = format!("已导出 PDF（{} 页）：{}", images.len(), path.display());
            }
            Err(err) => {
                let _ = fs::remove_file(&temp);
                let message = format!("导出 PDF 失败：{err}");
                s.status_msg.clone_from(&message);
                refresh_status(s);
                show_error(hwnd, &message);
                return;
            }
        }
        refresh_status(s);
    }
}

/// 使用系统“打开”对话框导入图片到当前会话。
fn import_images(hwnd: HWND) {
    let Some(files) = pick_image_files(hwnd) else {
        return;
    };
    let state = state_ptr(hwnd);
    if state.is_null() {
        return;
    }
    // SAFETY: state 有效；文件操作逐项检查。
    unsafe {
        let s = &mut *state;
        let import_dir = s.output_root.join("导入");
        if let Err(err) = fs::create_dir_all(&import_dir) {
            show_error(hwnd, &format!("无法创建导入目录：{err}"));
            return;
        }

        let mut imported = 0usize;
        let mut skipped = 0usize;
        for source in files {
            let Some(stem) = source.file_stem() else {
                skipped += 1;
                continue;
            };
            let target_name = format!("{}.jpg", stem.to_string_lossy());
            let target = unique_path(&import_dir.join(target_name));
            let dimensions = (|| {
                let file = fs::File::create(&target).ok()?;
                let mut writer = BufWriter::new(file);
                let dimensions =
                    imaging::transcode_to_jpeg(&source, s.jpeg_quality, &mut writer).ok()?;
                writer.flush().ok()?;
                writer.get_ref().sync_all().ok()?;
                Some(dimensions)
            })();
            let Some(dimensions) = dimensions else {
                let _ = fs::remove_file(&target);
                skipped += 1;
                continue;
            };
            let relative = target
                .strip_prefix(&s.output_root)
                .map_or_else(|_| target.clone(), Path::to_path_buf);
            let seq = s.session.next_seq;
            s.session.push_page(doc::Page {
                seq,
                path: relative,
                raw: None,
                width: dimensions.0,
                height: dimensions.1,
                captured_at: doc::now_unix(),
                device: Some("导入".to_string()),
            });
            s.session.closed_cleanly = false;
            let thumb_width = u32::try_from(s.thumb_width.max(1)).unwrap_or(1);
            let thumb_height = u32::try_from(s.thumb_height.max(1)).unwrap_or(1);
            let image = imaging::image_thumbnail_bgra(&target, thumb_width, thumb_height)
                .ok()
                .and_then(|(width, height, data)| {
                    controls::add_image_bgra(
                        s.thumbs,
                        thumb_width,
                        thumb_height,
                        width,
                        height,
                        &data,
                    )
                });
            let name = target.file_name().map_or_else(
                || target.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            );
            controls::add_page_item(s.pages, s.session.pages.len() - 1, &name, image);
            imported += 1;
        }

        let _ = s.store.save(&s.session);
        s.status_msg = if skipped == 0 {
            format!("已导入 {imported} 张图片")
        } else {
            format!("已导入 {imported} 张图片，跳过 {skipped} 张")
        };
        refresh_settings_preview(s);
        refresh_status(s);
    }
}

/// 对选中的页面执行离线 OCR。
fn ocr_selected_page(hwnd: HWND) {
    let state = state_ptr(hwnd);
    if state.is_null() {
        return;
    }
    // SAFETY: state 有效；外部进程调用返回结果后统一提示。
    unsafe {
        let s = &mut *state;
        let Some(index) = s.selected_page else {
            s.status_msg = "请先在左侧选择要识别的页面".to_string();
            refresh_status(s);
            return;
        };
        let path = {
            let Some(page) = s.session.pages.get(index) else {
                return;
            };
            s.output_root.join(&page.path)
        };
        if !path.exists() {
            show_error(hwnd, "图片文件不存在或已被移动");
            return;
        }
        let Some(executable) = ocr::find_tesseract(s.tesseract_path.as_deref()) else {
            show_error(hwnd, &ocr::OcrError::NotFound.to_string());
            s.status_msg = "未找到 Tesseract 5".to_string();
            refresh_status(s);
            return;
        };

        let mut languages = Vec::new();
        for language in [
            s.ocr_language.as_str(),
            ocr::DEFAULT_LANGUAGE,
            "chi_sim",
            "eng",
        ] {
            if !language.is_empty() && !languages.contains(&language) {
                languages.push(language);
            }
        }

        match ocr::recognize_with_fallback(&executable, &path, &languages) {
            Ok((text, language)) => {
                let Some(edited) = crate::text_edit::edit_text(hwnd, "OCR 识别结果", &text)
                else {
                    s.status_msg = format!("OCR 完成（{language}），已取消保存");
                    refresh_status(s);
                    return;
                };
                let text_path = path.with_extension("txt");
                match write_text_atomic(&text_path, &edited) {
                    Ok(()) => {
                        s.status_msg = format!(
                            "OCR 完成（{language}，{} 字符）：{}",
                            edited.chars().count(),
                            text_path.display()
                        );
                    }
                    Err(err) => {
                        let message = format!("文本文件写入失败：{err}");
                        s.status_msg.clone_from(&message);
                        refresh_status(s);
                        show_error(hwnd, &message);
                        return;
                    }
                }
            }
            Err(err) => {
                let message = err.to_string();
                s.status_msg.clone_from(&message);
                show_error(hwnd, &message);
            }
        }
        refresh_status(s);
    }
}

/// 编辑选中页已保存的 OCR 文本。
fn edit_ocr_text(hwnd: HWND) {
    let state = state_ptr(hwnd);
    if state.is_null() {
        return;
    }
    // SAFETY: state 有效；文件读写逐项检查。
    unsafe {
        let s = &mut *state;
        let Some(index) = s.selected_page else {
            s.status_msg = "请先在左侧选择要编辑文本的页面".to_string();
            refresh_status(s);
            return;
        };
        let Some(page) = s.session.pages.get(index) else {
            return;
        };
        let text_path = s.output_root.join(page.path.with_extension("txt"));
        let content = fs::read_to_string(&text_path).unwrap_or_default();
        let title = format!("OCR 文本 — 第 {} 页", index + 1);
        let Some(edited) = crate::text_edit::edit_text(hwnd, &title, &content) else {
            s.status_msg = "已取消文本编辑".to_string();
            refresh_status(s);
            return;
        };
        match write_text_atomic(&text_path, &edited) {
            Ok(()) => {
                s.status_msg = format!("文本已保存：{}", text_path.display());
            }
            Err(err) => {
                let message = format!("文本文件写入失败：{err}");
                s.status_msg.clone_from(&message);
                refresh_status(s);
                show_error(hwnd, &message);
                return;
            }
        }
        refresh_status(s);
    }
}

/// 把所有页面的 OCR 文本合并导出为一个 `.txt`。
fn export_all_text(hwnd: HWND) {
    let state = state_ptr(hwnd);
    if state.is_null() {
        return;
    }
    // SAFETY: state 有效；文件对话框与写入逐项检查。
    unsafe {
        let s = &mut *state;
        if s.session.pages.is_empty() {
            s.status_msg = "当前文档没有页面".to_string();
            refresh_status(s);
            return;
        }
        let default_name = format!(
            "usbscan_{}.txt",
            naming::TimeStamp::now().format("yyyyMMdd_HHmmss")
        );
        let Some(path) = save_file_dialog(hwnd, "导出 OCR 文本", &default_name, "txt") else {
            return;
        };

        let mut out = String::with_capacity(4096);
        out.push_str(&s.session.doc_name);
        out.push_str("\r\n\r\n");
        let mut exported = 0usize;
        for (index, page) in s.session.pages.iter().enumerate() {
            let text_path = s.output_root.join(page.path.with_extension("txt"));
            let Ok(text) = fs::read_to_string(&text_path) else {
                continue;
            };
            exported += 1;
            let _ = write!(
                out,
                "===== 第 {} 页 · {} =====\r\n",
                index + 1,
                page.path.display()
            );
            out.push_str(text.trim_end());
            out.push_str("\r\n\r\n");
        }
        if exported == 0 {
            s.status_msg = "没有可导出的 OCR 文本，请先识别至少一页".to_string();
            refresh_status(s);
            return;
        }
        match write_text_atomic(&path, &out) {
            Ok(()) => {
                s.status_msg = format!("已导出 {exported} 页文本：{}", path.display());
            }
            Err(err) => {
                let message = format!("导出文本失败：{err}");
                s.status_msg.clone_from(&message);
                refresh_status(s);
                show_error(hwnd, &message);
                return;
            }
        }
        refresh_status(s);
    }
}

/// 调用系统默认图片程序的打印动作。
fn print_selected_page(hwnd: HWND) {
    let state = state_ptr(hwnd);
    if state.is_null() {
        return;
    }
    // SAFETY: state 有效；ShellExecuteW 接收的字符串在调用期间有效。
    unsafe {
        let s = &mut *state;
        let Some(index) = s.selected_page else {
            s.status_msg = "请先在左侧选择要打印的页面".to_string();
            refresh_status(s);
            return;
        };
        let Some(page) = s.session.pages.get(index) else {
            return;
        };
        let path = s.output_root.join(&page.path);
        if !path.exists() {
            show_error(hwnd, "图片文件不存在或已被移动");
            return;
        }
        let operation = wide("print");
        let file = wide(&path.to_string_lossy());
        let result = ShellExecuteW(
            hwnd,
            operation.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
        if (result as isize) <= 32 {
            s.status_msg = "无法调用系统打印，请先在默认图片程序中打开该文件".to_string();
        } else {
            s.status_msg = format!("已发送到系统打印：{}", page.path.display());
        }
        refresh_status(s);
    }
}

fn save_file_dialog(
    hwnd: HWND,
    title: &str,
    default_name: &str,
    extension: &str,
) -> Option<PathBuf> {
    let mut buffer = vec![0u16; 1024];
    for (slot, ch) in buffer.iter_mut().zip(default_name.encode_utf16()) {
        *slot = ch;
    }
    let filter: Vec<u16> = format!("文件 (*.{extension})\0*.{extension}\0所有文件\0*.*\0\0")
        .encode_utf16()
        .collect();
    let title = wide(title);
    let extension = wide(extension);
    // SAFETY: 所有指针都指向调用期间存活的缓冲区。
    unsafe {
        let mut dialog: OPENFILENAMEW = std::mem::zeroed();
        dialog.lStructSize = u32::try_from(std::mem::size_of::<OPENFILENAMEW>()).unwrap_or(0);
        dialog.hwndOwner = hwnd;
        dialog.lpstrFilter = filter.as_ptr();
        dialog.lpstrFile = buffer.as_mut_ptr();
        dialog.nMaxFile = u32::try_from(buffer.len()).unwrap_or(0);
        dialog.lpstrTitle = title.as_ptr();
        dialog.lpstrDefExt = extension.as_ptr();
        dialog.Flags = OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST;
        if GetSaveFileNameW(&mut dialog) == 0 {
            return None;
        }
    }
    let len = buffer
        .iter()
        .position(|&ch| ch == 0)
        .unwrap_or(buffer.len());
    Some(PathBuf::from(String::from_utf16_lossy(&buffer[..len])))
}

fn pick_image_files(hwnd: HWND) -> Option<Vec<PathBuf>> {
    let mut buffer = vec![0u16; 32_768];
    let filter: Vec<u16> = "图片文件\0*.jpg;*.jpeg;*.png;*.bmp\0所有文件\0*.*\0\0"
        .encode_utf16()
        .collect();
    let title = wide("导入图片");
    // SAFETY: 缓冲区在调用期间存活。
    unsafe {
        let mut dialog: OPENFILENAMEW = std::mem::zeroed();
        dialog.lStructSize = u32::try_from(std::mem::size_of::<OPENFILENAMEW>()).unwrap_or(0);
        dialog.hwndOwner = hwnd;
        dialog.lpstrFilter = filter.as_ptr();
        dialog.lpstrFile = buffer.as_mut_ptr();
        dialog.nMaxFile = u32::try_from(buffer.len()).unwrap_or(0);
        dialog.lpstrTitle = title.as_ptr();
        dialog.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_ALLOWMULTISELECT | OFN_EXPLORER;
        if GetOpenFileNameW(&mut dialog) == 0 {
            return None;
        }
    }

    let mut parts = Vec::new();
    let mut start = 0usize;
    for (index, &ch) in buffer.iter().enumerate() {
        if ch != 0 {
            continue;
        }
        if index == start {
            break;
        }
        parts.push(String::from_utf16_lossy(&buffer[start..index]));
        start = index + 1;
    }
    if parts.is_empty() {
        return None;
    }
    if parts.len() == 1 {
        return Some(vec![PathBuf::from(parts.remove(0))]);
    }
    let directory = PathBuf::from(parts.remove(0));
    Some(parts.into_iter().map(|name| directory.join(name)).collect())
}

fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path.file_stem().map_or_else(
        || "image".to_string(),
        |stem| stem.to_string_lossy().into_owned(),
    );
    let extension = path
        .extension()
        .map(|ext| ext.to_string_lossy().into_owned());
    for index in 2..10_000 {
        let name = match &extension {
            Some(extension) => format!("{stem} ({index}).{extension}"),
            None => format!("{stem} ({index})"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    path.to_path_buf()
}

/// 把选中的页面复制到用户选择的目录。
fn export_selected_page(hwnd: HWND) {
    let state = state_ptr(hwnd);
    if state.is_null() {
        return;
    }
    // SAFETY: state 有效；文件操作逐项检查。
    unsafe {
        let s = &mut *state;
        let Some(index) = s.selected_page else {
            s.status_msg = "请先在左侧选择要导出的页面".to_string();
            refresh_status(s);
            return;
        };
        let Some(page) = s.session.pages.get(index) else {
            return;
        };
        let source = s.output_root.join(&page.path);
        if !source.exists() {
            show_error(hwnd, "图片文件不存在或已被移动");
            return;
        }
        let Some(folder) = choose_folder(hwnd, "选择导出目录") else {
            return;
        };
        let Some(file_name) = source.file_name() else {
            return;
        };
        let target = folder.join(file_name);
        if target == source {
            s.status_msg = "导出目录与源文件相同，无需复制".to_string();
            refresh_status(s);
            return;
        }
        if target.exists() {
            let text = wide(&format!("{} 已存在，是否覆盖？", target.display()));
            let title = wide("导出图片");
            if MessageBoxW(
                hwnd,
                text.as_ptr(),
                title.as_ptr(),
                MB_YESNO | MB_ICONWARNING,
            ) != IDYES
            {
                return;
            }
        }
        match fs::copy(&source, &target) {
            Ok(_) => {
                s.status_msg = format!("已导出到 {}", target.display());
            }
            Err(err) => {
                let message = format!("导出失败：{err}");
                s.status_msg.clone_from(&message);
                refresh_status(s);
                show_error(hwnd, &message);
                return;
            }
        }
        refresh_status(s);
    }
}

/// 删除页面上选中的页面。
fn delete_selected_page(hwnd: HWND) {
    let state = state_ptr(hwnd);
    if state.is_null() {
        return;
    }
    // SAFETY: state 有效；删除前二次确认。
    unsafe {
        let s = &mut *state;
        let Some(index) = s.selected_page else {
            s.status_msg = "请先在左侧选择要删除的页面".to_string();
            refresh_status(s);
            return;
        };
        let Some(page) = s.session.pages.get(index).cloned() else {
            return;
        };
        let text = wide(&format!(
            "确定删除第 {} 页？\n\n{}",
            index + 1,
            page.path.display()
        ));
        let title = wide("删除页面");
        if MessageBoxW(
            hwnd,
            text.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONQUESTION,
        ) != IDYES
        {
            return;
        }

        let path = s.output_root.join(&page.path);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                let message = format!("无法删除 {}：{err}", path.display());
                s.status_msg.clone_from(&message);
                refresh_status(s);
                show_error(hwnd, &message);
                return;
            }
        }

        s.session.remove_page(index);
        s.session.closed_cleanly = false;
        let _ = s.store.save(&s.session);
        reset_page_list(s);
        rebuild_page_list(s);
        s.selected_page = None;
        set_delete_page_enabled(hwnd, false);
        s.status_msg = format!("已删除第 {} 页", index + 1);
        refresh_status(s);
    }
}

/// 重新加载页面列表并恢复指定选中项。
fn reload_page_list(s: &mut AppState, selected: Option<usize>) {
    reset_page_list(s);
    rebuild_page_list(s);
    if let Some(index) = selected {
        if index < s.session.pages.len() {
            controls::set_list_selected(s.pages, index);
            s.selected_page = Some(index);
            set_delete_page_enabled(s.hwnd, true);
        }
    }
}

fn write_jpeg_atomic(
    path: &Path,
    width: u32,
    height: u32,
    bgra: &[u8],
    quality: u8,
) -> Result<(), String> {
    let temp = path.with_extension("tmp.jpg");
    let file = fs::File::create(&temp).map_err(|err| err.to_string())?;
    if let Err(err) = write_jpeg(file, width, height, bgra, quality) {
        let _ = fs::remove_file(&temp);
        return Err(err);
    }
    fs::rename(&temp, path).map_err(|err| {
        let _ = fs::remove_file(&temp);
        err.to_string()
    })
}

fn write_text_atomic(path: &Path, text: &str) -> Result<(), String> {
    let temp = path.with_extension("txt.tmp");
    let mut file = fs::File::create(&temp).map_err(|err| err.to_string())?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|err| err.to_string())?;
    fs::rename(&temp, path).map_err(|err| {
        let _ = fs::remove_file(&temp);
        err.to_string()
    })
}

/// 使用当前增强设置重新处理选中页。
fn reprocess_selected_page(hwnd: HWND) {
    let state = state_ptr(hwnd);
    if state.is_null() {
        return;
    }
    // SAFETY: state 有效；文件操作逐项检查。
    unsafe {
        let s = &mut *state;
        let Some(index) = s.selected_page else {
            s.status_msg = "请先在左侧选择要重新处理的页面".to_string();
            refresh_status(s);
            return;
        };
        let Some(page) = s.session.pages.get(index).cloned() else {
            return;
        };
        let Some(raw) = page
            .raw
            .as_ref()
            .map(|raw| s.output_root.join(raw))
            .filter(|path| path.exists())
        else {
            s.status_msg = "此页没有保留原图，无法重新处理".to_string();
            refresh_status(s);
            return;
        };
        let (width, height, data) = match imaging::load_bgra(&raw) {
            Ok(image) => image,
            Err(err) => {
                let message = format!("读取原图失败：{err}");
                s.status_msg.clone_from(&message);
                refresh_status(s);
                show_error(hwnd, &message);
                return;
            }
        };
        let (width, height, data) = if s.enhance.is_active() {
            match imaging::enhance(width, height, &data, s.enhance) {
                Ok(image) => (image.width, image.height, image.bgra),
                Err(err) => {
                    let message = format!("增强失败：{err}");
                    s.status_msg.clone_from(&message);
                    refresh_status(s);
                    show_error(hwnd, &message);
                    return;
                }
            }
        } else {
            (width, height, data)
        };

        let path = s.output_root.join(&page.path);
        if let Err(err) = write_jpeg_atomic(&path, width, height, &data, s.jpeg_quality) {
            let message = format!("写回成品失败：{err}");
            s.status_msg.clone_from(&message);
            refresh_status(s);
            show_error(hwnd, &message);
            return;
        }
        if let Some(updated) = s.session.pages.get_mut(index) {
            updated.width = width;
            updated.height = height;
        }
        s.session.updated_at = doc::now_unix();
        s.session.closed_cleanly = false;
        let _ = s.store.save(&s.session);
        reload_page_list(s, Some(index));
        s.status_msg = format!("已重新处理第 {} 页", index + 1);
        refresh_status(s);
    }
}

/// 手动四角点透视校正：拖动角点后写回成品图。
fn manual_perspective_selected(hwnd: HWND) {
    let state = state_ptr(hwnd);
    if state.is_null() {
        return;
    }
    // SAFETY: state 有效；文件与图像操作逐项检查。
    unsafe {
        let s = &mut *state;
        let Some(index) = s.selected_page else {
            s.status_msg = "请先在左侧选择要校正的页面".to_string();
            refresh_status(s);
            return;
        };
        let Some(page) = s.session.pages.get(index).cloned() else {
            return;
        };
        let path = s.output_root.join(&page.path);
        let (width, height, data) = match imaging::load_bgra(&path) {
            Ok(image) => image,
            Err(err) => {
                let message = format!("读取图片失败：{err}");
                s.status_msg.clone_from(&message);
                refresh_status(s);
                show_error(hwnd, &message);
                return;
            }
        };
        if width < 32 || height < 32 {
            s.status_msg = "页面尺寸过小，无法进行透视校正".to_string();
            refresh_status(s);
            return;
        }
        let initial = imaging::detect_document_corners(width, height, &data)
            .unwrap_or_else(|| default_corners(width, height));
        let Some(corners) =
            crate::corners::edit_corners(hwnd, width, height, &data, initial, s.dark)
        else {
            s.status_msg = "已取消透视校正".to_string();
            refresh_status(s);
            return;
        };
        let (out_width, out_height, out_data) =
            match imaging::perspective_from_corners(width, height, &data, &corners) {
                Ok(image) => image,
                Err(err) => {
                    let message = format!("透视校正失败：{err}");
                    s.status_msg.clone_from(&message);
                    refresh_status(s);
                    show_error(hwnd, &message);
                    return;
                }
            };
        if let Err(err) = write_jpeg_atomic(&path, out_width, out_height, &out_data, s.jpeg_quality)
        {
            let message = format!("写回图片失败：{err}");
            s.status_msg.clone_from(&message);
            refresh_status(s);
            show_error(hwnd, &message);
            return;
        }
        if let Some(updated) = s.session.pages.get_mut(index) {
            updated.width = out_width;
            updated.height = out_height;
        }
        s.session.updated_at = doc::now_unix();
        s.session.closed_cleanly = false;
        let _ = s.store.save(&s.session);
        reload_page_list(s, Some(index));
        s.status_msg = format!("已透视校正第 {} 页", index + 1);
        refresh_status(s);
    }
}

/// 在选中页上手绘批注并扁平化写回。
fn annotate_selected_page(hwnd: HWND) {
    let state = state_ptr(hwnd);
    if state.is_null() {
        return;
    }
    // SAFETY: state 有效；图像与文件操作逐项检查。
    unsafe {
        let s = &mut *state;
        let Some(index) = s.selected_page else {
            s.status_msg = "请先在左侧选择要批注的页面".to_string();
            refresh_status(s);
            return;
        };
        let Some(page) = s.session.pages.get(index).cloned() else {
            return;
        };
        let path = s.output_root.join(&page.path);
        let (width, height, data) = match imaging::load_bgra(&path) {
            Ok(image) => image,
            Err(err) => {
                let message = format!("读取图片失败：{err}");
                s.status_msg.clone_from(&message);
                refresh_status(s);
                show_error(hwnd, &message);
                return;
            }
        };
        let Some(annotations) =
            crate::annotate::edit_annotations(hwnd, width, height, &data, s.dark)
        else {
            s.status_msg = "已取消批注".to_string();
            refresh_status(s);
            return;
        };
        if annotations.is_empty() {
            s.status_msg = "未添加批注".to_string();
            refresh_status(s);
            return;
        }
        let rendered = match imaging::render_annotations(width, height, &data, &annotations) {
            Ok(rendered) => rendered,
            Err(err) => {
                let message = format!("批注渲染失败：{err}");
                s.status_msg.clone_from(&message);
                refresh_status(s);
                show_error(hwnd, &message);
                return;
            }
        };
        if let Err(err) = write_jpeg_atomic(&path, width, height, &rendered, s.jpeg_quality) {
            let message = format!("写回批注失败：{err}");
            s.status_msg.clone_from(&message);
            refresh_status(s);
            show_error(hwnd, &message);
            return;
        }
        s.session.updated_at = doc::now_unix();
        s.session.closed_cleanly = false;
        let _ = s.store.save(&s.session);
        reload_page_list(s, Some(index));
        s.status_msg = format!("已添加 {} 条批注", annotations.len());
        refresh_status(s);
    }
}

/// 四角无法自动检测时的默认内缩四边形。
fn default_corners(width: u32, height: u32) -> [(f64, f64); 4] {
    let left = f64::from(width) * 0.05;
    let top = f64::from(height) * 0.05;
    let right = f64::from(width) * 0.95;
    let bottom = f64::from(height) * 0.95;
    [(left, top), (right, top), (right, bottom), (left, bottom)]
}

#[derive(Clone, Copy)]
enum PageTransform {
    RotateRight,
    RotateLeft,
    Mirror,
}

fn transform_selected_page(hwnd: HWND, transform: PageTransform) {
    let state = state_ptr(hwnd);
    if state.is_null() {
        return;
    }
    // SAFETY: state 有效；文件操作逐项检查。
    unsafe {
        let s = &mut *state;
        let Some(index) = s.selected_page else {
            s.status_msg = "请先在左侧选择页面".to_string();
            refresh_status(s);
            return;
        };
        let Some(page) = s.session.pages.get(index).cloned() else {
            return;
        };
        let path = s.output_root.join(&page.path);
        let (width, height, data) = match imaging::load_bgra(&path) {
            Ok(image) => image,
            Err(err) => {
                let message = format!("读取图片失败：{err}");
                s.status_msg.clone_from(&message);
                refresh_status(s);
                show_error(hwnd, &message);
                return;
            }
        };
        let result = match transform {
            PageTransform::RotateRight => imaging::rotate90_bgra(width, height, &data, true),
            PageTransform::RotateLeft => imaging::rotate90_bgra(width, height, &data, false),
            PageTransform::Mirror => imaging::mirror_horizontal_bgra(width, height, &data),
        };
        let (width, height, data) = match result {
            Ok(image) => image,
            Err(err) => {
                let message = format!("图像变换失败：{err}");
                s.status_msg.clone_from(&message);
                refresh_status(s);
                show_error(hwnd, &message);
                return;
            }
        };
        if let Err(err) = write_jpeg_atomic(&path, width, height, &data, s.jpeg_quality) {
            let message = format!("写回图片失败：{err}");
            s.status_msg.clone_from(&message);
            refresh_status(s);
            show_error(hwnd, &message);
            return;
        }
        if let Some(updated) = s.session.pages.get_mut(index) {
            updated.width = width;
            updated.height = height;
        }
        s.session.updated_at = doc::now_unix();
        s.session.closed_cleanly = false;
        let _ = s.store.save(&s.session);
        reload_page_list(s, Some(index));
        s.status_msg = "页面已更新".to_string();
        refresh_status(s);
    }
}

/// 结束当前会话并开始新文档（磁盘上的图片保留）。
fn new_document(hwnd: HWND) {
    let state = state_ptr(hwnd);
    if state.is_null() {
        return;
    }
    // SAFETY: state 有效；确认框为标准 Win32 调用。
    unsafe {
        let s = &mut *state;
        if !s.session.pages.is_empty() {
            let text = wide("新建文档将结束当前会话。\n\n已保存的图片不会被删除。是否继续？");
            let title = wide("新建文档");
            if MessageBoxW(
                hwnd,
                text.as_ptr(),
                title.as_ptr(),
                MB_YESNO | MB_ICONQUESTION,
            ) != IDYES
            {
                return;
            }
            let _ = s.store.archive();
        }

        s.session = doc::Session::default();
        reset_page_list(s);
        let _ = s.store.save(&s.session);
        s.status_msg = "已新建文档".to_string();
        refresh_status(s);
    }
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

#[allow(clippy::too_many_lines)]
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

        let enhanced = if s.enhance.is_active() {
            match imaging::enhance(frame.width, frame.height, &frame.data, s.enhance) {
                Ok(image) => Some(image),
                Err(err) => {
                    s.status_msg = format!("图像增强失败，本次保存原图：{err}");
                    None
                }
            }
        } else {
            None
        };
        let (width, height, pixels) = enhanced.as_ref().map_or_else(
            || (frame.width, frame.height, frame.data.as_slice()),
            |image| (image.width, image.height, image.bgra.as_slice()),
        );

        let seq = s.session.next_seq;
        let rendered = {
            let ctx = naming::NamingContext {
                now: naming::TimeStamp::now(),
                device: s.camera_name.as_deref(),
                width: Some(width),
                height: Some(height),
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
        let reserved = match s
            .naming
            .reserve(&dir, &rendered.file_name, "jpg", s.collision)
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
        let result = write_jpeg(reserved.file, width, height, pixels, s.jpeg_quality);
        match result {
            Ok(()) => {
                let name = path.file_name().map_or_else(
                    || path.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                let image = add_frame_thumbnail(s, width, height, pixels);
                let relative = path
                    .strip_prefix(&s.output_root)
                    .map_or_else(|_| path.clone(), Path::to_path_buf);
                let raw = if enhanced.is_some() {
                    write_raw_copy(s, &frame, seq)
                } else {
                    None
                };
                s.session.push_page(doc::Page {
                    seq,
                    path: relative,
                    raw,
                    width,
                    height,
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

/// 把未增强的原始帧保存到 `.usbscan/raw/`，返回相对于输出根的路径。
fn write_raw_copy(s: &AppState, frame: &capture::Frame, seq: u64) -> Option<PathBuf> {
    let directory = s.output_root.join(".usbscan").join("raw");
    fs::create_dir_all(&directory).ok()?;
    let name = format!("{seq:06}_{}.jpg", doc::now_unix());
    let path = directory.join(&name);
    let file = fs::File::create(&path).ok()?;
    if write_jpeg(
        file,
        frame.width,
        frame.height,
        &frame.data,
        s.jpeg_quality.max(90),
    )
    .is_err()
    {
        let _ = fs::remove_file(&path);
        return None;
    }
    Some(PathBuf::from(".usbscan").join("raw").join(name))
}

/// 把一帧编码为 JPEG 并同步到磁盘。
fn write_jpeg(
    file: fs::File,
    width: u32,
    height: u32,
    bgra: &[u8],
    quality: u8,
) -> Result<(), String> {
    let mut writer = BufWriter::new(file);
    imaging::encode_jpeg_bgra(width, height, bgra, quality, &mut writer)
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
    let (frame, dpi_now, error, connected) = unsafe {
        if state.is_null() {
            (None, dpi::BASE_DPI, None, false)
        } else {
            let s = &*state;
            (
                s.capture.latest_frame(),
                s.dpi,
                s.camera_error.clone(),
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
        let width = rc.right - rc.left;
        let height = rc.bottom - rc.top;

        // 先画到离屏缓冲，再一次性 BitBlt 到窗口，消除“擦背景→画帧”的闪烁。
        let target = if state.is_null() {
            hdc
        } else {
            let s = &mut *state;
            if s.preview_buffer.ensure(hdc, width, height) {
                s.preview_buffer.dc
            } else {
                hdc
            }
        };

        let background = if dark {
            rgb(0x1e, 0x1f, 0x22)
        } else {
            GetSysColor(COLOR_APPWORKSPACE)
        };
        let brush = CreateSolidBrush(background);
        FillRect(target, &rc, brush);
        DeleteObject(brush as HGDIOBJ);

        if let Some(frame) = frame {
            draw_frame(target, &rc, &frame, dpi_now);
        } else {
            draw_placeholder(
                hwnd,
                target,
                &rc,
                dark,
                dpi_now,
                error.as_deref(),
                connected,
            );
        }

        if target != hdc {
            BitBlt(hdc, 0, 0, width, height, target, 0, 0, SRCCOPY);
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
        // 实时预览用 COLORONCOLOR 比 HALFTONE 快得多，足以避免掉帧和闪烁；
        // 成品 JPEG 直接使用原始帧，不受预览缩放质量影响。
        SetStretchBltMode(hdc, COLORONCOLOR);
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
    error: Option<&str>,
    connected: bool,
) {
    let (line1, line2) = if let Some(error) = error {
        ("摄像头不可用", error)
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
            DT_CENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
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
        write_jpeg(file, frame.width, frame.height, &frame.data, 92).expect("write jpeg");

        let bytes = fs::read(&path).expect("read back");
        assert!(bytes.starts_with(&[0xFF, 0xD8]), "JPEG SOI");
        assert!(bytes.ends_with(&[0xFF, 0xD9]), "JPEG EOI");
        let _ = fs::remove_dir_all(&dir);
    }
}
