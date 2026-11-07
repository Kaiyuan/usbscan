// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 文档图像增强。
//!
//! 这一阶段实现四个基础算子：自动裁边、纠偏、去阴影、白底净化。
//! 所有输入输出均为 top-down BGRA8，处理失败时由调用方决定是否使用原图。

#![allow(
    clippy::cast_precision_loss,
    clippy::similar_names,
    clippy::struct_excessive_bools,
    clippy::needless_range_loop
)]

use crate::ImagingError;

/// 增强选项。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EnhanceOptions {
    pub auto_crop: bool,
    pub perspective: bool,
    pub deskew: bool,
    pub remove_shadow: bool,
    pub white_background: bool,
    /// 综合强度 0–100。
    pub strength: u8,
}

impl EnhanceOptions {
    /// 是否至少启用了一个算子。
    #[must_use]
    pub const fn is_active(self) -> bool {
        self.auto_crop
            || self.perspective
            || self.deskew
            || self.remove_shadow
            || self.white_background
    }
}

/// 增强后的图像。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnhancedImage {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// 执行增强管线。
pub fn enhance(
    width: u32,
    height: u32,
    bgra: &[u8],
    options: EnhanceOptions,
) -> Result<EnhancedImage, ImagingError> {
    let expected = width as usize * height as usize * 4;
    if width == 0 || height == 0 {
        return Err(ImagingError::InvalidSize {
            width,
            height,
            max_width: 0,
            max_height: 0,
        });
    }
    if bgra.len() < expected {
        return Err(ImagingError::BufferSize {
            width,
            height,
            len: bgra.len(),
        });
    }
    let mut image = EnhancedImage {
        width,
        height,
        bgra: bgra[..expected].to_vec(),
    };
    if !options.is_active() {
        return Ok(image);
    }

    let mut perspective_applied = false;
    if options.perspective {
        if let Ok(Some((width, height, bgra))) =
            super::perspective::auto_perspective(image.width, image.height, &image.bgra)
        {
            image = EnhancedImage {
                width,
                height,
                bgra,
            };
            perspective_applied = true;
        }
    }
    if options.deskew && !perspective_applied {
        let corners = crate::detect_document_corners(image.width, image.height, &image.bgra);
        // 开启透视但没检测到文档时，不要对整张桌面做纠偏；否则会旋转整幅画面。
        if !(options.perspective && corners.is_none()) {
            let correction = corners
                .and_then(|corners| document_skew(&corners))
                .or_else(|| estimate_skew(&image));
            if let Some(correction) = correction {
                if correction.abs() >= 0.35 {
                    image = rotate(&image, correction);
                }
            }
        }
    }
    if options.auto_crop && !perspective_applied {
        // 同时开启透视时，检测失败说明当前画面没有可信文档边界；
        // 此时不要退回到背景色裁剪，以免把整张桌面裁成奇怪的比例。
        if !(options.perspective
            && crate::detect_document_corners(image.width, image.height, &image.bgra).is_none())
        {
            image = crop_to_content(&image);
        }
    }
    if options.remove_shadow && options.strength > 0 {
        remove_shadow(&mut image, options.strength);
    }
    if options.white_background && options.strength > 0 {
        normalize_levels(&mut image, options.strength);
    }
    Ok(image)
}

fn pixel(image: &EnhancedImage, x: usize, y: usize) -> &[u8] {
    let offset = (y * image.width as usize + x) * 4;
    &image.bgra[offset..offset + 4]
}

fn luminance(pixel: &[u8]) -> u32 {
    (u32::from(pixel[2]) * 299 + u32::from(pixel[1]) * 587 + u32::from(pixel[0]) * 114) / 1000
}

#[allow(clippy::too_many_lines)]
fn crop_to_content(image: &EnhancedImage) -> EnhancedImage {
    let width = image.width as usize;
    let height = image.height as usize;
    if width < 8 || height < 8 {
        return image.clone();
    }

    // 优先使用连通域检测出的文档边界；检测失败时回退到背景色裁剪。
    if let Some(corners) = crate::detect_document_corners(image.width, image.height, &image.bgra) {
        let min_x = corners
            .iter()
            .map(|point| point.0)
            .fold(f64::MAX, f64::min)
            .floor()
            .clamp(0.0, width as f64 - 1.0) as usize;
        let min_y = corners
            .iter()
            .map(|point| point.1)
            .fold(f64::MAX, f64::min)
            .floor()
            .clamp(0.0, height as f64 - 1.0) as usize;
        let max_x = corners
            .iter()
            .map(|point| point.0)
            .fold(f64::MIN, f64::max)
            .ceil()
            .clamp(0.0, width as f64 - 1.0) as usize;
        let max_y = corners
            .iter()
            .map(|point| point.1)
            .fold(f64::MIN, f64::max)
            .ceil()
            .clamp(0.0, height as f64 - 1.0) as usize;
        let margin = 2;
        let left = min_x.saturating_sub(margin);
        let top = min_y.saturating_sub(margin);
        let right = (max_x + margin).min(width - 1);
        let bottom = (max_y + margin).min(height - 1);
        let crop_width = right - left + 1;
        let crop_height = bottom - top + 1;
        if crop_width >= 16
            && crop_height >= 16
            && (crop_width < width * 99 / 100 || crop_height < height * 99 / 100)
        {
            return crop_rect(image, left, top, right, bottom);
        }
    }

    // 用四条边的平均颜色作为背景估计。
    let mut background = [0u32; 3];
    let mut samples = 0u32;
    let step = (width / 128).max(1);
    for x in (0..width).step_by(step) {
        for y in [0, height - 1] {
            let p = pixel(image, x, y);
            background[0] += u32::from(p[0]);
            background[1] += u32::from(p[1]);
            background[2] += u32::from(p[2]);
            samples += 1;
        }
    }
    let step = (height / 128).max(1);
    for y in (0..height).step_by(step) {
        for x in [0, width - 1] {
            let p = pixel(image, x, y);
            background[0] += u32::from(p[0]);
            background[1] += u32::from(p[1]);
            background[2] += u32::from(p[2]);
            samples += 1;
        }
    }
    if samples == 0 {
        return image.clone();
    }
    let background = [
        (background[0] / samples) as i32,
        (background[1] / samples) as i32,
        (background[2] / samples) as i32,
    ];

    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0usize;
    let mut max_y = 0usize;
    let mut content = 0usize;
    for y in 0..height {
        for x in 0..width {
            let p = pixel(image, x, y);
            let distance = (i32::from(p[2]) - background[0])
                .abs()
                .max((i32::from(p[1]) - background[1]).abs())
                .max((i32::from(p[0]) - background[2]).abs());
            if distance > 32 {
                content += 1;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    let total = width * height;
    if content < total / 100 || content > total * 98 / 100 {
        return image.clone();
    }

    let margin = (width.min(height) / 40).max(4);
    let left = min_x.saturating_sub(margin);
    let top = min_y.saturating_sub(margin);
    let right = (max_x + margin).min(width - 1);
    let bottom = (max_y + margin).min(height - 1);
    let crop_width = right - left + 1;
    let crop_height = bottom - top + 1;
    if crop_width < 16 || crop_height < 16 {
        return image.clone();
    }
    crop_rect(image, left, top, right, bottom)
}

fn crop_rect(
    image: &EnhancedImage,
    left: usize,
    top: usize,
    right: usize,
    bottom: usize,
) -> EnhancedImage {
    let width = image.width as usize;
    let crop_width = right - left + 1;
    let crop_height = bottom - top + 1;
    let mut out = vec![0u8; crop_width * crop_height * 4];
    for y in 0..crop_height {
        let src = ((top + y) * width + left) * 4;
        let dst = y * crop_width * 4;
        out[dst..dst + crop_width * 4].copy_from_slice(&image.bgra[src..src + crop_width * 4]);
    }
    EnhancedImage {
        width: crop_width as u32,
        height: crop_height as u32,
        bgra: out,
    }
}

/// 由文档四角估计需要旋转的角度（度）。
///
/// 取上下两条边的平均倾角；透视校正成功后不再调用本函数，避免把已经转正的
/// 纸张按褶皱纹理再次旋转。
fn document_skew(corners: &[(f64, f64); 4]) -> Option<f32> {
    let (top_left, top_right, bottom_right, bottom_left) =
        (corners[0], corners[1], corners[2], corners[3]);
    let top = (top_right.1 - top_left.1).atan2(top_right.0 - top_left.0);
    let bottom = (bottom_right.1 - bottom_left.1).atan2(bottom_right.0 - bottom_left.0);
    let average = f64::midpoint(top, bottom).to_degrees();
    (average.abs() <= 15.0).then_some(-average as f32)
}

fn estimate_skew(image: &EnhancedImage) -> Option<f32> {
    let step = (image.width.max(image.height) / 600).max(1) as usize;
    let width = image.width as usize / step;
    let height = image.height as usize / step;
    if width < 32 || height < 32 {
        return None;
    }

    let mut gray = vec![0u8; width * height];
    for y in 0..height {
        for x in 0..width {
            gray[y * width + x] =
                u8::try_from(luminance(pixel(image, x * step, y * step))).unwrap_or(0);
        }
    }

    // 取梯度较强的点参与投影，权重使用梯度幅值，减少背景弱边缘干扰。
    let mut points = Vec::new();
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let gx = i32::from(gray[y * width + x + 1]) - i32::from(gray[y * width + x - 1]);
            let gy = i32::from(gray[(y + 1) * width + x]) - i32::from(gray[(y - 1) * width + x]);
            let weight = gx.abs() + gy.abs();
            if weight > 50 {
                points.push((x as f32, y as f32, weight as f32));
            }
        }
    }
    if points.len() < 64 {
        return None;
    }

    let cx = width as f32 / 2.0;
    let cy = height as f32 / 2.0;
    let mut bins = vec![0.0f32; height + 2];
    let mut best_angle = 0.0f32;
    let mut best_score = 0.0f32;
    let mut angle = -15.0f32;
    while angle <= 15.0 {
        let score = projection_score(&points, cx, cy, angle, &mut bins);
        if score > best_score {
            best_score = score;
            best_angle = angle;
        }
        angle += 0.25;
    }

    // 在粗搜索结果附近做一次细化。
    let coarse = best_angle;
    let mut angle = coarse - 0.4;
    while angle <= coarse + 0.4 {
        let score = projection_score(&points, cx, cy, angle, &mut bins);
        if score > best_score {
            best_score = score;
            best_angle = angle;
        }
        angle += 0.05;
    }
    // 搜索得到的是把像素转正所需的角度。
    Some(best_angle)
}

fn projection_score(
    points: &[(f32, f32, f32)],
    cx: f32,
    cy: f32,
    angle: f32,
    bins: &mut [f32],
) -> f32 {
    bins.fill(0.0);
    let radians = angle.to_radians();
    let (sin, cos) = radians.sin_cos();
    for &(x, y, weight) in points {
        let dx = x - cx;
        let dy = y - cy;
        let rotated = dx * sin + dy * cos + cy;
        let index = rotated as isize;
        if index >= 0 && (index as usize) < bins.len() {
            bins[index as usize] += weight;
        }
    }
    let mean = bins.iter().sum::<f32>() / bins.len() as f32;
    bins.iter()
        .map(|value| (value - mean) * (value - mean))
        .sum()
}

fn rotate(image: &EnhancedImage, degrees: f32) -> EnhancedImage {
    let radians = degrees.to_radians();
    let (sin, cos) = radians.sin_cos();
    let width = image.width as f32;
    let height = image.height as f32;
    let new_width = (width * cos.abs() + height * sin.abs()).ceil().max(1.0) as u32;
    let new_height = (width * sin.abs() + height * cos.abs()).ceil().max(1.0) as u32;
    let mut out = vec![255u8; new_width as usize * new_height as usize * 4];
    let source_cx = width / 2.0;
    let source_cy = height / 2.0;
    let target_cx = new_width as f32 / 2.0;
    let target_cy = new_height as f32 / 2.0;

    for y in 0..new_height {
        for x in 0..new_width {
            let dx = x as f32 - target_cx;
            let dy = y as f32 - target_cy;
            let sx = dx * cos + dy * sin + source_cx;
            let sy = -dx * sin + dy * cos + source_cy;
            if sx < 0.0 || sy < 0.0 || sx >= width - 1.0 || sy >= height - 1.0 {
                continue;
            }
            let x0 = sx.floor() as usize;
            let y0 = sy.floor() as usize;
            let fx = sx - x0 as f32;
            let fy = sy - y0 as f32;
            let dst = (y as usize * new_width as usize + x as usize) * 4;
            for channel in 0..4 {
                let p00 = f32::from(pixel(image, x0, y0)[channel]);
                let p10 = f32::from(pixel(image, x0 + 1, y0)[channel]);
                let p01 = f32::from(pixel(image, x0, y0 + 1)[channel]);
                let p11 = f32::from(pixel(image, x0 + 1, y0 + 1)[channel]);
                let value = p00 * (1.0 - fx) * (1.0 - fy)
                    + p10 * fx * (1.0 - fy)
                    + p01 * (1.0 - fx) * fy
                    + p11 * fx * fy;
                out[dst + channel] = value.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    EnhancedImage {
        width: new_width,
        height: new_height,
        bgra: out,
    }
}

fn remove_shadow(image: &mut EnhancedImage, strength: u8) {
    let width = image.width as usize;
    let height = image.height as usize;
    let mut integral = vec![0u64; (width + 1) * (height + 1)];
    for y in 0..height {
        let mut row_sum = 0u64;
        for x in 0..width {
            row_sum += u64::from(luminance(pixel(image, x, y)));
            integral[(y + 1) * (width + 1) + x + 1] = integral[y * (width + 1) + x + 1] + row_sum;
        }
    }

    let radius = (width.min(height) / 20).max(8);
    let strength = u32::from(strength.min(100));
    let pixels = image.bgra.as_mut_slice();
    for y in 0..height {
        let top = y.saturating_sub(radius);
        let bottom = (y + radius + 1).min(height);
        for x in 0..width {
            let left = x.saturating_sub(radius);
            let right = (x + radius + 1).min(width);
            let area = (right - left) * (bottom - top);
            if area == 0 {
                continue;
            }
            let sum = integral[bottom * (width + 1) + right] + integral[top * (width + 1) + left]
                - integral[top * (width + 1) + right]
                - integral[bottom * (width + 1) + left];
            let mean = (sum / area as u64).max(1) as u32;
            let factor = (245_u32 * 100 / mean).clamp(100, 320);
            let offset = (y * width + x) * 4;
            for channel in 0..3 {
                let original = u32::from(pixels[offset + channel]);
                let corrected = (original * factor / 100).min(255);
                let blended = original + (corrected - original) * strength / 100;
                pixels[offset + channel] = u8::try_from(blended.min(255)).unwrap_or(255);
            }
        }
    }
}

fn normalize_levels(image: &mut EnhancedImage, strength: u8) {
    let mut histogram = [0u32; 256];
    for pixel in image.bgra.chunks_exact(4) {
        histogram[luminance(pixel) as usize] += 1;
    }
    let total: u32 = histogram.iter().sum();
    if total == 0 {
        return;
    }
    let low_target = total / 50; // 2%
    let high_target = total * 49 / 50;
    let mut cumulative = 0u32;
    let mut low = 0usize;
    let mut high = 255usize;
    for (value, &count) in histogram.iter().enumerate() {
        cumulative += count;
        if cumulative >= low_target {
            low = value;
            break;
        }
    }
    cumulative = 0;
    for (value, &count) in histogram.iter().enumerate() {
        cumulative += count;
        if cumulative >= high_target {
            high = value;
            break;
        }
    }
    if high <= low + 16 {
        return;
    }

    let strength = u32::from(strength.min(100));
    let span = (high - low) as u32;
    for pixel in image.bgra.chunks_exact_mut(4) {
        for channel in 0..3 {
            let original = u32::from(pixel[channel]);
            let stretched = (original.saturating_sub(low as u32) * 255 / span).min(255);
            let blended = original + (stretched - original) * strength / 100;
            pixel[channel] = u8::try_from(blended.min(255)).unwrap_or(255);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: u32, height: u32, bgra: [u8; 4]) -> Vec<u8> {
        let mut data = Vec::with_capacity(width as usize * height as usize * 4);
        for _ in 0..width * height {
            data.extend_from_slice(&bgra);
        }
        data
    }

    #[test]
    fn inactive_options_returns_copy() {
        let data = solid(4, 4, [10, 20, 30, 255]);
        let out = enhance(4, 4, &data, EnhanceOptions::default()).expect("enhance");
        assert_eq!(out.width, 4);
        assert_eq!(out.height, 4);
        assert_eq!(out.bgra, data);
    }

    #[test]
    fn shadow_removal_evens_gradient() {
        let width = 64u32;
        let height = 64u32;
        let mut data = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..height {
            for x in 0..width {
                let value = (80 + x * 140 / (width - 1)) as u8;
                data.extend_from_slice(&[value, value, value, 255]);
            }
        }
        let out = enhance(
            width,
            height,
            &data,
            EnhanceOptions {
                remove_shadow: true,
                strength: 100,
                ..EnhanceOptions::default()
            },
        )
        .expect("enhance");
        let left = luminance(&out.bgra[((height / 2) * width) as usize * 4..][..4]);
        let right = luminance(&out.bgra[(((height / 2) * width + width - 1) as usize * 4)..][..4]);
        assert!(left > 180, "left background should be brightened: {left}");
        assert!(
            (left as i32 - right as i32).abs() < 40,
            "gradient should be reduced: {left} vs {right}"
        );
    }

    #[test]
    fn crop_detects_content() {
        let width = 120u32;
        let height = 100u32;
        let mut data = solid(width, height, [20, 20, 20, 255]);
        for y in 30..70 {
            for x in 20..90 {
                let offset = (y * width + x) as usize * 4;
                data[offset..offset + 4].copy_from_slice(&[240, 240, 240, 255]);
            }
        }
        let out = enhance(
            width,
            height,
            &data,
            EnhanceOptions {
                auto_crop: true,
                ..EnhanceOptions::default()
            },
        )
        .expect("enhance");
        assert!(out.width < width);
        assert!(out.height < height);
        assert!(out.width >= 60);
        assert!(out.height >= 35);
    }

    #[test]
    fn crop_finds_bright_paper_on_wood() {
        let width = 300u32;
        let height = 220u32;
        let mut data = solid(width, height, [120, 95, 60, 255]);
        for y in 40..180 {
            for x in 70..240 {
                let offset = (y * width + x) as usize * 4;
                data[offset..offset + 4].copy_from_slice(&[245, 245, 240, 255]);
            }
        }
        for y in 0..40 {
            for x in 0..60 {
                let offset = (y * width + x) as usize * 4;
                data[offset..offset + 4].copy_from_slice(&[30, 30, 30, 255]);
            }
        }
        let out = enhance(
            width,
            height,
            &data,
            EnhanceOptions {
                auto_crop: true,
                ..EnhanceOptions::default()
            },
        )
        .expect("enhance");
        assert!(out.width >= 165 && out.width < 190, "width {}", out.width);
        assert!(
            out.height >= 135 && out.height < 160,
            "height {}",
            out.height
        );
        let center = ((out.height / 2) * out.width + out.width / 2) as usize * 4;
        assert!(out.bgra[center] > 200, "crop should keep paper");
    }

    #[test]
    fn rotation_expands_canvas() {
        let data = solid(20, 10, [255, 255, 255, 255]);
        let image = EnhancedImage {
            width: 20,
            height: 10,
            bgra: data,
        };
        let rotated = rotate(&image, 90.0);
        assert!((i64::from(rotated.width) - 10).abs() <= 1);
        assert!((i64::from(rotated.height) - 20).abs() <= 1);
    }

    #[test]
    fn deskew_estimates_rotated_content() {
        // 在白色背景上画一条旋转 +6° 的黑色长条。
        let width = 320u32;
        let height = 240u32;
        let mut data = solid(width, height, [255, 255, 255, 255]);
        let radians = 6.0f32.to_radians();
        let (sin, cos) = radians.sin_cos();
        for t in -100..=100 {
            let x = width as f32 / 2.0 + t as f32 * cos;
            let y = height as f32 / 2.0 + t as f32 * sin;
            for dy in -2..=2 {
                for dx in -2..=2 {
                    let px = x as isize + dx;
                    let py = y as isize + dy;
                    if px >= 0 && py >= 0 && (px as u32) < width && (py as u32) < height {
                        let offset = (py as u32 * width + px as u32) as usize * 4;
                        data[offset..offset + 4].copy_from_slice(&[0, 0, 0, 255]);
                    }
                }
            }
        }
        let image = EnhancedImage {
            width,
            height,
            bgra: data,
        };
        let angle = estimate_skew(&image).expect("angle");
        assert!(
            (angle + 6.0).abs() < 2.0,
            "expected correction around -6°, got {angle}"
        );
    }
}
