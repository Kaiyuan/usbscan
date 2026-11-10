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
/// 超采样倍数。小字号下 GDI 的灰阶抗锯齿不够细腻，先放大渲染再盒式缩小。
const SUPERSAMPLE: i32 = 4;

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

/// 把一个 Tabler 字形渲染成菜单图标位图。
///
/// 调用方负责在菜单销毁后调用 `DeleteObject` 释放返回的位图。
#[must_use]
pub fn menu_bitmap(codepoint: u32, size: i32, color: u32) -> HBITMAP {
    if !ensure_font() {
        return std::ptr::null_mut();
    }
    render_glyph(codepoint, size.clamp(12, 32), color).unwrap_or(std::ptr::null_mut())
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
    let (bitmap, bits) = create_dib(size)?;
    // SAFETY: create_dib 返回的缓冲区是 size×size 的 32 位像素。
    unsafe {
        let len = usize::try_from(size * size * 4).ok()?;
        std::slice::from_raw_parts_mut(bits, len).fill(0);
    }
    Some(bitmap)
}

/// 创建一个全透明的 32 位 top-down DIB，返回位图与像素缓冲区。
fn create_dib(size: i32) -> Option<(HBITMAP, *mut u8)> {
    if size <= 0 {
        return None;
    }
    // SAFETY: 标准 GDI 调用；失败时释放已创建的句柄。
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
        if bitmap.is_null() {
            return None;
        }
        if bits.is_null() {
            DeleteObject(bitmap);
            return None;
        }
        Some((bitmap, bits.cast::<u8>()))
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
///
/// 先在 `SUPERSAMPLE` 倍尺寸上绘制，再做盒式降采样，让小尺寸图标也有平滑边缘。
#[allow(clippy::too_many_lines)]
fn render_glyph(codepoint: u32, size: i32, color: u32) -> Option<HBITMAP> {
    let big = size.checked_mul(SUPERSAMPLE)?;
    let text = [u16::try_from(codepoint).ok()?];
    let face = crate::text::wide(FONT_FACE);
    let mut logfont: LOGFONTW = unsafe { std::mem::zeroed() };
    logfont.lfHeight = -big;
    logfont.lfWeight = FW_NORMAL;
    logfont.lfCharSet = DEFAULT_CHARSET;
    logfont.lfQuality = ANTIALIASED_QUALITY;
    let face_len = face.len().min(logfont.lfFaceName.len());
    logfont.lfFaceName[..face_len].copy_from_slice(&face[..face_len]);

    let (big_bitmap, big_bits) = create_dib(big)?;

    // SAFETY: 所有 GDI 句柄都在本函数内成对释放；像素缓冲区大小由 create_dib 保证。
    let coverage = unsafe {
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        if dc.is_null() {
            DeleteObject(big_bitmap);
            return None;
        }
        let old_bitmap = SelectObject(dc, big_bitmap);
        let big_len = usize::try_from(big * big * 4).ok()?;
        std::slice::from_raw_parts_mut(big_bits, big_len).fill(0);

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
        let x = (big - extent.cx) / 2;
        let y = (big - extent.cy) / 2;
        TextOutW(dc, x, y, text.as_ptr(), 1);

        if !old_font.is_null() {
            SelectObject(dc, old_font);
        }
        if !font.is_null() {
            DeleteObject(font);
        }
        SelectObject(dc, old_bitmap);
        DeleteDC(dc);

        std::slice::from_raw_parts(big_bits, big_len)
    };

    let (bitmap, target_bits) = create_dib(size)?;
    let red = (color & 0xFF) as u32;
    let green = ((color >> 8) & 0xFF) as u32;
    let blue = ((color >> 16) & 0xFF) as u32;
    let sample_count = u32::try_from(SUPERSAMPLE * SUPERSAMPLE).unwrap_or(1);

    // SAFETY: 目标位图与超采样位图都由本函数创建，索引范围在各自缓冲区之内。
    unsafe {
        let target_len = usize::try_from(size * size * 4).ok()?;
        let target = std::slice::from_raw_parts_mut(target_bits, target_len);
        for y in 0..size {
            for x in 0..size {
                let mut sum = 0u32;
                for dy in 0..SUPERSAMPLE {
                    for dx in 0..SUPERSAMPLE {
                        let bx = x * SUPERSAMPLE + dx;
                        let by = y * SUPERSAMPLE + dy;
                        let index = usize::try_from((by * big + bx) * 4).ok()?;
                        let pixel = &coverage[index..index + 4];
                        sum += u32::from(pixel[0].max(pixel[1]).max(pixel[2]));
                    }
                }
                // GDI 文字没有 alpha 通道：把灰度覆盖度转成预乘 alpha。
                let alpha = sum / sample_count;
                let index = usize::try_from((y * size + x) * 4).ok()?;
                target[index] = u8::try_from(blue * alpha / 255).unwrap_or(0);
                target[index + 1] = u8::try_from(green * alpha / 255).unwrap_or(0);
                target[index + 2] = u8::try_from(red * alpha / 255).unwrap_or(0);
                target[index + 3] = u8::try_from(alpha).unwrap_or(0);
            }
        }
        DeleteObject(big_bitmap);
    }
    Some(bitmap)
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
