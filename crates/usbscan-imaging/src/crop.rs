// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 页面裁剪。

use crate::{validate_bgra, ImagingError};

/// 按像素区域裁剪 BGRA 图像。
///
/// `(left, top, width, height)` 以左上角为原点，`width`/`height` 必须大于 0
/// 且完全落在原图范围内。
pub fn crop_bgra(
    width: u32,
    height: u32,
    bgra: &[u8],
    left: u32,
    top: u32,
    crop_width: u32,
    crop_height: u32,
) -> Result<(u32, u32, Vec<u8>), ImagingError> {
    validate_bgra(width, height, bgra)?;
    if crop_width == 0
        || crop_height == 0
        || left >= width
        || top >= height
        || crop_width > width - left
        || crop_height > height - top
    {
        return Err(ImagingError::Geometry(format!(
            "裁剪区域 {left},{top} {crop_width}×{crop_height} 超出图像 {width}×{height}"
        )));
    }

    let source_width = width as usize;
    let mut out = Vec::with_capacity(crop_width as usize * crop_height as usize * 4);
    for y in top..top + crop_height {
        let start = (y as usize * source_width + left as usize) * 4;
        let end = start + crop_width as usize * 4;
        out.extend_from_slice(&bgra[start..end]);
    }
    Ok((crop_width, crop_height, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(width: u32, height: u32) -> Vec<u8> {
        let mut data = Vec::with_capacity(width as usize * height as usize * 4);
        for y in 0..height {
            for x in 0..width {
                data.extend_from_slice(&[x as u8, y as u8, 7, 255]);
            }
        }
        data
    }

    #[test]
    fn crops_center_region() {
        let data = gradient(8, 6);
        let (width, height, out) = crop_bgra(8, 6, &data, 2, 1, 4, 3).expect("crop");
        assert_eq!((width, height), (4, 3));
        assert_eq!(out.len(), 48);
        assert_eq!(&out[0..4], &[2, 1, 7, 255]);
        assert_eq!(&out[out.len() - 4..], &[5, 3, 7, 255]);
    }

    #[test]
    fn rejects_out_of_bounds() {
        let data = gradient(8, 6);
        assert!(crop_bgra(8, 6, &data, 6, 0, 4, 2).is_err());
        assert!(crop_bgra(8, 6, &data, 0, 5, 2, 2).is_err());
        assert!(crop_bgra(8, 6, &data, 0, 0, 0, 2).is_err());
    }
}
