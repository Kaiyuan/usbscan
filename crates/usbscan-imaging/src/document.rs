// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 自动文档边界检测。
//!
//! 先把图像降采样，用边框像素的中位数估计背景色和背景亮度：
//! 1. 背景不是白色时，优先寻找“大面积、明亮、填充率高的连通区域”作为纸张；
//! 2. 否则退回“与背景颜色差异较大”的连通区域；
//! 3. 取选中连通域在原图分辨率下的四个极值点作为文档四角。
//!
//! 这样比直接对全图取极值更能抵抗桌面杂物、线缆和阴影。

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    clippy::trivially_copy_pass_by_ref
)]

struct Component {
    label: u32,
    size: usize,
    min_x: usize,
    min_y: usize,
    max_x: usize,
    max_y: usize,
    fill: f64,
}

/// 检测文档四角；顺序为左上、右上、右下、左下。
pub fn detect_document_corners(width: u32, height: u32, bgra: &[u8]) -> Option<[(f64, f64); 4]> {
    let width = width as usize;
    let height = height as usize;
    if width < 32 || height < 32 || bgra.len() < width * height * 4 {
        return None;
    }
    let step = width.max(height).div_ceil(640).max(1);
    let small_width = (width / step).max(1);
    let small_height = (height / step).max(1);
    if small_width < 24 || small_height < 24 {
        return None;
    }
    let count = small_width * small_height;

    let background = estimate_background(bgra, width, height, small_width, small_height, step)?;
    let background_lum = luminance(background);

    // 亮纸张候选：背景较暗时才启用。
    let bright_mask = if background_lum < 200 {
        let threshold = (background_lum + 45).max(150);
        let mut mask = vec![false; count];
        for y in 0..small_height {
            for x in 0..small_width {
                let offset = ((y * step) * width + x * step) * 4;
                if luminance([bgra[offset + 2], bgra[offset + 1], bgra[offset]]) >= threshold {
                    mask[y * small_width + x] = true;
                }
            }
        }
        Some((mask, threshold))
    } else {
        None
    };

    let (labels, best_label, threshold, use_color) = if let Some((mask, threshold)) = bright_mask {
        let (labels, components) = connected_components(&mask, small_width, small_height);
        if let Some(component) = choose_component(&components, small_width, small_height, true) {
            (labels, component.label, Some(threshold), false)
        } else {
            let (labels, label) =
                color_fallback(bgra, width, step, small_width, small_height, &background)?;
            (labels, label, None, true)
        }
    } else {
        let (labels, label) =
            color_fallback(bgra, width, step, small_width, small_height, &background)?;
        (labels, label, None, true)
    };

    let corners = full_resolution_corners(
        bgra,
        width,
        height,
        small_width,
        step,
        &labels,
        best_label,
        threshold,
        use_color,
        &background,
    )?;
    if !valid_quad(&corners, width, height) {
        return None;
    }
    Some(corners)
}

fn estimate_background(
    bgra: &[u8],
    width: usize,
    height: usize,
    small_width: usize,
    small_height: usize,
    step: usize,
) -> Option<[u8; 3]> {
    let mut channels: [Vec<u8>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut push = |x: usize, y: usize| {
        let offset = (y * width + x) * 4;
        channels[0].push(bgra[offset + 2]);
        channels[1].push(bgra[offset + 1]);
        channels[2].push(bgra[offset]);
    };
    for y in [0, (small_height - 1) * step] {
        for x in (0..width).step_by(step) {
            push(x, y);
        }
    }
    for x in [0, (small_width - 1) * step] {
        for y in (0..height).step_by(step) {
            push(x, y);
        }
    }
    if channels[0].is_empty() {
        return None;
    }
    Some([
        median(&mut channels[0]),
        median(&mut channels[1]),
        median(&mut channels[2]),
    ])
}

fn color_fallback(
    bgra: &[u8],
    width: usize,
    step: usize,
    small_width: usize,
    small_height: usize,
    background: &[u8; 3],
) -> Option<(Vec<u32>, u32)> {
    let count = small_width * small_height;
    let mut mask = vec![false; count];
    for y in 0..small_height {
        for x in 0..small_width {
            if distance_at(bgra, width, x * step, y * step, background) > 30 {
                mask[y * small_width + x] = true;
            }
        }
    }
    let (labels, components) = connected_components(&mask, small_width, small_height);
    choose_component(&components, small_width, small_height, false)
        .map(|component| (labels, component.label))
}

fn connected_components(mask: &[bool], width: usize, height: usize) -> (Vec<u32>, Vec<Component>) {
    let count = width * height;
    let mut labels = vec![0u32; count];
    let mut components = Vec::new();
    let mut stack = Vec::new();
    let mut next_label = 1u32;
    for start in 0..count {
        if !mask[start] || labels[start] != 0 {
            continue;
        }
        let label = next_label;
        next_label += 1;
        labels[start] = label;
        stack.push(start);
        let mut size = 0usize;
        let mut min_x = width;
        let mut min_y = height;
        let mut max_x = 0usize;
        let mut max_y = 0usize;
        while let Some(index) = stack.pop() {
            size += 1;
            let x = index % width;
            let y = index / width;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
            for neighbor in [
                index.checked_sub(1).filter(|_| x > 0),
                (x + 1 < width).then_some(index + 1),
                index.checked_sub(width).filter(|_| y > 0),
                (y + 1 < height).then_some(index + width),
            ]
            .into_iter()
            .flatten()
            {
                if mask[neighbor] && labels[neighbor] == 0 {
                    labels[neighbor] = label;
                    stack.push(neighbor);
                }
            }
        }
        let box_area = (max_x - min_x + 1) * (max_y - min_y + 1);
        components.push(Component {
            label,
            size,
            min_x,
            min_y,
            max_x,
            max_y,
            fill: size as f64 / box_area.max(1) as f64,
        });
    }
    (labels, components)
}

fn choose_component(
    components: &[Component],
    width: usize,
    height: usize,
    bright: bool,
) -> Option<&Component> {
    let total = width * height;
    let minimum = if bright { total * 3 / 100 } else { total / 20 };
    components
        .iter()
        .filter(|component| {
            let box_width = component.max_x - component.min_x + 1;
            let box_height = component.max_y - component.min_y + 1;
            let box_area = box_width * box_height;
            let touches_all = component.min_x <= 1
                && component.min_y <= 1
                && component.max_x + 2 >= width
                && component.max_y + 2 >= height;
            let touches_any = component.min_x <= 1
                || component.min_y <= 1
                || component.max_x + 2 >= width
                || component.max_y + 2 >= height;
            component.size >= minimum
                && component.size <= total * 80 / 100
                && box_area <= total * 85 / 100
                && !touches_all
                && !(bright && touches_any)
                && component.fill >= if bright { 0.30 } else { 0.45 }
                && box_width >= width / 8
                && box_height >= height / 8
                && box_width <= width * 95 / 100
                && box_height <= height * 95 / 100
        })
        .max_by(|a, b| {
            let a_score = a.size as f64 * a.fill;
            let b_score = b.size as f64 * b.fill;
            a_score.total_cmp(&b_score)
        })
}

#[allow(clippy::too_many_arguments)]
fn full_resolution_corners(
    bgra: &[u8],
    width: usize,
    height: usize,
    small_width: usize,
    step: usize,
    labels: &[u32],
    label: u32,
    threshold: Option<u32>,
    use_color: bool,
    background: &[u8; 3],
) -> Option<[(f64, f64); 4]> {
    let mut min_sum = (usize::MAX, usize::MAX);
    let mut max_sum = (0usize, 0usize);
    let mut max_diff = (0usize, 0usize);
    let mut min_diff = (usize::MAX, usize::MAX);
    let mut min_sum_value = i64::MAX;
    let mut max_sum_value = i64::MIN;
    let mut max_diff_value = i64::MIN;
    let mut min_diff_value = i64::MAX;
    let mut promoted = 0usize;
    for y in 0..height {
        let small_y = y / step;
        for x in 0..width {
            let small_x = x / step;
            if labels[small_y * small_width + small_x] != label {
                continue;
            }
            let offset = (y * width + x) * 4;
            let pixel = [bgra[offset + 2], bgra[offset + 1], bgra[offset]];
            let keep = if use_color {
                distance_at(bgra, width, x, y, background) > 24
            } else {
                luminance(pixel) >= threshold.unwrap_or(0)
            };
            if !keep {
                continue;
            }
            promoted += 1;
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
    if promoted < 64 {
        return None;
    }
    let mut points = [
        (min_sum.0 as f64, min_sum.1 as f64),
        (max_diff.0 as f64, max_diff.1 as f64),
        (max_sum.0 as f64, max_sum.1 as f64),
        (min_diff.0 as f64, min_diff.1 as f64),
    ];
    if points
        .iter()
        .enumerate()
        .any(|(index, point)| points.iter().skip(index + 1).any(|other| point == other))
    {
        return None;
    }
    order_clockwise(&mut points);
    Some(points)
}

fn luminance(pixel: [u8; 3]) -> u32 {
    (u32::from(pixel[0]) * 299 + u32::from(pixel[1]) * 587 + u32::from(pixel[2]) * 114) / 1000
}

fn median(values: &mut [u8]) -> u8 {
    values.sort_unstable();
    values[values.len() / 2]
}

fn distance_at(bgra: &[u8], width: usize, x: usize, y: usize, background: &[u8; 3]) -> i32 {
    let offset = (y * width + x) * 4;
    (i32::from(bgra[offset + 2]) - i32::from(background[0]))
        .abs()
        .max((i32::from(bgra[offset + 1]) - i32::from(background[1])).abs())
        .max((i32::from(bgra[offset]) - i32::from(background[2])).abs())
}

fn order_clockwise(points: &mut [(f64, f64); 4]) {
    let cx = points.iter().map(|point| point.0).sum::<f64>() / 4.0;
    let cy = points.iter().map(|point| point.1).sum::<f64>() / 4.0;
    points.sort_by(|a, b| {
        (a.1 - cy)
            .atan2(a.0 - cx)
            .total_cmp(&(b.1 - cy).atan2(b.0 - cx))
    });
    if signed_area(points) < 0.0 {
        points.reverse();
    }
    let start = points
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| (a.0 + a.1).total_cmp(&(b.0 + b.1)))
        .map_or(0, |(index, _)| index);
    points.rotate_left(start);
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

fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

fn valid_quad(points: &[(f64, f64); 4], width: usize, height: usize) -> bool {
    let area = signed_area(points).abs();
    let total = width as f64 * height as f64;
    if area < total * 0.05 || area > total * 1.2 {
        return false;
    }
    let min_x = points.iter().map(|point| point.0).fold(f64::MAX, f64::min);
    let max_x = points.iter().map(|point| point.0).fold(f64::MIN, f64::max);
    let min_y = points.iter().map(|point| point.1).fold(f64::MAX, f64::min);
    let max_y = points.iter().map(|point| point.1).fold(f64::MIN, f64::max);
    if (max_x - min_x) >= width as f64 * 0.985 && (max_y - min_y) >= height as f64 * 0.985 {
        return false;
    }
    let minimum = (width.min(height) as f64) * 0.12;
    let mut positive = false;
    let mut negative = false;
    for index in 0..4 {
        let a = points[index];
        let b = points[(index + 1) % 4];
        let c = points[(index + 2) % 4];
        if distance(a, b) < minimum {
            return false;
        }
        let cross = (b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0);
        if cross > 1e-6 {
            positive = true;
        } else if cross < -1e-6 {
            negative = true;
        } else {
            return false;
        }
        if positive && negative {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas(width: usize, height: usize, background: [u8; 4]) -> Vec<u8> {
        let mut data = Vec::with_capacity(width * height * 4);
        for _ in 0..width * height {
            data.extend_from_slice(&background);
        }
        data
    }

    fn fill_rect(
        data: &mut [u8],
        width: usize,
        left: usize,
        top: usize,
        right: usize,
        bottom: usize,
    ) {
        for y in top..bottom {
            for x in left..right {
                let offset = (y * width + x) * 4;
                data[offset..offset + 4].copy_from_slice(&[250, 250, 250, 255]);
            }
        }
    }

    #[test]
    fn detects_document_despite_noise() {
        let width = 240usize;
        let height = 180usize;
        let mut data = canvas(width, height, [20, 20, 20, 255]);
        fill_rect(&mut data, width, 40, 30, 200, 150);
        for index in 0..width * height {
            if index % 17 == 0 {
                let offset = index * 4;
                data[offset..offset + 4].copy_from_slice(&[240, 240, 240, 255]);
            }
        }
        let corners = detect_document_corners(width as u32, height as u32, &data).expect("detect");
        let xs: Vec<f64> = corners.iter().map(|point| point.0).collect();
        let ys: Vec<f64> = corners.iter().map(|point| point.1).collect();
        assert!(xs.iter().copied().fold(f64::MAX, f64::min) <= 42.0);
        assert!(xs.iter().copied().fold(f64::MIN, f64::max) >= 198.0);
        assert!(ys.iter().copied().fold(f64::MAX, f64::min) <= 32.0);
        assert!(ys.iter().copied().fold(f64::MIN, f64::max) >= 148.0);
    }

    #[test]
    fn ignores_uniform_background() {
        let data = canvas(100, 100, [255, 255, 255, 255]);
        assert!(detect_document_corners(100, 100, &data).is_none());
    }

    #[test]
    fn detects_bright_paper_on_wood() {
        let width = 300usize;
        let height = 220usize;
        let mut data = canvas(width, height, [120, 95, 60, 255]);
        fill_rect(&mut data, width, 70, 40, 240, 180);
        // 模拟键盘/线缆等深色杂物。
        fill_rect(&mut data, width, 0, 0, 60, 40);
        let corners = detect_document_corners(width as u32, height as u32, &data).expect("detect");
        let xs: Vec<f64> = corners.iter().map(|point| point.0).collect();
        let ys: Vec<f64> = corners.iter().map(|point| point.1).collect();
        assert!(xs.iter().copied().fold(f64::MAX, f64::min) <= 72.0);
        assert!(xs.iter().copied().fold(f64::MIN, f64::max) >= 238.0);
        assert!(ys.iter().copied().fold(f64::MAX, f64::min) <= 42.0);
        assert!(ys.iter().copied().fold(f64::MIN, f64::max) >= 178.0);
    }
}
