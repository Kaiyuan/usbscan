// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 原生菜单栏构建。

use std::sync::atomic::Ordering;

use windows_sys::Win32::Graphics::Gdi::{
    CreateSolidBrush, DeleteObject, DrawTextW, FillRect, GetDC, GetStockObject, GetSysColor,
    GetSysColorBrush, GetTextExtentPoint32W, ReleaseDC, SelectObject, SetBkMode, SetTextColor,
    COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_MENU, COLOR_MENUTEXT,
    DEFAULT_GUI_FONT, DT_CENTER, DT_LEFT, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, HGDIOBJ,
    TRANSPARENT,
};
use windows_sys::Win32::UI::Controls::{
    DRAWITEMSTRUCT, MEASUREITEMSTRUCT, ODS_DISABLED, ODS_SELECTED, ODT_MENU,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    VK_E, VK_F10, VK_F11, VK_F12, VK_F5, VK_F9, VK_N, VK_O, VK_P, VK_V, VK_Y, VK_Z,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateAcceleratorTableW, CreateMenu, CreatePopupMenu, GetMenuItemCount,
    GetMenuItemID, GetMenuState, GetMenuStringW, GetSubMenu, GetSystemMetrics, ModifyMenuW,
    SetMenuInfo, ACCEL, HACCEL, HMENU, MENUINFO, MF_BYCOMMAND, MF_BYPOSITION, MF_CHECKED,
    MF_DISABLED, MF_GRAYED, MF_OWNERDRAW, MF_POPUP, MF_SEPARATOR, MF_STRING, MIM_APPLYTOSUBMENUS,
    MIM_BACKGROUND, SM_CYMENU,
};

use crate::error::WinError;
use crate::text::wide;

/// 菜单与命令 ID。
///
/// 取值范围避开控件的 1000 以下区段与工具栏的 2000 段。
pub mod id {
    pub const FILE_NEW: usize = 1001;
    pub const FILE_OPEN: usize = 1002;
    pub const FILE_IMPORT: usize = 1003;
    pub const FILE_IMPORT_CLIPBOARD: usize = 1014;
    pub const FILE_IMPORT_PDF: usize = 1017;
    pub const FILE_EXPORT: usize = 1004;
    pub const FILE_PRINT: usize = 1005;
    pub const FILE_EXIT: usize = 1009;
    pub const FILE_REVEAL: usize = 1010;
    pub const FILE_EXPORT_JPG: usize = 1011;
    pub const FILE_EXPORT_TEXT: usize = 1012;
    pub const FILE_EXPORT_IMAGES: usize = 1013;
    pub const FILE_COPY_IMAGE: usize = 1015;
    pub const FILE_EXPORT_DOCX: usize = 1016;

    pub const EDIT_UNDO: usize = 1101;
    pub const EDIT_REDO: usize = 1102;
    pub const EDIT_DELETE_PAGE: usize = 1103;
    pub const EDIT_ROTATE_RIGHT: usize = 1104;
    pub const EDIT_ROTATE_LEFT: usize = 1105;
    pub const EDIT_MIRROR: usize = 1106;
    pub const EDIT_REPROCESS: usize = 1107;
    pub const EDIT_PERSPECTIVE: usize = 1108;
    pub const EDIT_ANNOTATE: usize = 1109;
    pub const EDIT_MOVE_UP: usize = 1110;
    pub const EDIT_MOVE_DOWN: usize = 1111;
    pub const EDIT_CROP: usize = 1112;
    pub const DEVICE_REFRESH: usize = 1201;
    pub const DEVICE_RESET: usize = 1202;
    pub const DEVICE_PROPERTIES: usize = 1203;

    pub const VIEW_THUMBS: usize = 1301;
    pub const VIEW_ENHANCE: usize = 1302;
    pub const VIEW_DARK: usize = 1303;

    pub const TOOL_OCR: usize = 1401;
    pub const TOOL_SETTINGS: usize = 1402;
    pub const TOOL_OCR_EDIT: usize = 1403;

    pub const HELP_MANUAL: usize = 1501;
    pub const HELP_ABOUT: usize = 1502;

    /// 打开选中的页面（右键菜单/双击）。
    pub const PAGE_OPEN: usize = 1601;

    /// 快捷键"拍照"，与工具栏按钮共用同一命令 ID。
    pub const CAPTURE: usize = 2001;
}

// 加速键标志
const FVIRTKEY: u8 = 0x01;
const FNOINVERT: u8 = 0x02;
const FCONTROL: u8 = 0x08;

/// 构建主窗口菜单栏。
///
/// 调用方负责在窗口销毁时由系统回收菜单句柄。
pub fn build_main_menu() -> Result<HMENU, WinError> {
    // SAFETY: 全部为标准菜单 API，返回值逐一检查。
    unsafe {
        let bar = CreateMenu();
        if bar.is_null() {
            return Err(WinError::last());
        }

        let file = CreatePopupMenu();
        append(file, MF_STRING, id::FILE_NEW, "新建文档(&N)\tCtrl+N")?;
        append(file, MF_STRING, id::FILE_OPEN, "打开工程(&O)…\tCtrl+O")?;
        append(file, MF_STRING, id::FILE_IMPORT, "导入图片(&I)…")?;
        append(file, MF_STRING, id::FILE_IMPORT_PDF, "导入 PDF(&P)…")?;
        append(
            file,
            MF_STRING,
            id::FILE_IMPORT_CLIPBOARD,
            "从剪贴板导入(&V)\tCtrl+V",
        )?;
        append(file, MF_STRING, id::FILE_REVEAL, "打开文件位置(&L)")?;
        append(file, MF_SEPARATOR, 0, "")?;
        append(file, MF_STRING, id::FILE_EXPORT, "导出 PDF(&E)…\tCtrl+E")?;
        append(
            file,
            MF_STRING,
            id::FILE_EXPORT_DOCX,
            "导出 Word (.docx)(&W)…",
        )?;
        append(file, MF_STRING, id::FILE_EXPORT_JPG, "导出当前页 JPG(&J)…")?;
        append(file, MF_STRING, id::FILE_COPY_IMAGE, "复制当前页图片(&C)")?;
        append(
            file,
            MF_STRING,
            id::FILE_EXPORT_TEXT,
            "导出全部 OCR 文本(&T)…",
        )?;
        append(file, MF_STRING, id::FILE_EXPORT_IMAGES, "导出全部 JPG(&G)…")?;
        append(file, MF_STRING, id::FILE_PRINT, "打印(&P)…\tCtrl+P")?;
        append(file, MF_SEPARATOR, 0, "")?;
        append(file, MF_STRING, id::FILE_EXIT, "退出(&X)")?;
        append(bar, MF_POPUP, file as usize, "文件(&F)")?;

        let edit = CreatePopupMenu();
        append(edit, MF_STRING, id::EDIT_UNDO, "撤销(&U)\tCtrl+Z")?;
        append(edit, MF_STRING, id::EDIT_REDO, "重做(&R)\tCtrl+Y")?;
        append(edit, MF_SEPARATOR, 0, "")?;
        append(edit, MF_STRING, id::EDIT_DELETE_PAGE, "删除当前页(&D)\tDel")?;
        append(edit, MF_STRING, id::EDIT_MOVE_UP, "上移当前页(&U)")?;
        append(edit, MF_STRING, id::EDIT_MOVE_DOWN, "下移当前页(&W)")?;
        append(edit, MF_SEPARATOR, 0, "")?;
        append(edit, MF_STRING, id::EDIT_ROTATE_RIGHT, "顺时针旋转 90°(&R)")?;
        append(edit, MF_STRING, id::EDIT_ROTATE_LEFT, "逆时针旋转 90°(&L)")?;
        append(edit, MF_STRING, id::EDIT_MIRROR, "水平镜像(&M)")?;
        append(edit, MF_STRING, id::EDIT_CROP, "裁剪(&K)…")?;
        append(edit, MF_SEPARATOR, 0, "")?;
        append(edit, MF_STRING, id::EDIT_REPROCESS, "重新处理增强(&P)")?;
        append(edit, MF_STRING, id::EDIT_PERSPECTIVE, "四角透视校正(&C)…")?;
        append(edit, MF_STRING, id::EDIT_ANNOTATE, "批注(&A)…")?;
        append(bar, MF_POPUP, edit as usize, "编辑(&E)")?;

        let device = CreatePopupMenu();
        append(
            device,
            MF_STRING,
            id::DEVICE_REFRESH,
            "重新检测设备(&R)\tF5",
        )?;
        append(device, MF_STRING, id::DEVICE_RESET, "重置设备(&S)")?;
        append(device, MF_SEPARATOR, 0, "")?;
        append(
            device,
            MF_STRING,
            id::DEVICE_PROPERTIES,
            "系统摄像头设置(&P)…",
        )?;
        append(bar, MF_POPUP, device as usize, "设备(&D)")?;

        let view = CreatePopupMenu();
        append(view, MF_STRING, id::VIEW_THUMBS, "页面面板(&T)")?;
        append(view, MF_STRING, id::VIEW_ENHANCE, "增强面板(&E)")?;
        append(view, MF_SEPARATOR, 0, "")?;
        append(view, MF_STRING, id::VIEW_DARK, "跟随系统主题(&D)")?;
        append(bar, MF_POPUP, view as usize, "视图(&V)")?;

        let tools = CreatePopupMenu();
        append(tools, MF_STRING, id::TOOL_OCR, "OCR 识别(&O)\tCtrl+Shift+T")?;
        append(tools, MF_STRING, id::TOOL_OCR_EDIT, "编辑 OCR 文本(&E)…")?;
        append(tools, MF_SEPARATOR, 0, "")?;
        append(tools, MF_STRING, id::TOOL_SETTINGS, "设置(&S)…")?;
        append(bar, MF_POPUP, tools as usize, "工具(&T)")?;

        let help = CreatePopupMenu();
        append(help, MF_STRING, id::HELP_MANUAL, "使用说明(&H)\tF1")?;
        append(help, MF_SEPARATOR, 0, "")?;
        append(help, MF_STRING, id::HELP_ABOUT, "关于 usbscan(&A)…")?;
        append(bar, MF_POPUP, help as usize, "帮助(&H)")?;

        Ok(bar)
    }
}

/// 构建页面缩略图右键菜单。
pub fn build_page_menu() -> Result<HMENU, WinError> {
    // SAFETY: 标准菜单 API；菜单句柄由调用方在 TrackPopupMenu 后销毁。
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return Err(WinError::last());
        }
        append(menu, MF_STRING, id::PAGE_OPEN, "打开(&O)")?;
        append(menu, MF_STRING, id::FILE_REVEAL, "在资源管理器中定位(&L)")?;
        append(menu, MF_SEPARATOR, 0, "")?;
        append(menu, MF_STRING, id::EDIT_ROTATE_RIGHT, "顺时针旋转 90°(&R)")?;
        append(menu, MF_STRING, id::EDIT_ROTATE_LEFT, "逆时针旋转 90°(&L)")?;
        append(menu, MF_STRING, id::EDIT_MIRROR, "水平镜像(&M)")?;
        append(menu, MF_STRING, id::EDIT_CROP, "裁剪(&K)…")?;
        append(menu, MF_STRING, id::EDIT_PERSPECTIVE, "四角透视校正(&C)…")?;
        append(menu, MF_STRING, id::EDIT_ANNOTATE, "批注(&A)…")?;
        append(menu, MF_STRING, id::EDIT_REPROCESS, "重新处理增强(&P)")?;
        append(menu, MF_SEPARATOR, 0, "")?;
        append(menu, MF_STRING, id::EDIT_MOVE_UP, "上移(&U)")?;
        append(menu, MF_STRING, id::EDIT_MOVE_DOWN, "下移(&W)")?;
        append(menu, MF_SEPARATOR, 0, "")?;
        append(menu, MF_STRING, id::TOOL_OCR, "OCR 识别(&O)")?;
        append(menu, MF_STRING, id::TOOL_OCR_EDIT, "编辑 OCR 文本(&E)…")?;
        append(menu, MF_STRING, id::FILE_EXPORT_JPG, "导出当前页 JPG(&J)…")?;
        append(menu, MF_STRING, id::FILE_EXPORT_DOCX, "导出 Word (.docx)…")?;
        append(menu, MF_STRING, id::FILE_COPY_IMAGE, "复制图片(&C)")?;
        append(menu, MF_STRING, id::FILE_PRINT, "打印(&P)…")?;
        append(menu, MF_SEPARATOR, 0, "")?;
        append(menu, MF_STRING, id::EDIT_DELETE_PAGE, "删除(&D)")?;
        Ok(menu)
    }
}

/// 构建键盘加速键表。
///
/// 使用加速键而不是 `WM_KEYDOWN`，这样焦点落在工具栏或列表上也照样生效。
pub fn build_accelerators() -> Result<HACCEL, WinError> {
    let keys = [
        ACCEL {
            fVirt: FVIRTKEY | FNOINVERT,
            key: VK_F9,
            cmd: id::CAPTURE as u16,
        },
        ACCEL {
            fVirt: FVIRTKEY | FCONTROL | FNOINVERT,
            key: VK_Z,
            cmd: id::EDIT_UNDO as u16,
        },
        ACCEL {
            fVirt: FVIRTKEY | FCONTROL | FNOINVERT,
            key: VK_Y,
            cmd: id::EDIT_REDO as u16,
        },
        ACCEL {
            fVirt: FVIRTKEY | FNOINVERT,
            key: VK_F5,
            cmd: id::DEVICE_REFRESH as u16,
        },
        ACCEL {
            fVirt: FVIRTKEY | FCONTROL | FNOINVERT,
            key: VK_N,
            cmd: id::FILE_NEW as u16,
        },
        ACCEL {
            fVirt: FVIRTKEY | FCONTROL | FNOINVERT,
            key: VK_E,
            cmd: id::FILE_EXPORT as u16,
        },
        ACCEL {
            fVirt: FVIRTKEY | FCONTROL | FNOINVERT,
            key: VK_P,
            cmd: id::FILE_PRINT as u16,
        },
        ACCEL {
            fVirt: FVIRTKEY | FCONTROL | FNOINVERT,
            key: VK_V,
            cmd: id::FILE_IMPORT_CLIPBOARD as u16,
        },
        ACCEL {
            fVirt: FVIRTKEY | FNOINVERT,
            key: VK_F10,
            cmd: 2002,
        },
        ACCEL {
            fVirt: FVIRTKEY | FNOINVERT,
            key: VK_F11,
            cmd: 2003,
        },
        ACCEL {
            fVirt: FVIRTKEY | FNOINVERT,
            key: VK_F12,
            cmd: 2004,
        },
        ACCEL {
            fVirt: FVIRTKEY | FCONTROL | FNOINVERT,
            key: VK_O,
            cmd: id::FILE_IMPORT as u16,
        },
    ];
    // SAFETY: 数组在调用期间有效，数量与指针一致。
    let table =
        unsafe { CreateAcceleratorTableW(keys.as_ptr(), i32::try_from(keys.len()).unwrap_or(0)) };
    if table.is_null() {
        Err(WinError::last())
    } else {
        Ok(table)
    }
}

unsafe fn append(menu: HMENU, flags: u32, id: usize, text: &str) -> Result<(), WinError> {
    let text = wide(text);
    let ok = unsafe { AppendMenuW(menu, flags, id, text.as_ptr()) };
    if ok == 0 {
        Err(WinError::last())
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 自绘菜单
// ---------------------------------------------------------------------------
//
// Win32 的 HMENU 在系统深色主题下仍由系统按浅色绘制。这里把所有菜单项改成
// owner-draw，用与界面一致的颜色绘制背景、文字、快捷键和勾选标记。

const MENU_TOP_LEVEL: usize = 1 << 31;
const MENU_GUTTER: i32 = 28;
const MENU_RIGHT_PADDING: i32 = 16;

fn item_size(width: i32, height: i32) -> usize {
    (width.clamp(0, 0xFFFF) as usize) | ((height.clamp(0, 0x3FFF) as usize) << 16)
}

fn item_width(data: usize) -> i32 {
    (data & 0xFFFF) as i32
}

fn item_height(data: usize) -> i32 {
    ((data >> 16) & 0x3FFF) as i32
}

/// 把主菜单栏及其下拉菜单改成自绘。
pub fn enable_owner_draw(bar: HMENU) {
    if bar.is_null() {
        return;
    }
    let height = unsafe { GetSystemMetrics(SM_CYMENU).max(20) + 6 };
    // SAFETY: 菜单句柄有效；只修改菜单项属性。
    unsafe {
        let count = GetMenuItemCount(bar);
        for index in 0..count {
            let submenu = GetSubMenu(bar, index);
            if submenu.is_null() {
                continue;
            }
            let popup_width = measure_popup_width(submenu) + MENU_GUTTER + MENU_RIGHT_PADDING;
            let items = GetMenuItemCount(submenu);
            for position in 0..items {
                let state = GetMenuState(submenu, position as u32, MF_BYPOSITION);
                if state & MF_SEPARATOR != 0 {
                    continue;
                }
                let id = GetMenuItemID(submenu, position);
                let data = item_size(popup_width, height);
                let _ = id;
                set_owner_draw(submenu, position as u32, data);
            }
            // 系统不绘制 owner-draw 的顶层菜单项；菜单栏交给
            // SetWindowTheme(DarkMode_Explorer) 与系统主题处理。
            let _ = submenu;
        }
    }
}

/// 把右键菜单改成自绘。
pub fn enable_owner_draw_popup(menu: HMENU) {
    if menu.is_null() {
        return;
    }
    let height = unsafe { GetSystemMetrics(SM_CYMENU).max(20) + 6 };
    // SAFETY: 菜单句柄有效；只修改菜单项属性。
    unsafe {
        let width = measure_popup_width(menu) + MENU_GUTTER + MENU_RIGHT_PADDING;
        let count = GetMenuItemCount(menu);
        for position in 0..count {
            let state = GetMenuState(menu, position as u32, MF_BYPOSITION);
            if state & MF_SEPARATOR != 0 {
                continue;
            }
            let id = GetMenuItemID(menu, position);
            let data = item_size(width, height);
            let _ = id;
            set_owner_draw(menu, position as u32, data);
        }
    }
}

/// 把普通菜单项改成 owner-draw，同时保留命令 ID。
fn set_owner_draw(menu: HMENU, position: u32, data: usize) {
    let id = unsafe { GetMenuItemID(menu, i32::try_from(position).unwrap_or(0)) };
    // SAFETY: 菜单句柄有效；data 作为应用数据传给 WM_DRAWITEM。
    unsafe {
        ModifyMenuW(
            menu,
            position,
            MF_BYPOSITION | MF_OWNERDRAW,
            id as usize,
            data as *const u16,
        );
    }
}

/// 应用菜单背景色。
pub fn refresh_theme(menu: HMENU, dark: bool) {
    if menu.is_null() {
        return;
    }
    // SAFETY: 菜单句柄有效。
    unsafe {
        apply_menu_background(menu, dark);
    }
}

/// 为右键菜单设置与当前主题一致的背景色。
pub fn apply_popup_theme(menu: HMENU, dark: bool) {
    if menu.is_null() {
        return;
    }
    // SAFETY: 菜单句柄有效。
    unsafe {
        apply_menu_background(menu, dark);
    }
}

unsafe fn apply_menu_background(menu: HMENU, dark: bool) {
    let brush = if dark {
        let existing = MENU_BRUSH.load(Ordering::Relaxed);
        if existing != 0 {
            existing as windows_sys::Win32::Graphics::Gdi::HBRUSH
        } else {
            let created = unsafe { CreateSolidBrush(rgb(0x1e, 0x1f, 0x22)) };
            MENU_BRUSH.store(created as usize, Ordering::Relaxed);
            created
        }
    } else {
        unsafe { GetSysColorBrush(COLOR_MENU) }
    };
    let mut info: MENUINFO = unsafe { std::mem::zeroed() };
    info.cbSize = u32::try_from(std::mem::size_of::<MENUINFO>()).unwrap_or(0);
    info.fMask = MIM_BACKGROUND | MIM_APPLYTOSUBMENUS;
    info.hbrBack = brush;
    // SAFETY: info 在调用期间有效。
    unsafe {
        let _ = SetMenuInfo(menu, &info);
    }
}

static MENU_BRUSH: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn measure_popup_width(menu: HMENU) -> i32 {
    // SAFETY: 菜单句柄有效。
    unsafe {
        let dc = GetDC(std::ptr::null_mut());
        if dc.is_null() {
            return 160;
        }
        let old = SelectObject(dc, GetStockObject(DEFAULT_GUI_FONT));
        let count = GetMenuItemCount(menu);
        let mut width = 0;
        for position in 0..count {
            let state = GetMenuState(menu, position as u32, MF_BYPOSITION);
            if state & MF_SEPARATOR != 0 {
                continue;
            }
            let text = menu_text(menu, position as u32, true);
            let (label, shortcut) = split_shortcut(&text);
            width = width.max(text_width_with(dc, &label) + text_width_with(dc, &shortcut) + 24);
        }
        SelectObject(dc, old);
        ReleaseDC(std::ptr::null_mut(), dc);
        width
    }
}

fn menu_text(menu: HMENU, id: u32, by_position: bool) -> String {
    let mut buffer = [0u16; 256];
    // SAFETY: 缓冲区在调用期间有效。
    let len = unsafe {
        GetMenuStringW(
            menu,
            id,
            buffer.as_mut_ptr(),
            i32::try_from(buffer.len()).unwrap_or(256),
            if by_position {
                MF_BYPOSITION
            } else {
                MF_BYCOMMAND
            },
        )
    };
    if len <= 0 {
        String::new()
    } else {
        String::from_utf16_lossy(&buffer[..(len as usize).min(buffer.len())])
    }
}

fn split_shortcut(text: &str) -> (String, String) {
    if let Some((label, shortcut)) = text.split_once('\t') {
        (label.to_string(), shortcut.to_string())
    } else {
        (text.to_string(), String::new())
    }
}

fn text_width_with(dc: windows_sys::Win32::Graphics::Gdi::HDC, text: &str) -> i32 {
    if text.is_empty() {
        return 0;
    }
    let text = wide(text);
    let mut size: windows_sys::Win32::Foundation::SIZE = unsafe { std::mem::zeroed() };
    // SAFETY: 字符串与输出结构在调用期间有效。
    unsafe {
        GetTextExtentPoint32W(
            dc,
            text.as_ptr(),
            i32::try_from(text.len().saturating_sub(1)).unwrap_or(0),
            &mut size,
        );
    }
    size.cx
}

/// 处理 `WM_MEASUREITEM`；不是菜单项时返回 false。
pub fn measure_item(item: *mut MEASUREITEMSTRUCT) -> bool {
    if item.is_null() {
        return false;
    }
    // SAFETY: 指针由系统在 WM_MEASUREITEM 中传入。
    unsafe {
        if (*item).CtlType != ODT_MENU {
            return false;
        }
        let data = (*item).itemData;
        let (width, height) = if data == 0 {
            (120, 28)
        } else {
            (item_width(data), item_height(data))
        };
        (*item).itemWidth = u32::try_from(width).unwrap_or(0);
        (*item).itemHeight = u32::try_from(height).unwrap_or(0);
        true
    }
}

/// 处理菜单的 `WM_DRAWITEM`；不是菜单项时返回 false。
#[allow(clippy::too_many_lines)]
pub fn draw_item(item: *const DRAWITEMSTRUCT, dark: bool) -> bool {
    if item.is_null() {
        return false;
    }
    // SAFETY: 指针由系统在 WM_DRAWITEM 中传入。
    unsafe {
        let item = &*item;
        if item.CtlType != ODT_MENU {
            return false;
        }
        let data = item.itemData;
        let top_level = data & MENU_TOP_LEVEL != 0;
        let id = item.itemID as u32;
        let flags = if top_level {
            MF_BYPOSITION
        } else {
            MF_BYCOMMAND
        };
        let state = GetMenuState(item.hwndItem, id, flags);
        let selected = item.itemState & ODS_SELECTED as u32 != 0;
        let disabled =
            item.itemState & ODS_DISABLED as u32 != 0 || state & (MF_DISABLED | MF_GRAYED) != 0;

        let (bg, fg) = if selected {
            let bg = if dark {
                rgb(0x2d, 0x62, 0xc4)
            } else {
                GetSysColor(COLOR_HIGHLIGHT)
            };
            let fg = if dark {
                rgb(0xff, 0xff, 0xff)
            } else {
                GetSysColor(COLOR_HIGHLIGHTTEXT)
            };
            (bg, fg)
        } else {
            let bg = if dark {
                rgb(0x1e, 0x1f, 0x22)
            } else {
                GetSysColor(COLOR_MENU)
            };
            let fg = if disabled {
                GetSysColor(COLOR_GRAYTEXT)
            } else if dark {
                rgb(0xe6, 0xe8, 0xeb)
            } else {
                GetSysColor(COLOR_MENUTEXT)
            };
            (bg, fg)
        };

        let brush = CreateSolidBrush(bg);
        FillRect(item.hDC, &item.rcItem, brush);
        DeleteObject(brush as HGDIOBJ);
        SetBkMode(item.hDC, TRANSPARENT as i32);
        SetTextColor(item.hDC, fg);

        let text = menu_text(item.hwndItem, id, top_level);
        let (label, shortcut) = split_shortcut(&text);
        let label_w = wide(&label);
        let shortcut_w = wide(&shortcut);
        let old_font = SelectObject(item.hDC, GetStockObject(DEFAULT_GUI_FONT));
        if !top_level && state & MF_CHECKED != 0 {
            let center_x = item.rcItem.left + 14;
            let center_y = (item.rcItem.top + item.rcItem.bottom) / 2;
            let pen = windows_sys::Win32::Graphics::Gdi::CreatePen(
                windows_sys::Win32::Graphics::Gdi::PS_SOLID,
                2,
                fg,
            );
            let old_pen = SelectObject(item.hDC, pen);
            let _ = windows_sys::Win32::Graphics::Gdi::MoveToEx(
                item.hDC,
                center_x - 4,
                center_y,
                std::ptr::null_mut(),
            );
            let _ = windows_sys::Win32::Graphics::Gdi::LineTo(item.hDC, center_x - 1, center_y + 4);
            let _ = windows_sys::Win32::Graphics::Gdi::LineTo(item.hDC, center_x + 5, center_y - 4);
            SelectObject(item.hDC, old_pen);
            DeleteObject(pen as HGDIOBJ);
        }

        let mut text_rect = item.rcItem;
        if top_level {
            DrawTextW(
                item.hDC,
                label_w.as_ptr(),
                i32::try_from(label_w.len().saturating_sub(1)).unwrap_or(0),
                &mut text_rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );
        } else {
            text_rect.left += MENU_GUTTER;
            text_rect.right -= MENU_RIGHT_PADDING;
            if shortcut.is_empty() {
                DrawTextW(
                    item.hDC,
                    label_w.as_ptr(),
                    i32::try_from(label_w.len().saturating_sub(1)).unwrap_or(0),
                    &mut text_rect,
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE,
                );
            } else {
                let mut left_rect = text_rect;
                left_rect.right -= text_width_with(item.hDC, &shortcut) + 24;
                DrawTextW(
                    item.hDC,
                    label_w.as_ptr(),
                    i32::try_from(label_w.len().saturating_sub(1)).unwrap_or(0),
                    &mut left_rect,
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE,
                );
                DrawTextW(
                    item.hDC,
                    shortcut_w.as_ptr(),
                    i32::try_from(shortcut_w.len().saturating_sub(1)).unwrap_or(0),
                    &mut text_rect,
                    DT_RIGHT | DT_VCENTER | DT_SINGLELINE,
                );
            }
        }

        SelectObject(item.hDC, old_font);
        true
    }
}

fn rgb(r: u8, g: u8, b: u8) -> u32 {
    u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16)
}
