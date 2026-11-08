// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 页面缩略图的原生右键菜单。

use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, PostMessageW, SetForegroundWindow,
    TrackPopupMenu, MF_SEPARATOR, MF_STRING, TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TPM_TOPALIGN, WM_NULL,
};

use crate::window::main_window;

/// 右键菜单选择结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageMenuCommand {
    None,
    Rename,
    AutoCrop,
    AutoDeskew,
    Ocr,
    ExportJpg,
    Delete,
}

impl PageMenuCommand {
    fn from_id(id: i32) -> Self {
        match id {
            1 => Self::Rename,
            2 => Self::AutoCrop,
            3 => Self::AutoDeskew,
            4 => Self::Ocr,
            5 => Self::ExportJpg,
            6 => Self::Delete,
            _ => Self::None,
        }
    }
}

/// 右键菜单标签。由主程序按当前语言提供。
#[derive(Clone, Copy, Debug)]
pub struct PageMenuLabels<'a> {
    pub rename: &'a str,
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
        for (id, label) in [
            (1usize, labels.rename),
            (2, labels.auto_crop),
            (3, labels.auto_deskew),
            (4, labels.ocr),
            (5, labels.export_jpg),
        ] {
            let label: Vec<u16> = label.encode_utf16().chain(std::iter::once(0)).collect();
            AppendMenuW(menu, MF_STRING, id, label.as_ptr());
        }
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        let label: Vec<u16> = labels
            .delete
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        AppendMenuW(menu, MF_STRING, 6, label.as_ptr());

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
        let _ = PostMessageW(hwnd, WM_NULL, 0, 0);
        PageMenuCommand::from_id(command)
    }
}
