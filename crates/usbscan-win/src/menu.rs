// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 原生菜单栏构建。

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    VK_E, VK_F10, VK_F11, VK_F5, VK_F9, VK_N, VK_O, VK_P, VK_V, VK_Y, VK_Z,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateAcceleratorTableW, CreateMenu, CreatePopupMenu, ACCEL, HACCEL, HMENU,
    MF_POPUP, MF_SEPARATOR, MF_STRING,
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
        append(device, MF_STRING, id::DEVICE_PROPERTIES, "摄像头属性(&P)…")?;
        append(bar, MF_POPUP, device as usize, "设备(&D)")?;

        let view = CreatePopupMenu();
        append(view, MF_STRING, id::VIEW_THUMBS, "页面面板(&T)")?;
        append(view, MF_STRING, id::VIEW_ENHANCE, "增强面板(&E)")?;
        append(view, MF_SEPARATOR, 0, "")?;
        append(view, MF_STRING, id::VIEW_DARK, "深色模式(&D)")?;
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
