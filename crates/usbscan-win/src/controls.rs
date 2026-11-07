// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 原生控件创建与操作。
//!
//! 这一层只做"创建系统控件 + 发消息"，不含业务逻辑。
//! 所有控件都是真实 Win32 控件，因此天然获得主题、无障碍与键盘导航支持。

use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS,
};
use windows_sys::Win32::UI::Controls::{
    ImageList_Add, ImageList_Create, ImageList_Destroy, HIMAGELIST, ILC_COLOR32, LVIF_IMAGE,
    LVIF_TEXT, LVITEMW, LVM_DELETEITEM, LVM_GETNEXTITEM, LVM_INSERTITEMW, LVM_SETBKCOLOR,
    LVM_SETIMAGELIST, LVM_SETTEXTBKCOLOR, LVM_SETTEXTCOLOR, LVSIL_NORMAL, SB_SETMINHEIGHT,
    SB_SETPARTS, SB_SETTEXTW, TBM_SETPOS, TBM_SETRANGE, TBM_SETTICFREQ, TB_ADDBUTTONS,
    TB_ADDSTRINGW, TB_AUTOSIZE, TB_BUTTONSTRUCTSIZE, TB_SETBUTTONSIZE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, GetWindowTextLengthW, GetWindowTextW, SendMessageW, BS_AUTOCHECKBOX,
    BS_GROUPBOX, BS_PUSHBUTTON, WS_CHILD, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

use crate::text::wide;

// 工具栏风格位
const TBSTYLE_FLAT: u32 = 0x0800;
const TBSTYLE_LIST: u32 = 0x1000;
const CCS_NODIVIDER: u32 = 0x0040;
const CCS_NORESIZE: u32 = 0x0010;
const TB_SETIMAGELIST: u32 = 0x0430;
const BTNS_BUTTON: u8 = 0x0000;
const BTNS_SEP: u8 = 0x0001;
const BTNS_AUTOSIZE: u8 = 0x0010;
const BTNS_SHOWTEXT: u8 = 0x0040;
const TBSTATE_ENABLED: u8 = 0x04;
const I_IMAGENONE: i32 = -2;

// 状态栏风格
const SBARS_SIZEGRIP: u32 = 0x0100;

// 列表视图风格
const LVS_ICON: u32 = 0x0000;
const LVS_SINGLESEL: u32 = 0x0004;
const LVS_SHOWSELALWAYS: u32 = 0x0008;
const LVS_SHAREIMAGELISTS: u32 = 0x0040;
const LVS_NOSORTHEADER: u32 = 0x8000;
const LVM_DELETEALLITEMS: u32 = 0x1009;
const LVM_SETITEMSTATE: u32 = 0x102B;
const LVNI_SELECTED: u32 = 0x0002;
const LVIS_FOCUSED: u32 = 0x0001;
const LVIS_SELECTED: u32 = 0x0002;
const LVIF_STATE: u32 = 0x0008;

// 轨道条风格
const TBS_HORZ: u32 = 0x0000;
const TBS_AUTOTICKS: u32 = 0x0001;
const TBM_GETPOS: u32 = 0x0400;

// 编辑框风格
const ES_AUTOHSCROLL: u32 = 0x0080;
const WS_EX_CLIENTEDGE: u32 = 0x0000_0200;

// 复选框消息
const BM_GETCHECK: u32 = 0x00F0;
const BM_SETCHECK: u32 = 0x00F1;
const BST_CHECKED: usize = 1;

// 下拉框风格与消息
const CBS_DROPDOWNLIST: u32 = 0x0003;
const CB_ADDSTRING: u32 = 0x0143;
const CB_GETCURSEL: u32 = 0x0147;
const CB_RESETCONTENT: u32 = 0x014B;
const CB_SETCURSEL: u32 = 0x014E;

/// `TBBUTTON` 的本地定义，避免依赖 crate 中可能变动的布局。
///
/// 布局必须与 `commctrl.h` 完全一致：
/// `iBitmap(i32) idCommand(i32) fsState(u8) fsStyle(u8) bReserved[2] dwData(usize) iString(isize)`
#[repr(C)]
struct TbButton {
    bitmap: i32,
    command: i32,
    state: u8,
    style: u8,
    reserved: [u8; 2],
    data: usize,
    string: isize,
}

/// 工具栏按钮描述。
pub struct ToolButton {
    pub id: usize,
    pub label: &'static str,
    pub separator_before: bool,
    /// Tabler Icons 图标；`None` 表示纯文字按钮。
    pub icon: Option<crate::icons::Icon>,
}

/// 创建状态栏。
pub fn create_status_bar(parent: HWND, hinstance: HINSTANCE) -> HWND {
    let class = wide("msctls_statusbar32");
    let empty = wide("");
    // SAFETY: 参数均为有效的 Win32 值。
    unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            empty.as_ptr(),
            WS_CHILD | WS_VISIBLE | SBARS_SIZEGRIP,
            0,
            0,
            0,
            0,
            parent,
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null(),
        )
    }
}

/// 设置状态栏分区宽度。最后一项传 `-1` 表示自动占满剩余空间。
pub fn set_status_parts(status: HWND, parts: &[i32]) {
    if status.is_null() || parts.is_empty() {
        return;
    }
    // SAFETY: parts 在调用期间有效，数量与指针一致。
    unsafe {
        SendMessageW(status, SB_SETPARTS, parts.len(), parts.as_ptr() as LPARAM);
    }
}

/// 设置某个分区的文本。
pub fn set_status_text(status: HWND, part: usize, text: &str) {
    if status.is_null() {
        return;
    }
    let text = wide(text);
    // SAFETY: 字符串在调用期间有效。
    unsafe {
        SendMessageW(status, SB_SETTEXTW, part, text.as_ptr() as LPARAM);
    }
}

/// 设置状态栏最小高度（逻辑像素，已按 DPI 缩放）。
pub fn set_status_min_height(status: HWND, height: i32) {
    if status.is_null() {
        return;
    }
    // SAFETY: 纯消息调用。
    unsafe {
        SendMessageW(status, SB_SETMINHEIGHT, height.max(0) as usize, 0);
    }
}

/// 设置列表视图的背景色与文字色。
pub fn set_list_colors(list: HWND, background: u32, text: u32) {
    if list.is_null() {
        return;
    }
    // SAFETY: 纯消息调用，颜色为普通整数。
    unsafe {
        SendMessageW(list, LVM_SETBKCOLOR, 0, background as LPARAM);
        SendMessageW(list, LVM_SETTEXTBKCOLOR, 0, background as LPARAM);
        SendMessageW(list, LVM_SETTEXTCOLOR, 0, text as LPARAM);
    }
}

/// 往页面列表追加一项。
///
/// `image` 是图像列表中的索引；`None` 表示只显示文字。
pub fn add_page_item(list: HWND, index: usize, text: &str, image: Option<usize>) {
    if list.is_null() {
        return;
    }
    let text = wide(text);
    // SAFETY: 文本在调用期间有效；LVITEMW 布局与系统一致。
    unsafe {
        let mut item: LVITEMW = std::mem::zeroed();
        item.mask = LVIF_TEXT;
        if let Some(image) = image {
            item.mask |= LVIF_IMAGE;
            item.iImage = i32::try_from(image).unwrap_or(I_IMAGENONE);
        } else {
            item.iImage = I_IMAGENONE;
        }
        item.iItem = i32::try_from(index).unwrap_or(i32::MAX);
        item.pszText = text.as_ptr().cast_mut();
        SendMessageW(
            list,
            LVM_INSERTITEMW,
            0,
            std::ptr::from_ref(&item) as LPARAM,
        );
    }
}

/// 创建缩略图图像列表。
#[must_use]
pub fn create_image_list(width: i32, height: i32) -> HIMAGELIST {
    // SAFETY: 标准控件 API；返回空句柄时调用方会跳过图像。
    unsafe { ImageList_Create(width, height, ILC_COLOR32, 4, 16) }
}

/// 把图像列表绑定到列表视图（图标模式）。
pub fn set_list_image_list(list: HWND, images: HIMAGELIST) {
    if list.is_null() {
        return;
    }
    // SAFETY: 纯消息调用；图像列表由调用方保证生命周期。
    unsafe {
        SendMessageW(list, LVM_SETIMAGELIST, LVSIL_NORMAL as usize, images);
    }
}

/// 把 BGRA 缩略图居中放进透明画布并加入图像列表。
///
/// 返回图像索引；失败时返回 `None`（列表仍显示文字）。
pub fn add_image_bgra(
    images: HIMAGELIST,
    canvas_width: u32,
    canvas_height: u32,
    width: u32,
    height: u32,
    bgra: &[u8],
) -> Option<usize> {
    if images == 0 || width == 0 || height == 0 || canvas_width == 0 || canvas_height == 0 {
        return None;
    }
    if width > canvas_width || height > canvas_height {
        return None;
    }
    let expected = width as usize * height as usize * 4;
    if bgra.len() < expected {
        return None;
    }

    let mut info: BITMAPINFO = unsafe { std::mem::zeroed() };
    info.bmiHeader = BITMAPINFOHEADER {
        biSize: u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>()).unwrap_or(40),
        biWidth: i32::try_from(canvas_width).ok()?,
        // 负高度表示 top-down，与 BGRA 内存布局一致。
        biHeight: -i32::try_from(canvas_height).ok()?,
        biPlanes: 1,
        biBitCount: 32,
        ..unsafe { std::mem::zeroed() }
    };

    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    // SAFETY: BITMAPINFO 与输出指针在调用期间有效；返回的 HBITMAP 由本函数释放。
    let bitmap = unsafe {
        CreateDIBSection(
            std::ptr::null_mut(),
            &info,
            DIB_RGB_COLORS,
            &raw mut bits,
            std::ptr::null_mut(),
            0,
        )
    };
    if bitmap.is_null() || bits.is_null() {
        return None;
    }

    // 透明画布 + 居中缩略图；像素为不透明 BGRA，无需预乘。
    let canvas_len = canvas_width as usize * canvas_height as usize * 4;
    // SAFETY: CreateDIBSection 保证缓冲区至少为 canvas_len 字节。
    let canvas = unsafe { std::slice::from_raw_parts_mut(bits.cast::<u8>(), canvas_len) };
    canvas.fill(0);
    let offset_x = (canvas_width - width) as usize / 2;
    let offset_y = (canvas_height - height) as usize / 2;
    for y in 0..height as usize {
        let src = y * width as usize * 4;
        let dst = ((y + offset_y) * canvas_width as usize + offset_x) * 4;
        canvas[dst..dst + width as usize * 4].copy_from_slice(&bgra[src..src + width as usize * 4]);
    }

    // SAFETY: 图像列表句柄有效；位图由 ImageList_Add 复制，随后释放。
    let index = unsafe { ImageList_Add(images, bitmap, std::ptr::null_mut()) };
    unsafe {
        DeleteObject(bitmap);
    }
    if index < 0 {
        None
    } else {
        usize::try_from(index).ok()
    }
}

/// 释放图像列表。
pub fn destroy_image_list(images: HIMAGELIST) {
    if images != 0 {
        // SAFETY: 句柄由 create_image_list 创建，且只释放一次。
        unsafe {
            ImageList_Destroy(images);
        }
    }
}

/// 清空页面列表。
pub fn clear_page_list(list: HWND) {
    if list.is_null() {
        return;
    }
    // SAFETY: 纯消息调用。
    unsafe {
        SendMessageW(list, LVM_DELETEALLITEMS, 0, 0);
    }
}

/// 返回页面列表当前选中的索引。
#[must_use]
pub fn list_selected(list: HWND) -> Option<usize> {
    if list.is_null() {
        return None;
    }
    // SAFETY: 纯消息调用；wParam = -1 表示从头查找。
    let index = unsafe { SendMessageW(list, LVM_GETNEXTITEM, usize::MAX, LVNI_SELECTED as LPARAM) };
    if index < 0 {
        None
    } else {
        usize::try_from(index).ok()
    }
}

/// 选中列表中的指定项。
pub fn set_list_selected(list: HWND, index: usize) {
    if list.is_null() {
        return;
    }
    // SAFETY: LVITEMW 为 POD，索引在调用方范围内。
    unsafe {
        let mut item: LVITEMW = std::mem::zeroed();
        item.mask = LVIF_STATE;
        item.state = LVIS_SELECTED | LVIS_FOCUSED;
        item.stateMask = LVIS_SELECTED | LVIS_FOCUSED;
        SendMessageW(
            list,
            LVM_SETITEMSTATE,
            index,
            std::ptr::from_ref(&item) as LPARAM,
        );
    }
}

/// 删除页面列表的最后一项。
pub fn delete_last_page(list: HWND, count_after: usize) {
    if list.is_null() {
        return;
    }
    // SAFETY: 纯消息调用。
    unsafe {
        SendMessageW(list, LVM_DELETEITEM, count_after, 0);
    }
}

/// 绑定工具栏图像列表。
pub fn set_toolbar_images(toolbar: HWND, images: HIMAGELIST) {
    if toolbar.is_null() {
        return;
    }
    // SAFETY: 纯消息调用；图像列表生命周期由调用方管理。
    unsafe {
        SendMessageW(toolbar, TB_SETIMAGELIST, 0, images);
    }
}

/// 设置状态栏背景色（`SB_SETBKCOLOR`）。
pub fn set_status_background(status: HWND, color: u32) {
    if status.is_null() {
        return;
    }
    // SAFETY: 纯消息调用。
    unsafe {
        SendMessageW(status, 8193, 0, color as LPARAM);
    }
}

/// 字体变化后重新计算工具栏按钮尺寸。
pub fn autosize_toolbar(toolbar: HWND) {
    if toolbar.is_null() {
        return;
    }
    // SAFETY: 纯消息调用。
    unsafe {
        SendMessageW(toolbar, TB_AUTOSIZE, 0, 0);
    }
}

/// 创建工具栏。
///
/// 当前使用纯文本按钮；等图标资源就绪后改为 `TB_ADDBITMAP`。
pub fn create_toolbar(
    parent: HWND,
    hinstance: HINSTANCE,
    buttons: &[ToolButton],
    height: i32,
    dpi: u32,
) -> HWND {
    let class = wide("ToolbarWindow32");
    let empty = wide("");
    let style = WS_CHILD | WS_VISIBLE | TBSTYLE_FLAT | TBSTYLE_LIST | CCS_NODIVIDER | CCS_NORESIZE;

    // SAFETY: 标准工具栏创建流程。
    unsafe {
        let bar = CreateWindowExW(
            0,
            class.as_ptr(),
            empty.as_ptr(),
            style,
            0,
            0,
            0,
            height,
            parent,
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null(),
        );
        if bar.is_null() {
            return bar;
        }

        SendMessageW(bar, TB_BUTTONSTRUCTSIZE, std::mem::size_of::<TbButton>(), 0);
        SendMessageW(
            bar,
            TB_SETBUTTONSIZE,
            0,
            ((crate::dpi::scale(26, dpi) & 0xFFFF) << 16 | (crate::dpi::scale(120, dpi) & 0xFFFF))
                as LPARAM,
        );

        let mut raw: Vec<TbButton> = Vec::with_capacity(buttons.len() * 2);
        for b in buttons {
            if b.separator_before {
                raw.push(TbButton {
                    bitmap: I_IMAGENONE,
                    command: 0,
                    state: 0,
                    style: BTNS_SEP,
                    reserved: [0; 2],
                    data: 0,
                    string: -1,
                });
            }
            let label = wide(b.label);
            let index = SendMessageW(bar, TB_ADDSTRINGW, 0, label.as_ptr() as LPARAM);
            raw.push(TbButton {
                bitmap: b.icon.map_or(I_IMAGENONE, |icon| {
                    i32::try_from(icon.index()).unwrap_or(I_IMAGENONE)
                }),
                command: i32::try_from(b.id).unwrap_or(0),
                state: TBSTATE_ENABLED,
                style: BTNS_BUTTON | BTNS_AUTOSIZE | BTNS_SHOWTEXT,
                reserved: [0; 2],
                data: 0,
                string: index,
            });
        }

        SendMessageW(bar, TB_ADDBUTTONS, raw.len(), raw.as_ptr() as LPARAM);
        SendMessageW(bar, TB_AUTOSIZE, 0, 0);
        bar
    }
}

/// 创建页面缩略图列表（图标模式）。
pub fn create_page_list(parent: HWND, hinstance: HINSTANCE) -> HWND {
    let class = wide("SysListView32");
    let empty = wide("");
    let style = WS_CHILD
        | WS_VISIBLE
        | LVS_ICON
        | LVS_SINGLESEL
        | LVS_SHOWSELALWAYS
        | LVS_SHAREIMAGELISTS
        | LVS_NOSORTHEADER;
    // SAFETY: 标准控件创建。
    unsafe {
        CreateWindowExW(
            ws_ex_clientedge(),
            class.as_ptr(),
            empty.as_ptr(),
            style,
            0,
            0,
            0,
            0,
            parent,
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null(),
        )
    }
}

/// 创建单行编辑框。
pub fn create_edit(parent: HWND, hinstance: HINSTANCE, id: usize) -> HWND {
    let class = wide("EDIT");
    let empty = wide("");
    // SAFETY: 标准控件创建。
    unsafe {
        CreateWindowExW(
            WS_EX_CLIENTEDGE,
            class.as_ptr(),
            empty.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL,
            0,
            0,
            0,
            0,
            parent,
            id as usize as _,
            hinstance,
            std::ptr::null(),
        )
    }
}

/// 创建普通按钮。
pub fn create_push_button(parent: HWND, hinstance: HINSTANCE, id: usize, text: &str) -> HWND {
    let class = wide("BUTTON");
    let text = wide(text);
    // SAFETY: 标准控件创建。
    unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            text.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | (BS_PUSHBUTTON as u32),
            0,
            0,
            0,
            0,
            parent,
            id as usize as _,
            hinstance,
            std::ptr::null(),
        )
    }
}

/// 读取控件文本。
#[must_use]
pub fn get_text(control: HWND) -> String {
    if control.is_null() {
        return String::new();
    }
    // SAFETY: 先查询长度，再按长度分配缓冲区。
    unsafe {
        let len = GetWindowTextLengthW(control);
        if len <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; len as usize + 1];
        let copied = GetWindowTextW(control, buffer.as_mut_ptr(), buffer.len() as i32);
        String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
    }
}

/// 设置控件文本。
pub fn set_text(control: HWND, text: &str) {
    if control.is_null() {
        return;
    }
    let text = wide(text);
    // SAFETY: 字符串在调用期间有效。
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW(control, text.as_ptr());
    }
}

/// 读取轨道条当前位置。
#[must_use]
pub fn track_position(track: HWND) -> i32 {
    if track.is_null() {
        return 0;
    }
    // SAFETY: 纯消息调用。
    unsafe { SendMessageW(track, TBM_GETPOS, 0, 0) as i32 }
}

/// 设置轨道条位置。
pub fn set_track_position(track: HWND, value: i32) {
    if track.is_null() {
        return;
    }
    // SAFETY: 纯消息调用。
    unsafe {
        SendMessageW(track, TBM_SETPOS, 1, value as LPARAM);
    }
}

/// 启用/禁用控件。
pub fn set_enabled(control: HWND, enabled: bool) {
    if control.is_null() {
        return;
    }
    // SAFETY: 纯 Win32 调用。
    unsafe {
        windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow(control, i32::from(enabled));
    }
}

/// 读取复选框状态。
#[must_use]
pub fn checkbox_checked(control: HWND) -> bool {
    if control.is_null() {
        return false;
    }
    // SAFETY: 纯消息调用。
    unsafe { SendMessageW(control, BM_GETCHECK, 0, 0) == BST_CHECKED as isize }
}

/// 设置复选框状态。
pub fn set_checkbox(control: HWND, checked: bool) {
    if control.is_null() {
        return;
    }
    // SAFETY: 纯消息调用。
    unsafe {
        SendMessageW(
            control,
            BM_SETCHECK,
            if checked { BST_CHECKED } else { 0 },
            0,
        );
    }
}

/// 创建分组框。
pub fn create_group(parent: HWND, hinstance: HINSTANCE, text: &str) -> HWND {
    let class = wide("BUTTON");
    let text = wide(text);
    // SAFETY: 标准控件创建。
    unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            text.as_ptr(),
            WS_CHILD | WS_VISIBLE | (BS_GROUPBOX as u32),
            0,
            0,
            0,
            0,
            parent,
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null(),
        )
    }
}

/// 创建复选框。
pub fn create_checkbox(parent: HWND, hinstance: HINSTANCE, id: usize, text: &str) -> HWND {
    let class = wide("BUTTON");
    let text = wide(text);
    // SAFETY: 标准控件创建。
    unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            text.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | (BS_AUTOCHECKBOX as u32),
            0,
            0,
            0,
            0,
            parent,
            id as usize as _,
            hinstance,
            std::ptr::null(),
        )
    }
}

/// 创建设备下拉框。
///
/// 下拉列表的高度由窗口高度决定，布局时传入较大的高度，
/// 控件闭合时只绘制一行。
pub fn create_combo_box(parent: HWND, hinstance: HINSTANCE, id: usize) -> HWND {
    let class = wide("COMBOBOX");
    let empty = wide("");
    // SAFETY: 标准控件创建。
    unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            empty.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | CBS_DROPDOWNLIST,
            0,
            0,
            0,
            0,
            parent,
            id as usize as _,
            hinstance,
            std::ptr::null(),
        )
    }
}

/// 用新的设备列表替换下拉框内容，并选中指定项。
pub fn set_combo_items(combo: HWND, items: &[String], selected: Option<usize>) {
    if combo.is_null() {
        return;
    }
    // SAFETY: 字符串在每次 SendMessage 调用期间保持存活，控件会复制内容。
    unsafe {
        SendMessageW(combo, CB_RESETCONTENT, 0, 0);
        for item in items {
            let text = wide(item);
            SendMessageW(combo, CB_ADDSTRING, 0, text.as_ptr() as LPARAM);
        }
        set_combo_index(combo, selected);
    }
}

/// 设置下拉框当前选中项；`None` 表示清空选择。
pub fn set_combo_index(combo: HWND, index: Option<usize>) {
    if combo.is_null() {
        return;
    }
    let index = index.unwrap_or(usize::MAX);
    // SAFETY: 纯消息调用。
    unsafe {
        SendMessageW(combo, CB_SETCURSEL, index, 0);
    }
}

/// 读取下拉框当前选中项。
#[must_use]
pub fn combo_selection(combo: HWND) -> Option<usize> {
    if combo.is_null() {
        return None;
    }
    // SAFETY: 纯消息调用。
    let index = unsafe { SendMessageW(combo, CB_GETCURSEL, 0, 0) };
    usize::try_from(index).ok()
}

/// 创建水平轨道条。
pub fn create_trackbar(parent: HWND, hinstance: HINSTANCE, id: usize) -> HWND {
    let class = wide("msctls_trackbar32");
    let empty = wide("");
    // SAFETY: 标准控件创建。
    unsafe {
        let bar = CreateWindowExW(
            0,
            class.as_ptr(),
            empty.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | TBS_HORZ | TBS_AUTOTICKS,
            0,
            0,
            0,
            0,
            parent,
            id as usize as _,
            hinstance,
            std::ptr::null(),
        );
        if !bar.is_null() {
            SendMessageW(bar, TBM_SETRANGE, 1, (100_u32 << 16) as LPARAM);
            SendMessageW(bar, TBM_SETPOS, 1, 50);
            SendMessageW(bar, TBM_SETTICFREQ, 10, 0);
        }
        bar
    }
}

/// 创建静态文本。
pub fn create_label(parent: HWND, hinstance: HINSTANCE, text: &str) -> HWND {
    let class = wide("STATIC");
    let text = wide(text);
    // SAFETY: 标准控件创建。
    unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            text.as_ptr(),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            0,
            0,
            parent,
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null(),
        )
    }
}

const fn ws_ex_clientedge() -> u32 {
    0x0000_0200
}
