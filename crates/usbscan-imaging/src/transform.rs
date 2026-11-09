// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 无损的 90° 旋转与水平镜像（BGRA8）。

use crate::ImagingError;

fn validate(width: u32, height: u32, bgra: &[u8]) -> Result<(), ImagingError> {
    if width == 0 || height == 0 {
        return Err(ImagingError::InvalidSize {
            width,
            height,
            max_width: 0,
            max_height: 0,
        });
    }
    let expected = width as usize * height as usize * 4;
    if bgra.len() < expected {
        return Err(ImagingError::BufferSize {
            width,
            height,
            len: bgra.len(),
        });
    }
    Ok(())
}

/// 顺时针或逆时针旋转 90°。
pub fn rotate90_bgra(
    width: u32,
    height: u32,
    bgra: &[u8],
    clockwise: bool,
) -> Result<(u32, u32, Vec<u8>), ImagingError> {
    validate(width, height, bgra)?;
    let source_width = width as usize;
    let source_height = height as usize;
    let target_width = source_height;
    let target_height = source_width;
    let mut out = vec![0u8; target_width * target_height * 4];
    for y in 0..target_height {
        for x in 0..target_width {
            let (source_x, source_y) = if clockwise {
                (y, source_height - 1 - x)
            } else {
                (source_width - 1 - y, x)
            };
            let source = (source_y * source_width + source_x) * 4;
            let target = (y * target_width + x) * 4;
            out[target..target + 4].copy_from_slice(&bgra[source..source + 4]);
        }
    }
    Ok((target_width as u32, target_height as u32, out))
}

/// 按 90° 的整数倍顺时针旋转。
///
/// `degrees` 会被归一到 `0/90/180/270`；其他值按 `0` 处理。
pub fn rotate_bgra(
    width: u32,
    height: u32,
    bgra: &[u8],
    degrees: u32,
) -> Result<(u32, u32, Vec<u8>), ImagingError> {
    validate(width, height, bgra)?;
    let source_width = width as usize;
    let source_height = height as usize;
    let (target_width, target_height) = match degrees % 360 {
        90 | 270 => (source_height, source_width),
        _ => (source_width, source_height),
    };
    if degrees % 360 == 0 {
        return Ok((width, height, bgra.to_vec()));
    }
    let mut out = vec![0u8; target_width * target_height * 4];
    for y in 0..target_height {
        for x in 0..target_width {
            let (source_x, source_y) = match degrees % 360 {
                // 顺时针 90°。
                90 => (y, source_height - 1 - x),
                // 逆时针 90°（顺时针 270°）。
                270 => (source_width - 1 - y, x),
                // 180°。
                _ => (source_width - 1 - x, source_height - 1 - y),
            };
            let source = (source_y * source_width + source_x) * 4;
            let target = (y * target_width + x) * 4;
            out[target..target + 4].copy_from_slice(&bgra[source..source + 4]);
        }
    }
    Ok((target_width as u32, target_height as u32, out))
}

/// 水平镜像（左右翻转）。
pub fn mirror_horizontal_bgra(
    width: u32,
    height: u32,
    bgra: &[u8],
) -> Result<(u32, u32, Vec<u8>), ImagingError> {
    validate(width, height, bgra)?;
    let width_usize = width as usize;
    let height_usize = height as usize;
    let mut out = vec![0u8; width_usize * height_usize * 4];
    for y in 0..height_usize {
        for x in 0..width_usize {
            let source = (y * width_usize + (width_usize - 1 - x)) * 4;
            let target = (y * width_usize + x) * 4;
            out[target..target + 4].copy_from_slice(&bgra[source..source + 4]);
        }
    }
    Ok((width, height, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixels() -> Vec<u8> {
        // 2x2: A B
        //      C D
        vec![1, 1, 1, 255, 2, 2, 2, 255, 3, 3, 3, 255, 4, 4, 4, 255]
    }

    #[test]
    fn rotates_clockwise() {
        let (width, height, data) = rotate90_bgra(2, 2, &pixels(), true).expect("rotate");
        assert_eq!((width, height), (2, 2));
        // 顺时针：C A / D B
        assert_eq!(data[0], 3);
        assert_eq!(data[8], 4);
        assert_eq!(data[4], 1);
        assert_eq!(data[12], 2);
    }

    #[test]
    fn rotates_counter_clockwise() {
        let (_, _, data) = rotate90_bgra(2, 2, &pixels(), false).expect("rotate");
        // 逆时针：B D / A C
        assert_eq!(data[0], 2);
        assert_eq!(data[4], 4);
        assert_eq!(data[8], 1);
        assert_eq!(data[12], 3);
    }

    #[test]
    fn rotates_by_degrees() {
        let (width, height, data) = rotate_bgra(2, 2, &pixels(), 180).expect("rotate");
        assert_eq!((width, height), (2, 2));
        // 180°：D C / B A
        assert_eq!([data[0], data[4], data[8], data[12]], [4, 3, 2, 1]);

        let (_, _, data) = rotate_bgra(2, 2, &pixels(), 270).expect("rotate");
        // 270°：B D / A C
        assert_eq!([data[0], data[4], data[8], data[12]], [2, 4, 1, 3]);

        let (_, _, data) = rotate_bgra(2, 2, &pixels(), 0).expect("rotate");
        assert_eq!([data[0], data[4], data[8], data[12]], [1, 2, 3, 4]);
    }

    #[test]
    fn mirrors_horizontally() {
        let (_, _, data) = mirror_horizontal_bgra(2, 2, &pixels()).expect("mirror");
        // B A / D C
        assert_eq!(data[0], 2);
        assert_eq!(data[4], 1);
        assert_eq!(data[8], 4);
        assert_eq!(data[12], 3);
    }
}
