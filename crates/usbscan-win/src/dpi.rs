// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! DPI 感知与尺寸缩放。

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};

/// 设计基准 DPI。
pub const BASE_DPI: u32 = 96;

/// 查询窗口当前所在显示器的 DPI。
///
/// 窗口无效时退回到系统 DPI，再失败则返回 96。
#[must_use]
pub fn dpi_for_window(hwnd: HWND) -> u32 {
    // SAFETY: hwnd 由调用方保证有效（或为 null，API 会返回 0）。
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi != 0 {
        return dpi;
    }
    let sys = unsafe { GetDpiForSystem() };
    if sys != 0 {
        sys
    } else {
        BASE_DPI
    }
}

/// 把 96 DPI 下的逻辑尺寸换算成实际像素。
///
/// 使用整数运算并四舍五入，避免浮点误差导致相邻控件出现 1px 缝隙。
#[must_use]
pub fn scale(value: i32, dpi: u32) -> i32 {
    let scaled = i64::from(value) * i64::from(dpi) + i64::from(BASE_DPI) / 2;
    (scaled / i64::from(BASE_DPI)) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_at_base_dpi_is_identity() {
        assert_eq!(scale(100, 96), 100);
    }

    #[test]
    fn scale_rounds() {
        assert_eq!(scale(10, 120), 13); // 12.5 -> 13
        assert_eq!(scale(10, 144), 15);
        assert_eq!(scale(180, 120), 225);
    }
}
