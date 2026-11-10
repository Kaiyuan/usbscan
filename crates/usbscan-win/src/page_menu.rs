// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 页面缩略图的原生右键菜单。

use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::Graphics::Gdi::{DeleteObject, GetSysColor, COLOR_MENUTEXT, HBITMAP};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, GetSystemMetrics, PostMessageW,
    SetForegroundWindow, SetMenuItemBitmaps, TrackPopupMenu, MF_BYCOMMAND, MF_SEPARATOR, MF_STRING,
    SM_CXMENUCHECK, SM_CYMENUCHECK, TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TPM_TOPALIGN,
    WM_NULL,
};

use crate::icons;
use crate::window::main_window;

// Tabler Icons 字形码点。
const ICON_OPEN: u32 = 0xEB0A; // photo
const ICON_RENAME: u32 = 0xEB04; // pencil
const ICON_REVEAL: u32 = 0xFAF7; // folder-open
const ICON_AUTO_CROP: u32 = 0xEA85; // crop
const ICON_AUTO_DESKEW: u32 = 0xEB16; // rotate
const ICON_OCR: u32 = 0xF204; // text-recognition
const ICON_EXPORT_JPG: u32 = 0xEDE9; // file-export
const ICON_DELETE: u32 = 0xEB41; // trash

/// 右键菜单选择结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageMenuCommand {
    None,
    Open,
    Rename,
    RevealInExplorer,
    AutoCrop,
    AutoDeskew,
    Ocr,
    ExportJpg,
    Delete,
}

impl PageMenuCommand {
    fn from_id(id: i32) -> Self {
        match id {
            1 => Self::Open,
            2 => Self::Rename,
            3 => Self::RevealInExplorer,
            4 => Self::AutoCrop,
            5 => Self::AutoDeskew,
            6 => Self::Ocr,
            7 => Self::ExportJpg,
            8 => Self::Delete,
            _ => Self::None,
        }
    }
}

/// 右键菜单标签。由主程序按当前语言提供。
#[derive(Clone, Copy, Debug)]
pub struct PageMenuLabels<'a> {
    pub open: &'a str,
    pub rename: &'a str,
    pub reveal: &'a str,
    pub auto_crop: &'a str,
    pub auto_deskew: &'a str,
    pub ocr: &'a str,
    pub export_jpg: &'a str,
    pub delete: &'a str,
}

/// 在鼠标位置弹出页面右键菜单并返回用户选择。
#[must_use]
pub fn show_page_menu(labels: &PageMenuLabels<'_>) -> PageMenuCommand {
    let hwnd = main_window();
    if hwnd.is_null() {
        return PageMenuCommand::None;
    }
    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: 标准 Win32 调用，指针均有效。
    unsafe {
        if GetCursorPos(&mut point) == 0 {
            return PageMenuCommand::None;
        }
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return PageMenuCommand::None;
        }

        let mut bitmaps = Vec::new();
        let size = GetSystemMetrics(SM_CXMENUCHECK)
            .max(GetSystemMetrics(SM_CYMENUCHECK))
            .max(12);
        let color = GetSysColor(COLOR_MENUTEXT);

        append_item(menu, &mut bitmaps, 1, labels.open, ICON_OPEN, size, color);
        append_item(
            menu,
            &mut bitmaps,
            2,
            labels.rename,
            ICON_RENAME,
            size,
            color,
        );
        append_item(
            menu,
            &mut bitmaps,
            3,
            labels.reveal,
            ICON_REVEAL,
            size,
            color,
        );
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        append_item(
            menu,
            &mut bitmaps,
            4,
            labels.auto_crop,
            ICON_AUTO_CROP,
            size,
            color,
        );
        append_item(
            menu,
            &mut bitmaps,
            5,
            labels.auto_deskew,
            ICON_AUTO_DESKEW,
            size,
            color,
        );
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        append_item(menu, &mut bitmaps, 6, labels.ocr, ICON_OCR, size, color);
        append_item(
            menu,
            &mut bitmaps,
            7,
            labels.export_jpg,
            ICON_EXPORT_JPG,
            size,
            color,
        );
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        append_item(
            menu,
            &mut bitmaps,
            8,
            labels.delete,
            ICON_DELETE,
            size,
            color,
        );

        // 让菜单在点击外部时能正常关闭。
        SetForegroundWindow(hwnd);
        let command = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_LEFTALIGN | TPM_TOPALIGN,
            point.x,
            point.y,
            0,
            hwnd,
            std::ptr::null(),
        );
        DestroyMenu(menu);
        for bitmap in bitmaps {
            DeleteObject(bitmap);
        }
        let _ = PostMessageW(hwnd, WM_NULL, 0, 0);
        PageMenuCommand::from_id(command)
    }
}

/// 追加一个带图标的菜单项。
///
/// `SetMenuItemBitmaps` 不复制位图，因此创建的位图要由调用方在菜单销毁后释放。
unsafe fn append_item(
    menu: windows_sys::Win32::UI::WindowsAndMessaging::HMENU,
    bitmaps: &mut Vec<HBITMAP>,
    id: usize,
    label: &str,
    codepoint: u32,
    size: i32,
    color: u32,
) {
    let label: Vec<u16> = label.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: 菜单句柄与字符串指针由调用方保证有效。
    unsafe {
        AppendMenuW(menu, MF_STRING, id, label.as_ptr());
        let bitmap = icons::menu_bitmap(codepoint, size, color);
        if bitmap.is_null() {
            return;
        }
        if SetMenuItemBitmaps(menu, id as u32, MF_BYCOMMAND, bitmap, bitmap) == 0 {
            DeleteObject(bitmap);
        } else {
            bitmaps.push(bitmap);
        }
    }
}
