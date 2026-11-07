// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 页面批注渲染。
//!
//! 批注保存在图片坐标系中，应用时直接叠加到 BGRA 像素上。支持手写笔迹
//! 与矩形框，颜色可带透明度（用于黄色荧光笔效果）。

#![allow(clippy::cast_precision_loss)]

use crate::{validate_bgra, ImagingError};

/// 批注颜色（RGB + 透明度）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnnotationColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    /// 0 完全透明，255 完全不透明。
    pub alpha: u8,
}

impl AnnotationColor {
    /// 构造不透明颜色。
    #[must_use]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self {
            r,
            g,
            b,
            alpha: 255,
        }
    }
}

/// 内置批注颜色。
pub mod colors {
    use super::AnnotationColor;

    pub const RED: AnnotationColor = AnnotationColor::rgb(0xD3, 0x2F, 0x2F);
    pub const BLUE: AnnotationColor = AnnotationColor::rgb(0x15, 0x65, 0xC0);
    pub const BLACK: AnnotationColor = AnnotationColor::rgb(0x10, 0x10, 0x10);
    /// 半透明黄色荧光笔。
    pub const YELLOW: AnnotationColor = AnnotationColor {
        r: 0xFF,
        g: 0xD5,
        b: 0x00,
        alpha: 110,
    };
}

/// 一条批注。
#[derive(Debug, Clone, PartialEq)]
pub enum Annotation {
    /// 手写笔迹。
    Pen {
        color: AnnotationColor,
        /// 线宽（图片像素）。
        width: u32,
        points: Vec<(f64, f64)>,
    },
    /// 矩形框。
    Rect {
        color: AnnotationColor,
        width: u32,
        /// 左上与右下坐标，允许反向。
        rect: (f64, f64, f64, f64),
    },
}

impl Annotation {
    /// 颜色。
    #[must_use]
    pub const fn color(&self) -> AnnotationColor {
        match self {
            Self::Pen { color, .. } | Self::Rect { color, .. } => *color,
        }
    }

    /// 线宽。
    #[must_use]
    pub const fn width(&self) -> u32 {
        match self {
            Self::Pen { width, .. } | Self::Rect { width, .. } => *width,
        }
    }
}

/// 把批注叠加到图像副本上并返回。
pub fn render_annotations(
    width: u32,
    height: u32,
    bgra: &[u8],
    annotations: &[Annotation],
) -> Result<Vec<u8>, ImagingError> {
    validate_bgra(width, height, bgra)?;
    let mut out = bgra.to_vec();
    for annotation in annotations {
        match annotation {
            Annotation::Pen {
                color,
                width: stroke,
                points,
            } => {
                for pair in points.windows(2) {
                    draw_line(&mut out, width, height, pair[0], pair[1], *color, *stroke);
                }
                if points.len() == 1 {
                    let point = points[0];
                    stamp_circle(
                        &mut out,
                        width,
                        height,
                        point.0,
                        point.1,
                        f64::from((*stroke).max(1)) / 2.0,
                        *color,
                    );
                }
            }
            Annotation::Rect {
                color,
                width: stroke,
                rect,
            } => {
                let (x0, y0, x1, y1) = (
                    rect.0.min(rect.2),
                    rect.1.min(rect.3),
                    rect.0.max(rect.2),
                    rect.1.max(rect.3),
                );
                let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
                for index in 0..4 {
                    draw_line(
                        &mut out,
                        width,
                        height,
                        corners[index],
                        corners[(index + 1) % 4],
                        *color,
                        *stroke,
                    );
                }
            }
        }
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn draw_line(
    out: &mut [u8],
    width: u32,
    height: u32,
    from: (f64, f64),
    to: (f64, f64),
    color: AnnotationColor,
    stroke: u32,
) {
    let radius = f64::from(stroke.max(1)) / 2.0;
    let dx = to.0 - from.0;
    let dy = to.1 - from.1;
    let distance = (dx * dx + dy * dy).sqrt();
    if distance < 1e-9 {
        stamp_circle(out, width, height, from.0, from.1, radius, color);
        return;
    }
    let steps = (distance / radius.max(0.75)).ceil().max(1.0) as usize;
    for index in 0..=steps {
        let t = index as f64 / steps as f64;
        let x = from.0 + dx * t;
        let y = from.1 + dy * t;
        stamp_circle(out, width, height, x, y, radius, color);
    }
}

fn stamp_circle(
    out: &mut [u8],
    width: u32,
    height: u32,
    center_x: f64,
    center_y: f64,
    radius: f64,
    color: AnnotationColor,
) {
    let min_x = (center_x - radius).floor().max(0.0) as i64;
    let max_x = (center_x + radius).ceil().min(f64::from(width) - 1.0) as i64;
    let min_y = (center_y - radius).floor().max(0.0) as i64;
    let max_y = (center_y + radius).ceil().min(f64::from(height) - 1.0) as i64;
    let radius_squared = radius * radius;
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let dx = x as f64 + 0.5 - center_x;
            let dy = y as f64 + 0.5 - center_y;
            if dx * dx + dy * dy > radius_squared {
                continue;
            }
            let offset = (y as usize * width as usize + x as usize) * 4;
            blend(&mut out[offset..offset + 4], color);
        }
    }
}

fn blend(pixel: &mut [u8], color: AnnotationColor) {
    let alpha = f64::from(color.alpha) / 255.0;
    let channels = [color.b, color.g, color.r];
    for (index, target) in pixel.iter_mut().take(3).enumerate() {
        *target = (f64::from(channels[index]) * alpha + f64::from(*target) * (1.0 - alpha))
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    pixel[3] = 255;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn white(width: u32, height: u32) -> Vec<u8> {
        vec![255u8; width as usize * height as usize * 4]
    }

    fn pixel(data: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let offset = (y as usize * width as usize + x as usize) * 4;
        [
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]
    }

    #[test]
    fn pen_marks_pixels() {
        let data = white(32, 32);
        let annotation = Annotation::Pen {
            color: colors::RED,
            width: 3,
            points: vec![(5.0, 5.0), (20.0, 20.0)],
        };
        let out = render_annotations(32, 32, &data, &[annotation]).expect("render");
        assert_eq!(pixel(&out, 32, 6, 6), [0x2F, 0x2F, 0xD3, 255]);
        assert_eq!(pixel(&out, 32, 30, 2), [255, 255, 255, 255]);
    }

    #[test]
    fn rectangle_only_draws_border() {
        let data = white(40, 40);
        let annotation = Annotation::Rect {
            color: colors::BLUE,
            width: 2,
            rect: (8.0, 8.0, 30.0, 30.0),
        };
        let out = render_annotations(40, 40, &data, &[annotation]).expect("render");
        assert_eq!(pixel(&out, 40, 8, 8), [0xC0, 0x65, 0x15, 255]);
        assert_eq!(pixel(&out, 40, 20, 20), [255, 255, 255, 255]);
    }

    #[test]
    fn highlighter_blends_with_background() {
        let data = white(16, 16);
        let annotation = Annotation::Pen {
            color: colors::YELLOW,
            width: 4,
            points: vec![(8.0, 8.0), (8.0, 8.0)],
        };
        let out = render_annotations(16, 16, &data, &[annotation]).expect("render");
        let center = pixel(&out, 16, 8, 8);
        assert!(center[0] < 150, "blue {center:?}");
        assert!(center[1] > 200 && center[2] > 250, "{center:?}");
    }

    #[test]
    fn out_of_bounds_is_clipped() {
        let data = white(10, 10);
        let annotation = Annotation::Pen {
            color: colors::BLACK,
            width: 2,
            points: vec![(-50.0, -50.0), (100.0, 100.0)],
        };
        let out = render_annotations(10, 10, &data, &[annotation]).expect("render");
        assert_eq!(out.len(), data.len());
    }

    #[test]
    fn rejects_short_buffer() {
        let error = render_annotations(10, 10, &[0u8; 8], &[]).unwrap_err();
        assert!(matches!(error, ImagingError::BufferSize { .. }));
    }
}
