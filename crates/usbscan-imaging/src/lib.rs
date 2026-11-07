// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 图像编解码。
//!
//! 目前只提供 JPEG 编码；采集后端输出 BGRA，
//! 这里负责在落盘前转成 JPEG 需要的 RGB。

use std::io::{self, Write};
use std::path::Path;

use image::codecs::jpeg::JpegEncoder;
use image::{ExtendedColorType, ImageEncoder};

mod enhance;
mod perspective;
mod transform;

pub use enhance::{enhance, EnhanceOptions, EnhancedImage};
pub use perspective::{auto_perspective, detect_document_corners, perspective_from_corners};
pub use transform::{mirror_horizontal_bgra, rotate90_bgra};

/// 图像处理错误。
#[derive(Debug)]
pub enum ImagingError {
    /// 像素缓冲区尺寸与宽高不匹配。
    BufferSize { width: u32, height: u32, len: usize },
    /// 宽高或缩略图上限为 0。
    InvalidSize {
        width: u32,
        height: u32,
        max_width: u32,
        max_height: u32,
    },
    /// 几何变换失败。
    Geometry(String),
    /// 编码或写入失败。
    Jpeg(image::ImageError),
    /// 输出目标写入失败。
    Io(io::Error),
}

impl std::fmt::Display for ImagingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BufferSize { width, height, len } => {
                write!(f, "像素缓冲区大小不符：{width}×{height}，实际 {len} 字节")
            }
            Self::InvalidSize {
                width,
                height,
                max_width,
                max_height,
            } => write!(
                f,
                "图像尺寸无效：{width}×{height}，缩略图上限 {max_width}×{max_height}"
            ),
            Self::Geometry(message) => write!(f, "几何变换失败：{message}"),
            Self::Jpeg(err) => write!(f, "图像编解码失败：{err}"),
            Self::Io(err) => write!(f, "写入失败：{err}"),
        }
    }
}

impl std::error::Error for ImagingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Jpeg(err) => Some(err),
            Self::Io(err) => Some(err),
            Self::BufferSize { .. } | Self::InvalidSize { .. } | Self::Geometry(_) => None,
        }
    }
}

impl From<image::ImageError> for ImagingError {
    fn from(err: image::ImageError) -> Self {
        Self::Jpeg(err)
    }
}

impl From<io::Error> for ImagingError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

/// 把 top-down 的 BGRA8 像素编码为 JPEG。
///
/// `quality` 取值 1–100，推荐 92（文档扫描件在清晰度与体积之间折中）。
pub fn encode_jpeg_bgra<W: Write>(
    width: u32,
    height: u32,
    bgra: &[u8],
    quality: u8,
    out: W,
) -> Result<(), ImagingError> {
    let expected = width as usize * height as usize * 4;
    if bgra.len() < expected {
        return Err(ImagingError::BufferSize {
            width,
            height,
            len: bgra.len(),
        });
    }

    let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
    for pixel in bgra[..expected].chunks_exact(4) {
        // BGRA -> RGB
        rgb.push(pixel[2]);
        rgb.push(pixel[1]);
        rgb.push(pixel[0]);
    }

    let encoder = JpegEncoder::new_with_quality(out, quality.clamp(1, 100));
    encoder.write_image(&rgb, width, height, ExtendedColorType::Rgb8)?;
    Ok(())
}

/// 把 top-down BGRA8 转成 RGB24（预览之外的导出用）。
#[must_use]
pub fn bgra_to_rgb(width: u32, height: u32, bgra: &[u8]) -> Vec<u8> {
    let pixels = width as usize * height as usize;
    let mut rgb = Vec::with_capacity(pixels * 3);
    for pixel in bgra.chunks_exact(4).take(pixels) {
        rgb.push(pixel[2]);
        rgb.push(pixel[1]);
        rgb.push(pixel[0]);
    }
    rgb
}

/// 把一帧 BGRA 缩略到不超过 `max_width × max_height`，保持宽高比。
///
/// 缩略图只用于界面列表，采用最近邻采样，避免为数十张图片引入
/// 重量级缩放依赖。返回 `(宽, 高, BGRA 数据)`。
pub fn thumbnail_bgra(
    width: u32,
    height: u32,
    bgra: &[u8],
    max_width: u32,
    max_height: u32,
) -> Result<(u32, u32, Vec<u8>), ImagingError> {
    let expected = width as usize * height as usize * 4;
    if width == 0 || height == 0 || max_width == 0 || max_height == 0 {
        return Err(ImagingError::InvalidSize {
            width,
            height,
            max_width,
            max_height,
        });
    }
    if bgra.len() < expected {
        return Err(ImagingError::BufferSize {
            width,
            height,
            len: bgra.len(),
        });
    }

    // 用整数比较宽高比，避免浮点误差导致缩略图越界。
    let scale_width = u64::from(max_width) * u64::from(height);
    let scale_height = u64::from(max_height) * u64::from(width);
    let (target_width, target_height) = if scale_width <= scale_height {
        let target_width = max_width.min(width);
        let target_height =
            (u64::from(height) * u64::from(target_width) / u64::from(width)).max(1) as u32;
        (target_width, target_height)
    } else {
        let target_height = max_height.min(height);
        let target_width =
            (u64::from(width) * u64::from(target_height) / u64::from(height)).max(1) as u32;
        (target_width, target_height)
    };

    let mut out = Vec::with_capacity(target_width as usize * target_height as usize * 4);
    for y in 0..target_height {
        let src_y = (u64::from(y) * u64::from(height) / u64::from(target_height)) as usize;
        let row = src_y * width as usize;
        for x in 0..target_width {
            let src_x = (u64::from(x) * u64::from(width) / u64::from(target_width)) as usize;
            let offset = (row + src_x) * 4;
            out.extend_from_slice(&bgra[offset..offset + 4]);
        }
    }
    Ok((target_width, target_height, out))
}

/// 读取图片为 top-down BGRA8。
pub fn load_bgra(path: &Path) -> Result<(u32, u32, Vec<u8>), ImagingError> {
    let image = image::open(path)?.to_rgba8();
    let (width, height) = image.dimensions();
    let rgba = image.into_raw();
    let mut bgra = Vec::with_capacity(rgba.len());
    for pixel in rgba.chunks_exact(4) {
        bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
    Ok((width, height, bgra))
}

/// 把任意受支持的图片转码为 JPEG，返回原始像素尺寸。
pub fn transcode_to_jpeg<W: Write>(
    source: &Path,
    quality: u8,
    out: W,
) -> Result<(u32, u32), ImagingError> {
    let image = image::open(source)?.to_rgb8();
    let (width, height) = image.dimensions();
    let encoder = JpegEncoder::new_with_quality(out, quality.clamp(1, 100));
    encoder.write_image(image.as_raw(), width, height, ExtendedColorType::Rgb8)?;
    Ok((width, height))
}

/// 读取磁盘图片的宽高，不完整解码像素。
pub fn image_dimensions(path: &Path) -> Result<(u32, u32), ImagingError> {
    let dimensions = image::image_dimensions(path)?;
    Ok(dimensions)
}

/// 从磁盘上的图片生成 BGRA 缩略图。
pub fn image_thumbnail_bgra(
    path: &Path,
    max_width: u32,
    max_height: u32,
) -> Result<(u32, u32, Vec<u8>), ImagingError> {
    let image = image::open(path)?.to_rgba8();
    let (width, height) = image.dimensions();
    let rgba = image.into_raw();
    let mut bgra = Vec::with_capacity(rgba.len());
    for pixel in rgba.chunks_exact(4) {
        bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
    thumbnail_bgra(width, height, &bgra, max_width, max_height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_small_jpeg() {
        let bgra = vec![0u8; 2 * 2 * 4];
        let mut out = Vec::new();
        encode_jpeg_bgra(2, 2, &bgra, 90, &mut out).expect("encode");
        assert!(out.starts_with(&[0xFF, 0xD8]), "JPEG SOI 标记");
        assert!(out.ends_with(&[0xFF, 0xD9]), "JPEG EOI 标记");
    }

    #[test]
    fn rejects_short_buffer() {
        let err = encode_jpeg_bgra(4, 4, &[0; 8], 90, Vec::new());
        assert!(matches!(err, Err(ImagingError::BufferSize { .. })));
    }

    #[test]
    fn converts_channel_order() {
        let bgra = [10, 20, 30, 255, 1, 2, 3, 255];
        assert_eq!(bgra_to_rgb(2, 1, &bgra), vec![30, 20, 10, 3, 2, 1]);
    }

    #[test]
    fn thumbnails_keep_aspect_ratio() {
        let bgra = vec![0x40u8; 100 * 50 * 4];
        let (width, height, data) = thumbnail_bgra(100, 50, &bgra, 40, 40).expect("thumbnail");
        assert_eq!((width, height), (40, 20));
        assert_eq!(data.len(), 40 * 20 * 4);

        // 小图不会被放大。
        let (width, height, _) =
            thumbnail_bgra(10, 10, &vec![0; 10 * 10 * 4], 40, 40).expect("small");
        assert_eq!((width, height), (10, 10));
    }

    #[test]
    fn rejects_empty_sizes() {
        let error = thumbnail_bgra(0, 10, &[], 40, 40);
        assert!(matches!(error, Err(ImagingError::InvalidSize { .. })));
    }

    #[test]
    fn decodes_thumbnail_from_jpeg() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-tmp");
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join(format!("imaging-thumb-{}.jpg", std::process::id()));

        let bgra = vec![255u8; 64 * 32 * 4];
        let mut jpeg = Vec::new();
        encode_jpeg_bgra(64, 32, &bgra, 90, &mut jpeg).expect("encode");
        std::fs::write(&path, &jpeg).expect("write");

        let (width, height, data) = image_thumbnail_bgra(&path, 24, 24).expect("thumbnail");
        assert_eq!((width, height), (24, 12));
        assert_eq!(data.len(), 24 * 12 * 4);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn releases_file_after_thumbnail() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-tmp");
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join(format!("imaging-release-{}.jpg", std::process::id()));

        let bgra = vec![255u8; 32 * 32 * 4];
        let mut jpeg = Vec::new();
        encode_jpeg_bgra(32, 32, &bgra, 90, &mut jpeg).expect("encode");
        std::fs::write(&path, &jpeg).expect("write");

        let _ = image_thumbnail_bgra(&path, 16, 16).expect("thumbnail");
        std::fs::remove_file(&path).expect("file must be closed after decoding");
    }
}
