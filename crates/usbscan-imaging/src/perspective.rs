// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 自动透视校正。
//!
//! 第一版采用“内容极值点”估计文档四角：上、右、下、左最远点构成四边形，
//! 再通过单应性变换把它映射成矩形。对倾斜拍摄的纸张效果较好；页面被裁掉
//! 或背景与纸张颜色接近时会自动跳过。

#![allow(
    clippy::cast_lossless,
    clippy::cast_precision_loss,
    clippy::needless_range_loop,
    clippy::too_many_lines
)]

use crate::ImagingError;

/// 尝试自动透视校正；检测不到可信四边形时返回 `None`。
pub fn auto_perspective(
    width: u32,
    height: u32,
    bgra: &[u8],
) -> Result<Option<(u32, u32, Vec<u8>)>, ImagingError> {
    crate::validate_bgra(width, height, bgra)?;
    let Some(corners) = crate::detect_document_corners(width, height, bgra) else {
        return Ok(None);
    };
    perspective_from_corners(width, height, bgra, &corners).map(Some)
}

/// 用指定四角点执行透视校正。角点顺序为左上、右上、右下、左下。
pub fn perspective_from_corners(
    width: u32,
    height: u32,
    bgra: &[u8],
    corners: &[(f64, f64); 4],
) -> Result<(u32, u32, Vec<u8>), ImagingError> {
    crate::validate_bgra(width, height, bgra)?;
    let target_width = distance(corners[0], corners[1])
        .max(distance(corners[3], corners[2]))
        .round()
        .max(16.0) as u32;
    let target_height = distance(corners[0], corners[3])
        .max(distance(corners[1], corners[2]))
        .round()
        .max(16.0) as u32;
    if target_width > width * 4 || target_height > height * 4 {
        return Err(ImagingError::InvalidSize {
            width: target_width,
            height: target_height,
            max_width: width * 4,
            max_height: height * 4,
        });
    }

    let destination = [
        (0.0, 0.0),
        (f64::from(target_width - 1), 0.0),
        (f64::from(target_width - 1), f64::from(target_height - 1)),
        (0.0, f64::from(target_height - 1)),
    ];
    let homography = solve_homography(&destination, corners)
        .ok_or_else(|| ImagingError::Geometry("透视四角点退化，无法求解".to_string()))?;

    let mut out = vec![255u8; target_width as usize * target_height as usize * 4];
    let source_width = width as usize;
    for y in 0..target_height {
        for x in 0..target_width {
            let (u, v) = apply_homography(&homography, f64::from(x), f64::from(y));
            if u < 0.0 || v < 0.0 || u >= width as f64 - 1.0 || v >= height as f64 - 1.0 {
                continue;
            }
            let x0 = u.floor() as usize;
            let y0 = v.floor() as usize;
            let fx = u - x0 as f64;
            let fy = v - y0 as f64;
            let destination_offset = (y as usize * target_width as usize + x as usize) * 4;
            for channel in 0..4 {
                let p00 = f64::from(bgra[(y0 * source_width + x0) * 4 + channel]);
                let p10 = f64::from(bgra[(y0 * source_width + x0 + 1) * 4 + channel]);
                let p01 = f64::from(bgra[((y0 + 1) * source_width + x0) * 4 + channel]);
                let p11 = f64::from(bgra[((y0 + 1) * source_width + x0 + 1) * 4 + channel]);
                let value = p00 * (1.0 - fx) * (1.0 - fy)
                    + p10 * fx * (1.0 - fy)
                    + p01 * (1.0 - fx) * fy
                    + p11 * fx * fy;
                out[destination_offset + channel] = value.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Ok((target_width, target_height, out))
}

/// 旧版全图极值点检测（保留供回归对照）。
#[allow(dead_code)]
pub fn detect_document_corners_legacy(
    width: u32,
    height: u32,
    bgra: &[u8],
) -> Option<[(f64, f64); 4]> {
    let width_usize = width as usize;
    let height_usize = height as usize;
    if width_usize < 16 || height_usize < 16 {
        return None;
    }

    // 用边框平均颜色估计背景。
    let mut background = [0u32; 3];
    let mut samples = 0u32;
    let step = (width_usize / 128).max(1);
    for x in (0..width_usize).step_by(step) {
        for y in [0, height_usize - 1] {
            let offset = (y * width_usize + x) * 4;
            for channel in 0..3 {
                background[channel] += u32::from(bgra[offset + channel]);
            }
            samples += 1;
        }
    }
    let step = (height_usize / 128).max(1);
    for y in (0..height_usize).step_by(step) {
        for x in [0, width_usize - 1] {
            let offset = (y * width_usize + x) * 4;
            for channel in 0..3 {
                background[channel] += u32::from(bgra[offset + channel]);
            }
            samples += 1;
        }
    }
    if samples == 0 {
        return None;
    }
    let background = [
        background[0] / samples,
        background[1] / samples,
        background[2] / samples,
    ];

    let mut min_sum = (usize::MAX, usize::MAX);
    let mut max_sum = (0usize, 0usize);
    let mut max_diff = (0usize, 0usize);
    let mut min_diff = (usize::MAX, usize::MAX);
    let mut min_sum_value = i64::MAX;
    let mut max_sum_value = i64::MIN;
    let mut max_diff_value = i64::MIN;
    let mut min_diff_value = i64::MAX;
    let mut content = 0usize;
    for y in 0..height_usize {
        for x in 0..width_usize {
            let offset = (y * width_usize + x) * 4;
            let distance = (i32::from(bgra[offset + 2]) - background[0] as i32)
                .abs()
                .max((i32::from(bgra[offset + 1]) - background[1] as i32).abs())
                .max((i32::from(bgra[offset]) - background[2] as i32).abs());
            if distance <= 32 {
                continue;
            }
            content += 1;
            let sum = x as i64 + y as i64;
            let diff = x as i64 - y as i64;
            if sum < min_sum_value {
                min_sum_value = sum;
                min_sum = (x, y);
            }
            if sum > max_sum_value {
                max_sum_value = sum;
                max_sum = (x, y);
            }
            if diff > max_diff_value {
                max_diff_value = diff;
                max_diff = (x, y);
            }
            if diff < min_diff_value {
                min_diff_value = diff;
                min_diff = (x, y);
            }
        }
    }

    let total = width_usize * height_usize;
    if content < total / 10 || content > total * 99 / 100 {
        return None;
    }
    if [min_sum, max_diff, max_sum, min_diff]
        .iter()
        .enumerate()
        .any(|(index, point)| {
            [min_sum, max_diff, max_sum, min_diff]
                .iter()
                .skip(index + 1)
                .any(|other| point == other)
        })
    {
        return None;
    }

    let mut points = [
        (min_sum.0 as f64, min_sum.1 as f64),
        (max_diff.0 as f64, max_diff.1 as f64),
        (max_sum.0 as f64, max_sum.1 as f64),
        (min_diff.0 as f64, min_diff.1 as f64),
    ];
    if signed_area(&points) < 0.0 {
        points.reverse();
    }
    let start = points
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| (a.0 + a.1).total_cmp(&(b.0 + b.1)))
        .map_or(0, |(index, _)| index);
    points.rotate_left(start);

    let area = polygon_area(&points);
    if area < total as f64 * 0.08 || area > total as f64 * 1.2 {
        return None;
    }
    let minimum = f64::from(width.min(height)) * 0.15;
    for index in 0..4 {
        if distance(points[index], points[(index + 1) % 4]) < minimum {
            return None;
        }
    }
    Some(points)
}

fn signed_area(points: &[(f64, f64); 4]) -> f64 {
    let mut sum = 0.0;
    for index in 0..4 {
        let (x1, y1) = points[index];
        let (x2, y2) = points[(index + 1) % 4];
        sum += x1 * y2 - x2 * y1;
    }
    sum / 2.0
}

fn polygon_area(points: &[(f64, f64); 4]) -> f64 {
    let mut sum = 0.0;
    for index in 0..4 {
        let (x1, y1) = points[index];
        let (x2, y2) = points[(index + 1) % 4];
        sum += x1 * y2 - x2 * y1;
    }
    sum.abs() / 2.0
}

fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    let dx = a.0 - b.0;
    let dy = a.1 - b.1;
    (dx * dx + dy * dy).sqrt()
}

/// 解 8x8 线性方程，返回 dst→src 的单应性参数。
fn solve_homography(destination: &[(f64, f64); 4], source: &[(f64, f64); 4]) -> Option<[f64; 8]> {
    let mut matrix = [[0.0f64; 9]; 8];
    for index in 0..4 {
        let (x, y) = destination[index];
        let (u, v) = source[index];
        matrix[index * 2] = [x, y, 1.0, 0.0, 0.0, 0.0, -x * u, -y * u, u];
        matrix[index * 2 + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -x * v, -y * v, v];
    }

    // 高斯消元 + 部分主元。
    for column in 0..8 {
        let mut pivot = column;
        for row in column + 1..8 {
            if matrix[row][column].abs() > matrix[pivot][column].abs() {
                pivot = row;
            }
        }
        if matrix[pivot][column].abs() < 1e-10 {
            return None;
        }
        matrix.swap(column, pivot);
        let divisor = matrix[column][column];
        for value in &mut matrix[column][column..] {
            *value /= divisor;
        }
        for row in 0..8 {
            if row == column {
                continue;
            }
            let factor = matrix[row][column];
            if factor == 0.0 {
                continue;
            }
            for index in column..9 {
                matrix[row][index] -= factor * matrix[column][index];
            }
        }
    }

    let mut result = [0.0f64; 8];
    for (index, value) in result.iter_mut().enumerate() {
        *value = matrix[index][8];
    }
    Some(result)
}

fn apply_homography(h: &[f64; 8], x: f64, y: f64) -> (f64, f64) {
    let denominator = h[6] * x + h[7] * y + 1.0;
    if denominator.abs() < 1e-10 {
        return (-1.0, -1.0);
    }
    (
        (h[0] * x + h[1] * y + h[2]) / denominator,
        (h[3] * x + h[4] * y + h[5]) / denominator,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn white_image(width: u32, height: u32) -> Vec<u8> {
        vec![255u8; width as usize * height as usize * 4]
    }

    #[test]
    fn homography_maps_corners() {
        let destination = [(0.0, 0.0), (99.0, 0.0), (99.0, 49.0), (0.0, 49.0)];
        let source = [(10.0, 5.0), (110.0, 8.0), (108.0, 60.0), (12.0, 58.0)];
        let h = solve_homography(&destination, &source).expect("homography");
        for index in 0..4 {
            let (u, v) = apply_homography(&h, destination[index].0, destination[index].1);
            assert!((u - source[index].0).abs() < 1e-6);
            assert!((v - source[index].1).abs() < 1e-6);
        }
    }

    #[test]
    fn detects_and_corrects_trapezoid() {
        let width = 160u32;
        let height = 120u32;
        let mut data = vec![20u8; width as usize * height as usize * 4];
        for pixel in data.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
        // 一个梯形页面：上边比下边窄。
        let top_left = (50.0f64, 20.0f64);
        let top_right = (110.0f64, 20.0f64);
        let bottom_right = (140.0f64, 100.0f64);
        let bottom_left = (20.0f64, 100.0f64);
        let edges = [
            (top_left, top_right),
            (top_right, bottom_right),
            (bottom_right, bottom_left),
            (bottom_left, top_left),
        ];
        for y in 0..height as usize {
            for x in 0..width as usize {
                let point = (x as f64, y as f64);
                if inside_polygon(point, &edges) {
                    let offset = (y * width as usize + x) * 4;
                    data[offset..offset + 4].copy_from_slice(&[255, 255, 255, 255]);
                }
            }
        }
        let (out_width, out_height, out) = auto_perspective(width, height, &data)
            .expect("perspective")
            .expect("detected");
        assert!(out_width > 60 && out_height > 40);
        let center = ((out_height as usize / 2) * out_width as usize + out_width as usize / 2) * 4;
        assert!(out[center] > 200, "center should be paper");
    }

    #[test]
    fn blank_page_is_skipped() {
        let data = white_image(64, 64);
        assert!(auto_perspective(64, 64, &data)
            .expect("perspective")
            .is_none());
    }

    #[allow(clippy::type_complexity)]
    fn inside_polygon(point: (f64, f64), edges: &[((f64, f64), (f64, f64)); 4]) -> bool {
        let mut inside = true;
        for (a, b) in edges {
            let cross = (b.0 - a.0) * (point.1 - a.1) - (b.1 - a.1) * (point.0 - a.0);
            if cross < 0.0 {
                inside = false;
            }
        }
        inside
    }
}
