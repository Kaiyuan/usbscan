// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 原生打印：通过 `PrintDlgW` 选择打印机，把整份会话逐页绘制到打印机 DC。

use std::path::{Path, PathBuf};

use usbscan_imaging as imaging;

use windows_sys::Win32::Foundation::GlobalFree;
use windows_sys::Win32::Graphics::Gdi::{
    DeleteDC, GetDeviceCaps, SetStretchBltMode, StretchDIBits, BITMAPINFO, BITMAPINFOHEADER,
    COLORONCOLOR, DIB_RGB_COLORS, HDC, HORZRES, SRCCOPY, VERTRES,
};
use windows_sys::Win32::Storage::Xps::{AbortDoc, EndDoc, EndPage, StartDocW, StartPage, DOCINFOW};
use windows_sys::Win32::UI::Controls::Dialogs::{
    CommDlgExtendedError, PrintDlgW, PD_NOPAGENUMS, PD_NOSELECTION, PD_RETURNDC, PRINTDLGW,
};

use crate::text::wide;

/// 计算图片在纸张可打印区域内的居中位置。
#[must_use]
pub fn fit_rect(
    image_width: u32,
    image_height: u32,
    page_width: i32,
    page_height: i32,
    margin: i32,
) -> (i32, i32, i32, i32) {
    let page_width = page_width.max(1);
    let page_height = page_height.max(1);
    let margin = margin.max(0);
    let max_width = (page_width - margin * 2).max(1);
    let max_height = (page_height - margin * 2).max(1);
    let image_width = f64::from(image_width.max(1));
    let image_height = f64::from(image_height.max(1));
    let scale = (f64::from(max_width) / image_width).min(f64::from(max_height) / image_height);
    let width = (image_width * scale).round().max(1.0) as i32;
    let height = (image_height * scale).round().max(1.0) as i32;
    (
        (page_width - width) / 2,
        (page_height - height) / 2,
        width,
        height,
    )
}

/// 打印一组 JPEG 页面；用户取消打印对话框时返回 `Ok(false)`。
pub fn print_files(
    parent: windows_sys::Win32::Foundation::HWND,
    files: &[PathBuf],
) -> Result<bool, String> {
    if files.is_empty() {
        return Err("没有可打印的页面".to_string());
    }
    // SAFETY: PRINTDLGW 全零是合法 POD。
    let mut dialog: PRINTDLGW = unsafe { std::mem::zeroed() };
    dialog.lStructSize = u32::try_from(std::mem::size_of::<PRINTDLGW>()).unwrap_or(0);
    dialog.hwndOwner = parent;
    dialog.Flags = PD_RETURNDC | PD_NOSELECTION | PD_NOPAGENUMS;
    dialog.nCopies = 1;
    // SAFETY: 标准打印对话框。
    let ok = unsafe { PrintDlgW(&mut dialog) };
    if ok == 0 {
        let error = unsafe { CommDlgExtendedError() };
        if error == 0 {
            return Ok(false);
        }
        return Err(format!("打印对话框失败：0x{error:08X}"));
    }
    let hdc = dialog.hDC;
    if hdc.is_null() {
        return Err("打印对话框没有返回打印机 DC".to_string());
    }
    let result = print_to_dc(hdc, files);
    // SAFETY: 释放系统分配的 DC 与 DEVMODE/DEVNAMES。
    unsafe {
        DeleteDC(hdc);
        if !dialog.hDevMode.is_null() {
            GlobalFree(dialog.hDevMode);
        }
        if !dialog.hDevNames.is_null() {
            GlobalFree(dialog.hDevNames);
        }
    }
    result.map(|()| true)
}

fn print_to_dc(hdc: HDC, files: &[PathBuf]) -> Result<(), String> {
    let title = wide("usbscan 文档");
    // SAFETY: DOCINFOW 全零是合法 POD，标题字符串在调用期间有效。
    let mut docinfo: DOCINFOW = unsafe { std::mem::zeroed() };
    docinfo.cbSize = i32::try_from(std::mem::size_of::<DOCINFOW>()).unwrap_or(0);
    docinfo.lpszDocName = title.as_ptr();
    // SAFETY: 标准打印作业流程。
    let started = unsafe { StartDocW(hdc, &docinfo) };
    if started <= 0 {
        return Err("无法启动打印作业".to_string());
    }
    let page_width = unsafe { GetDeviceCaps(hdc, HORZRES as i32) };
    let page_height = unsafe { GetDeviceCaps(hdc, VERTRES as i32) };
    let result = (|| {
        for file in files {
            let (width, height, data) = imaging::load_bgra(file)
                .map_err(|err| format!("读取 {} 失败：{err}", file.display()))?;
            let (x, y, draw_width, draw_height) =
                fit_rect(width, height, page_width, page_height, 30);
            // SAFETY: 打印机 DC 与 BGRA 缓冲区有效。
            unsafe {
                if StartPage(hdc) <= 0 {
                    return Err("无法开始新页".to_string());
                }
                let mut info: BITMAPINFO = std::mem::zeroed();
                info.bmiHeader = BITMAPINFOHEADER {
                    biSize: u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>()).unwrap_or(40),
                    biWidth: i32::try_from(width).unwrap_or(1),
                    biHeight: -i32::try_from(height).unwrap_or(1),
                    biPlanes: 1,
                    biBitCount: 32,
                    ..std::mem::zeroed()
                };
                SetStretchBltMode(hdc, COLORONCOLOR);
                StretchDIBits(
                    hdc,
                    x,
                    y,
                    draw_width,
                    draw_height,
                    0,
                    0,
                    i32::try_from(width).unwrap_or(1),
                    i32::try_from(height).unwrap_or(1),
                    data.as_ptr().cast(),
                    &info,
                    DIB_RGB_COLORS,
                    SRCCOPY,
                );
                if EndPage(hdc) <= 0 {
                    return Err("结束打印页失败".to_string());
                }
            }
        }
        // SAFETY: 与 StartDocW 成对调用。
        unsafe {
            if EndDoc(hdc) <= 0 {
                return Err("结束打印作业失败".to_string());
            }
        }
        Ok(())
    })();
    if result.is_err() {
        // SAFETY: 取消未完成的打印作业。
        unsafe {
            AbortDoc(hdc as _);
        }
    }
    result
}

/// 把页面路径转换为可打印列表。
#[must_use]
pub fn page_paths(root: &Path, pages: &[impl AsRef<Path>]) -> Vec<PathBuf> {
    pages.iter().map(|page| root.join(page.as_ref())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_rect_centers_and_preserves_aspect() {
        let (x, y, width, height) = fit_rect(100, 50, 1000, 800, 50);
        assert_eq!((width, height), (900, 450));
        assert_eq!((x, y), (50, 175));
    }

    #[test]
    fn fit_rect_handles_tall_image() {
        let (_, _, width, height) = fit_rect(50, 100, 800, 800, 50);
        assert_eq!((width, height), (350, 700));
    }
}
