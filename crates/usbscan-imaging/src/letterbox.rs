// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 去掉部分 UVC 驱动补出的黑色边框。
//!
//! 有些摄像头只支持 16:9 感光元件，却在 4:3 分辨率下返回上下带黑边的画面。
//! 黑边会干扰文档检测，也会被一起拍进扫描件，因此采集后先识别并裁掉。

/// 判定「黑边像素」的亮度上限。
const DARK_LUMA: u32 = 24;
/// 一行/一列中黑像素达到该比例时，视为黑边。
const DARK_RATIO_PERCENT: usize = 99;
/// 每一侧最多裁掉的比例，避免把暗场景误当成黑边。
const MAX_TRIM_PERCENT: u32 = 40;

/// 裁掉四周几乎纯黑的边框。
///
/// 返回 `(x, y, width, height, bgra)`；没有可裁的边框时返回 `None`。
#[must_use]
pub fn trim_letterbox(
    width: u32,
    height: u32,
    bgra: &[u8],
) -> Option<(u32, u32, u32, u32, Vec<u8>)> {
    let w = width as usize;
    let h = height as usize;
    if w < 8 || h < 8 || bgra.len() < w * h * 4 {
        return None;
    }
    let row_dark = |y: usize| -> bool {
        let start = y * w * 4;
        let row = &bgra[start..start + w * 4];
        let mut dark = 0usize;
        for pixel in row.chunks_exact(4) {
            if luma(pixel) < DARK_LUMA {
                dark += 1;
            }
        }
        dark * 100 >= w * DARK_RATIO_PERCENT
    };
    let column_dark = |x: usize| -> bool {
        let mut dark = 0usize;
        for y in 0..h {
            let offset = (y * w + x) * 4;
            if luma(&bgra[offset..offset + 4]) < DARK_LUMA {
                dark += 1;
            }
        }
        dark * 100 >= h * DARK_RATIO_PERCENT
    };

    let max_y = (height * MAX_TRIM_PERCENT / 100) as usize;
    let max_x = (width * MAX_TRIM_PERCENT / 100) as usize;
    let mut top = 0usize;
    while top < max_y && row_dark(top) {
        top += 1;
    }
    let mut bottom = 0usize;
    while bottom < max_y && row_dark(h - 1 - bottom) {
        bottom += 1;
    }
    let mut left = 0usize;
    while left < max_x && column_dark(left) {
        left += 1;
    }
    let mut right = 0usize;
    while right < max_x && column_dark(w - 1 - right) {
        right += 1;
    }

    if top + bottom < 8 && left + right < 8 {
        return None;
    }
    let new_width = width.saturating_sub((left + right) as u32);
    let new_height = height.saturating_sub((top + bottom) as u32);
    if new_width < width / 2 || new_height < height / 2 {
        return None;
    }

    let mut data = Vec::with_capacity(new_width as usize * new_height as usize * 4);
    for y in top..h - bottom {
        let start = (y * w + left) * 4;
        let end = start + new_width as usize * 4;
        data.extend_from_slice(&bgra[start..end]);
    }
    Some((left as u32, top as u32, new_width, new_height, data))
}

fn luma(pixel: &[u8]) -> u32 {
    (u32::from(pixel[2]) * 299 + u32::from(pixel[1]) * 587 + u32::from(pixel[0]) * 114) / 1000
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas(width: usize, height: usize, bgra: [u8; 4]) -> Vec<u8> {
        let mut data = Vec::with_capacity(width * height * 4);
        for _ in 0..width * height {
            data.extend_from_slice(&bgra);
        }
        data
    }

    #[test]
    fn trims_top_and_bottom_bars() {
        let width = 100usize;
        let height = 80usize;
        let mut data = canvas(width, height, [40, 40, 40, 255]);
        for y in 0..12 {
            for x in 0..width {
                let offset = (y * width + x) * 4;
                data[offset..offset + 4].copy_from_slice(&[0, 0, 0, 255]);
            }
        }
        for y in 68..height {
            for x in 0..width {
                let offset = (y * width + x) * 4;
                data[offset..offset + 4].copy_from_slice(&[0, 0, 0, 255]);
            }
        }
        let (x, y, w, h, cropped) =
            trim_letterbox(width as u32, height as u32, &data).expect("trim");
        assert_eq!((x, y, w, h), (0, 12, 100, 56));
        assert_eq!(cropped.len(), 100 * 56 * 4);
        assert_eq!(&cropped[..4], &[40, 40, 40, 255]);
    }

    #[test]
    fn keeps_normal_frames() {
        let data = canvas(64, 48, [120, 120, 120, 255]);
        assert!(trim_letterbox(64, 48, &data).is_none());
    }

    #[test]
    fn refuses_to_crop_most_of_the_frame() {
        let width = 100usize;
        let height = 100usize;
        let mut data = canvas(width, height, [0, 0, 0, 255]);
        for y in 45..55 {
            for x in 0..width {
                let offset = (y * width + x) * 4;
                data[offset..offset + 4].copy_from_slice(&[200, 200, 200, 255]);
            }
        }
        assert!(trim_letterbox(width as u32, height as u32, &data).is_none());
    }
}
