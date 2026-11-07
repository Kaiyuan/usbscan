// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 图像帧与像素格式。

use crate::error::CaptureError;

/// 采集输出的像素格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// 32 位 BGRA，内存中逐行 top-down。
    Bgra8,
}

/// 一帧图像。
///
/// 采集后端统一输出 top-down、行间无填充的 BGRA8，
/// 这样预览（GDI `StretchDIBits`）与文件编码都可以直接使用。
#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub data: Vec<u8>,
    /// 媒体时间戳（100 纳秒）。
    pub timestamp: i64,
}

impl Frame {
    /// 每行字节数。
    #[must_use]
    pub fn stride(&self) -> usize {
        self.width as usize * 4
    }

    /// 从 Media Foundation 的缓冲区构造，处理行填充与自下而上的布局。
    pub fn from_bgra(
        width: u32,
        height: u32,
        stride: i32,
        data: &[u8],
        timestamp: i64,
    ) -> Result<Self, CaptureError> {
        if width == 0 || height == 0 {
            return Err(CaptureError::BadFrame("尺寸为 0".to_string()));
        }
        let row_bytes = width as usize * 4;
        let stride_abs = stride.unsigned_abs() as usize;
        let needed = (stride_abs * (height as usize - 1)) + row_bytes;
        if stride_abs < row_bytes || data.len() < needed {
            return Err(CaptureError::BadFrame(format!(
                "需要 {needed} 字节，实际 {}",
                data.len()
            )));
        }

        let mut out = Vec::with_capacity(row_bytes * height as usize);
        if stride >= 0 {
            for row in 0..height as usize {
                let start = row * stride_abs;
                out.extend_from_slice(&data[start..start + row_bytes]);
            }
        } else {
            // 负 stride 表示图像自下而上存储。
            for row in (0..height as usize).rev() {
                let start = row * stride_abs;
                out.extend_from_slice(&data[start..start + row_bytes]);
            }
        }

        Ok(Self {
            width,
            height,
            format: PixelFormat::Bgra8,
            data: out,
            timestamp,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_top_down_rows() {
        // 两行，每行 2 像素，带 4 字节行尾填充。
        let mut raw = vec![0u8; 2 * 12];
        for (i, byte) in raw.iter_mut().enumerate() {
            *byte = i as u8;
        }
        let frame = Frame::from_bgra(2, 2, 12, &raw, 0).expect("frame");
        assert_eq!(frame.stride(), 8);
        assert_eq!(&frame.data[0..8], &raw[0..8]);
        assert_eq!(&frame.data[8..16], &raw[12..20]);
    }

    #[test]
    fn flips_bottom_up() {
        let mut raw = vec![0u8; 16];
        for (i, byte) in raw.iter_mut().enumerate() {
            *byte = i as u8;
        }
        let frame = Frame::from_bgra(2, 2, -8, &raw, 0).expect("frame");
        // 第 0 行应来自缓冲区最后 8 字节。
        assert_eq!(&frame.data[0..8], &raw[8..16]);
        assert_eq!(&frame.data[8..16], &raw[0..8]);
    }

    #[test]
    fn rejects_short_buffer() {
        assert!(Frame::from_bgra(4, 4, 16, &[0; 10], 0).is_err());
    }
}
