// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! Tabler Icons 工具栏图标。
//!
//! 图标字体来自 <https://github.com/tabler/tabler-icons>（MIT）。
//! 这里在运行时把字体中的字形渲染成 32 位 BGRA 位图，再交给工具栏的
//! `HIMAGELIST`，因此不需要在仓库里保存几十张 PNG。

use std::sync::OnceLock;

use windows_sys::Win32::Foundation::SIZE;
use windows_sys::Win32::Graphics::Gdi::{
    AddFontMemResourceEx, CreateCompatibleDC, CreateDIBSection, CreateFontIndirectW, DeleteDC,
    DeleteObject, GetTextExtentPoint32W, SelectObject, SetBkMode, SetTextColor, TextOutW,
    BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, HBITMAP, LOGFONTW,
};
use windows_sys::Win32::UI::Controls::{ImageList_Add, ImageList_Create, HIMAGELIST, ILC_COLOR32};

/// 嵌入的可缩放图标字体（约 1.8 MB）。
const FONT_DATA: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/icons/tabler-icons.ttf"
));

const FONT_FACE: &str = "tabler-icons";
const ANTIALIASED_QUALITY: u8 = 4;
const DEFAULT_CHARSET: u8 = 1;
const FW_NORMAL: i32 = 400;

/// 工具栏图标。顺序必须与 `ALL` 和 `index()` 保持一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum Icon {
    New = 0,
    Capture = 1,
    Burst = 2,
    Delay = 3,
    Video = 4,
    Crop = 5,
    Deskew = 6,
    Ocr = 7,
    Export = 8,
    Reveal = 9,
    Print = 10,
}

const ALL: [Icon; 11] = [
    Icon::New,
    Icon::Capture,
    Icon::Burst,
    Icon::Delay,
    Icon::Video,
    Icon::Crop,
    Icon::Deskew,
    Icon::Ocr,
    Icon::Export,
    Icon::Reveal,
    Icon::Print,
];

impl Icon {
    /// 该图标在图像列表中的索引。
    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Tabler Icons 字体中的私有区码点。
    const fn codepoint(self) -> u32 {
        match self {
            Self::New => 0xEAA0,     // file-plus
            Self::Capture => 0xEA54, // camera
            Self::Burst => 0xEC3B,   // camera-plus
            Self::Delay => 0xEA70,   // clock
            Self::Video => 0xED22,   // video
            Self::Crop => 0xEA85,    // crop
            Self::Deskew => 0xEB16,  // rotate
            Self::Ocr => 0xF204,     // text-recognition
            Self::Export => 0xEDE9,  // file-export
            Self::Reveal => 0xFAF7,  // folder-open
            Self::Print => 0xEB0E,   // printer
        }
    }
}

/// 创建工具栏图标列表。
///
/// `size` 是物理像素尺寸，`color` 是 `COLORREF`（`0x00BBGGRR`）。
/// 失败时返回 0，工具栏仍会回退为纯文字按钮。
#[must_use]
pub fn create_toolbar_icons(size: i32, color: u32) -> HIMAGELIST {
    if !ensure_font() {
        return 0;
    }
    let size = size.max(8);
    // SAFETY: 标准 ImageList 创建；失败时返回空句柄。
    let list = unsafe { ImageList_Create(size, size, ILC_COLOR32, 16, 4) };
    if list == 0 {
        return 0;
    }

    for icon in ALL {
        let bitmap = render_glyph(icon.codepoint(), size, color)
            .unwrap_or_else(|| blank_bitmap(size).unwrap_or(std::ptr::null_mut()));
        if bitmap.is_null() {
            continue;
        }
        // SAFETY: 位图由本函数创建，加入后即可释放。
        unsafe {
            ImageList_Add(list, bitmap, std::ptr::null_mut());
            DeleteObject(bitmap);
        }
    }
    list
}

/// 把内存中的 TTF 注册到当前进程。
fn ensure_font() -> bool {
    static LOADED: OnceLock<bool> = OnceLock::new();
    *LOADED.get_or_init(|| {
        let mut count = 0u32;
        // SAFETY: 字体数据是静态嵌入的，生命周期覆盖整个进程。
        let handle = unsafe {
            AddFontMemResourceEx(
                FONT_DATA.as_ptr().cast(),
                u32::try_from(FONT_DATA.len()).unwrap_or(0),
                std::ptr::null(),
                &raw mut count,
            )
        };
        !handle.is_null() && count > 0
    })
}

fn blank_bitmap(size: i32) -> Option<HBITMAP> {
    // SAFETY: 创建一个全透明 32 位 DIB。
    unsafe {
        let info = dib_info(size);
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let bitmap = CreateDIBSection(
            std::ptr::null_mut(),
            &info,
            DIB_RGB_COLORS,
            &raw mut bits,
            std::ptr::null_mut(),
            0,
        );
        if !bitmap.is_null() && !bits.is_null() {
            std::slice::from_raw_parts_mut(bits.cast::<u8>(), (size * size * 4) as usize).fill(0);
        }
        (!bitmap.is_null()).then_some(bitmap)
    }
}

fn dib_info(size: i32) -> BITMAPINFO {
    let mut info: BITMAPINFO = unsafe { std::mem::zeroed() };
    info.bmiHeader = BITMAPINFOHEADER {
        biSize: u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>()).unwrap_or(40),
        biWidth: size,
        // 负高度表示 top-down。
        biHeight: -size,
        biPlanes: 1,
        biBitCount: 32,
        ..unsafe { std::mem::zeroed() }
    };
    info
}

/// 把一个字形渲染成透明背景的 BGRA 位图。
#[allow(clippy::too_many_lines)]
fn render_glyph(codepoint: u32, size: i32, color: u32) -> Option<HBITMAP> {
    let text = [u16::try_from(codepoint).ok()?];
    let face = crate::text::wide(FONT_FACE);
    let mut logfont: LOGFONTW = unsafe { std::mem::zeroed() };
    logfont.lfHeight = -size;
    logfont.lfWeight = FW_NORMAL;
    logfont.lfCharSet = DEFAULT_CHARSET;
    logfont.lfQuality = ANTIALIASED_QUALITY;
    let face_len = face.len().min(logfont.lfFaceName.len());
    logfont.lfFaceName[..face_len].copy_from_slice(&face[..face_len]);

    // SAFETY: 所有 GDI 句柄都在本函数内成对释放。
    unsafe {
        let info = dib_info(size);
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let bitmap = CreateDIBSection(
            std::ptr::null_mut(),
            &info,
            DIB_RGB_COLORS,
            &raw mut bits,
            std::ptr::null_mut(),
            0,
        );
        if bitmap.is_null() || bits.is_null() {
            return None;
        }
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        if dc.is_null() {
            DeleteObject(bitmap);
            return None;
        }
        let old_bitmap = SelectObject(dc, bitmap);
        std::slice::from_raw_parts_mut(bits.cast::<u8>(), (size * size * 4) as usize).fill(0);

        let font = CreateFontIndirectW(&logfont);
        let old_font = if font.is_null() {
            std::ptr::null_mut()
        } else {
            SelectObject(dc, font)
        };
        SetBkMode(dc, 1); // TRANSPARENT
        SetTextColor(dc, 0x00FF_FFFF); // 白色，稍后按亮度转成 alpha

        let mut extent: SIZE = std::mem::zeroed();
        GetTextExtentPoint32W(dc, text.as_ptr(), 1, &raw mut extent);
        let x = (size - extent.cx) / 2;
        let y = (size - extent.cy) / 2;
        TextOutW(dc, x, y, text.as_ptr(), 1);

        if !old_font.is_null() {
            SelectObject(dc, old_font);
        }
        if !font.is_null() {
            DeleteObject(font);
        }
        SelectObject(dc, old_bitmap);
        DeleteDC(dc);

        // GDI 文字没有 alpha 通道：把灰度覆盖度转成预乘 alpha。
        let pixels = std::slice::from_raw_parts_mut(bits.cast::<u8>(), (size * size * 4) as usize);
        let red = (color & 0xFF) as u32;
        let green = ((color >> 8) & 0xFF) as u32;
        let blue = ((color >> 16) & 0xFF) as u32;
        for pixel in pixels.chunks_exact_mut(4) {
            let alpha = u32::from(pixel[0].max(pixel[1]).max(pixel[2]));
            pixel[0] = u8::try_from(blue * alpha / 255).unwrap_or(0);
            pixel[1] = u8::try_from(green * alpha / 255).unwrap_or(0);
            pixel[2] = u8::try_from(red * alpha / 255).unwrap_or(0);
            pixel[3] = u8::try_from(alpha).unwrap_or(0);
        }
        Some(bitmap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::Controls::ImageList_Destroy;

    #[test]
    fn loads_tabler_font_and_creates_icons() {
        assert!(ensure_font(), "Tabler Icons 字体应能注册");
        let list = create_toolbar_icons(24, 0x00FF_FFFF);
        assert_ne!(list, 0, "工具栏图像列表应创建成功");
        // SAFETY: 句柄由本测试创建。
        unsafe {
            ImageList_Destroy(list);
        }
    }

    #[test]
    fn icon_indices_are_stable() {
        assert_eq!(Icon::New.index(), 0);
        assert_eq!(Icon::Print.index(), 10);
        for (index, icon) in ALL.iter().enumerate() {
            assert_eq!(icon.index(), index);
        }
    }
}
