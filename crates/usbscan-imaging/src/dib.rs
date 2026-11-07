// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! Windows `CF_DIB` 剪贴板位图解码。
//!
//! 支持最常见的 24/32 位未压缩位图；行序既可以是自下而上，也可以是自上而下。

use crate::ImagingError;

/// 解码 `CF_DIB` 数据为自上而下的 BGRA。
pub fn decode_dib(data: &[u8]) -> Result<(u32, u32, Vec<u8>), ImagingError> {
    if data.len() < 40 {
        return Err(ImagingError::Format("DIB 数据过短".to_string()));
    }
    let header_size = read_u32(data, 0)? as usize;
    if header_size < 40 || header_size > data.len() {
        return Err(ImagingError::Format("DIB 头长度无效".to_string()));
    }
    let width = read_i32(data, 4)?;
    let height = read_i32(data, 8)?;
    let bit_count = read_u16(data, 14)?;
    let compression = read_u32(data, 16)?;
    if width <= 0 || height == 0 {
        return Err(ImagingError::Format("DIB 尺寸无效".to_string()));
    }
    let width = width as u32;
    let top_down = height < 0;
    let height = height.unsigned_abs();
    if !matches!(bit_count, 24 | 32) {
        return Err(ImagingError::Format(format!("暂不支持 {bit_count} 位 DIB")));
    }
    // BI_RGB（0）与 BI_BITFIELDS（3）都按 BGRA/BGR 顺序读取。
    if !matches!(compression, 0 | 3) {
        return Err(ImagingError::Format(format!(
            "暂不支持 DIB 压缩格式 {compression}"
        )));
    }

    let bytes_per_pixel = usize::from(bit_count / 8);
    let stride = (width as usize * usize::from(bit_count)).div_ceil(32) * 4;
    let pixel_offset = header_size;
    let required = pixel_offset
        .checked_add(stride * height as usize)
        .ok_or_else(|| ImagingError::Format("DIB 尺寸溢出".to_string()))?;
    if data.len() < required {
        return Err(ImagingError::BufferSize {
            width,
            height,
            len: data.len(),
        });
    }

    let mut out = vec![255u8; width as usize * height as usize * 4];
    for y in 0..height as usize {
        let source_row = if top_down { y } else { height as usize - 1 - y };
        let source_start = pixel_offset + source_row * stride;
        let target_start = y * width as usize * 4;
        for x in 0..width as usize {
            let source = source_start + x * bytes_per_pixel;
            let target = target_start + x * 4;
            out[target] = data[source];
            out[target + 1] = data[source + 1];
            out[target + 2] = data[source + 2];
            out[target + 3] = 255;
        }
    }
    Ok((width, height, out))
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32, ImagingError> {
    data.get(offset..offset + 4)
        .and_then(|slice| slice.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| ImagingError::Format("DIB 字段越界".to_string()))
}

fn read_i32(data: &[u8], offset: usize) -> Result<i32, ImagingError> {
    data.get(offset..offset + 4)
        .and_then(|slice| slice.try_into().ok())
        .map(i32::from_le_bytes)
        .ok_or_else(|| ImagingError::Format("DIB 字段越界".to_string()))
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16, ImagingError> {
    data.get(offset..offset + 2)
        .and_then(|slice| slice.try_into().ok())
        .map(u16::from_le_bytes)
        .ok_or_else(|| ImagingError::Format("DIB 字段越界".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(width: i32, height: i32, bits: u16) -> Vec<u8> {
        let mut data = vec![0u8; 40];
        data[0..4].copy_from_slice(&40u32.to_le_bytes());
        data[4..8].copy_from_slice(&width.to_le_bytes());
        data[8..12].copy_from_slice(&height.to_le_bytes());
        data[14..16].copy_from_slice(&bits.to_le_bytes());
        data
    }

    #[test]
    fn decodes_bottom_up_24_bit() {
        let mut data = header(2, 2, 24);
        // 每行 8 字节：底行红色、顶行蓝色（BGR 顺序）。
        data.extend_from_slice(&[0, 0, 255, 0, 0, 255, 0, 0]);
        data.extend_from_slice(&[255, 0, 0, 255, 0, 0, 0, 0]);
        let (width, height, bgra) = decode_dib(&data).expect("decode");
        assert_eq!((width, height), (2, 2));
        assert_eq!(&bgra[0..4], &[255, 0, 0, 255], "top-left blue");
        assert_eq!(&bgra[8..12], &[0, 0, 255, 255], "bottom-left red");
    }

    #[test]
    fn decodes_top_down_32_bit() {
        let mut data = header(1, -1, 32);
        data.extend_from_slice(&[10, 20, 30, 0]);
        let (width, height, bgra) = decode_dib(&data).expect("decode");
        assert_eq!((width, height), (1, 1));
        assert_eq!(bgra, vec![10, 20, 30, 255]);
    }

    #[test]
    fn rejects_unsupported_depth() {
        let data = header(1, 1, 8);
        assert!(matches!(decode_dib(&data), Err(ImagingError::Format(_))));
    }
}
