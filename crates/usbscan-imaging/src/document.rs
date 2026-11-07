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

/// 颜色差异超过该值的像素才参与文档候选（背景估计来自画面边缘）。
const COLOR_DISTANCE_MASK: i32 = 75;
/// 判断单个像素是否属于文档时的颜色差异阈值，略低于掩码阈值以容忍纸张阴影。
const COLOR_DISTANCE_PIXEL: i32 = 60;

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

    // 亮纸张候选：背景较暗时才启用。纸张通常又亮又接近中性色，
    // 用亮度 + 饱和度联合过滤，避免把木桌、橙色物体当成纸张。
    let bright_mask = if background_lum < 200 {
        let threshold = (background_lum + 40).max(95);
        let mut mask = vec![false; count];
        for y in 0..small_height {
            for x in 0..small_width {
                let offset = ((y * step) * width + x * step) * 4;
                let b = bgra[offset];
                let g = bgra[offset + 1];
                let r = bgra[offset + 2];
                let saturation = u32::from(r.max(g).max(b)) - u32::from(r.min(g).min(b));
                if luminance([r, g, b]) >= threshold && saturation <= 100 {
                    mask[y * small_width + x] = true;
                }
            }
        }
        // 先开运算断开细白线/手指等桥接，再闭运算填平纸张褶皱的小孔。
        let mask = morph_open(&mask, small_width, small_height, 2);
        let mask = morph_close(&mask, small_width, small_height, 1);
        Some((mask, threshold))
    } else {
        None
    };

    let (labels, best_label, threshold, use_color, roi) = if let Some((mask, threshold)) =
        bright_mask
    {
        let (labels, components) = connected_components(&mask, small_width, small_height);
        let core_roi = core_roi(&mask, small_width, small_height, 8);
        if let Some(component) = choose_component(&components, small_width, small_height, true) {
            let projected = refine_component_box(
                &labels,
                component.label,
                small_width,
                small_height,
                component,
            );
            let roi = if component.fill >= 0.5 {
                projected
            } else {
                core_roi.or(projected)
            };
            (labels, component.label, Some(threshold), false, roi)
        } else {
            let (labels, label) =
                color_fallback(bgra, width, step, small_width, small_height, &background)?;
            (labels, label, None, true, None)
        }
    } else {
        let (labels, label) =
            color_fallback(bgra, width, step, small_width, small_height, &background)?;
        (labels, label, None, true, None)
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
        roi,
    )?;
    if !valid_quad(&corners, width, height) {
        return None;
    }
    let coverage = quad_paper_coverage(
        &corners,
        bgra,
        width,
        height,
        threshold,
        use_color,
        &background,
    );
    let support = quad_edge_support(&corners, bgra, width, height);
    let minimum_coverage = if use_color { 0.70 } else { 0.86 };
    if coverage < minimum_coverage || support < 0.50 {
        return None;
    }
    Some(corners)
}

/// 采样四边形四条边，估计有多少采样点落在真实边缘上。
fn quad_edge_support(corners: &[(f64, f64); 4], bgra: &[u8], width: usize, height: usize) -> f64 {
    let mut supported = 0usize;
    let mut total = 0usize;
    for side in 0..4 {
        let start = corners[side];
        let end = corners[(side + 1) % 4];
        for step in 1..64 {
            let t = f64::from(step) / 64.0;
            let x = start.0 + (end.0 - start.0) * t;
            let y = start.1 + (end.1 - start.1) * t;
            let x = x.round() as isize;
            let y = y.round() as isize;
            let mut best = 0i32;
            for dy in -15..=15 {
                for dx in -15..=15 {
                    let px = x + dx;
                    let py = y + dy;
                    if px <= 0 || py <= 0 || px >= width as isize - 1 || py >= height as isize - 1 {
                        continue;
                    }
                    let gx = pixel_luminance(bgra, width, px as usize + 1, py as usize)
                        - pixel_luminance(bgra, width, px as usize - 1, py as usize);
                    let gy = pixel_luminance(bgra, width, px as usize, py as usize + 1)
                        - pixel_luminance(bgra, width, px as usize, py as usize - 1);
                    best = best.max(gx.abs() + gy.abs());
                }
            }
            if best > 45 {
                supported += 1;
            }
            total += 1;
        }
    }
    if total == 0 {
        0.0
    } else {
        supported as f64 / total as f64
    }
}

fn pixel_luminance(bgra: &[u8], width: usize, x: usize, y: usize) -> i32 {
    let offset = (y * width + x) * 4;
    (i32::from(bgra[offset + 2]) * 299
        + i32::from(bgra[offset + 1]) * 587
        + i32::from(bgra[offset]) * 114)
        / 1000
}

/// 采样四边形内部，估计有多少比例确实属于纸张。
///
/// 桌面杂物、白键盘等也可能形成四边形，但它们无法让整个四边形内部都保持
/// 纸张颜色。覆盖率不足时宁可跳过自动裁剪，也不错误裁掉页面。
#[allow(clippy::too_many_arguments)]
fn quad_paper_coverage(
    corners: &[(f64, f64); 4],
    bgra: &[u8],
    width: usize,
    height: usize,
    threshold: Option<u32>,
    use_color: bool,
    background: &[u8; 3],
) -> f64 {
    const GRID: usize = 16;
    let mut inside = 0usize;
    let mut paper = 0usize;
    for v in 0..=GRID {
        for u in 0..=GRID {
            let fu = u as f64 / GRID as f64;
            let fv = v as f64 / GRID as f64;
            let top_x = corners[0].0 + (corners[1].0 - corners[0].0) * fu;
            let top_y = corners[0].1 + (corners[1].1 - corners[0].1) * fu;
            let bottom_x = corners[3].0 + (corners[2].0 - corners[3].0) * fu;
            let bottom_y = corners[3].1 + (corners[2].1 - corners[3].1) * fu;
            let x = top_x + (bottom_x - top_x) * fv;
            let y = top_y + (bottom_y - top_y) * fv;
            if x < 0.0 || y < 0.0 {
                continue;
            }
            let (x, y) = (x as usize, y as usize);
            if x >= width || y >= height {
                continue;
            }
            inside += 1;
            let offset = (y * width + x) * 4;
            let pixel = [bgra[offset + 2], bgra[offset + 1], bgra[offset]];
            let is_paper = if use_color {
                distance_at(bgra, width, x, y, background) > COLOR_DISTANCE_PIXEL
            } else {
                let limit = threshold.unwrap_or(150).saturating_sub(20);
                let saturation = u32::from(pixel[0].max(pixel[1]).max(pixel[2]))
                    - u32::from(pixel[0].min(pixel[1]).min(pixel[2]));
                luminance(pixel) >= limit && saturation <= 75
            };
            if is_paper {
                paper += 1;
            }
        }
    }
    if inside == 0 {
        0.0
    } else {
        paper as f64 / inside as f64
    }
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
            if distance_at(bgra, width, x * step, y * step, background) > COLOR_DISTANCE_MASK {
                mask[y * small_width + x] = true;
            }
        }
    }
    let (labels, components) = connected_components(&mask, small_width, small_height);
    choose_component(&components, small_width, small_height, false)
        .map(|component| (labels, component.label))
}

/// 3×3 腐蚀；用于断开只有少量像素宽的桥接。
fn morph_erode(mask: &[bool], width: usize, height: usize) -> Vec<bool> {
    let mut out = vec![false; mask.len()];
    for y in 1..height.saturating_sub(1) {
        for x in 1..width.saturating_sub(1) {
            let index = y * width + x;
            if !mask[index] {
                continue;
            }
            let mut solid = true;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let neighbor = (y as isize + dy) as usize * width + (x as isize + dx) as usize;
                    if !mask[neighbor] {
                        solid = false;
                        break;
                    }
                }
                if !solid {
                    break;
                }
            }
            out[index] = solid;
        }
    }
    out
}

/// 3×3 膨胀。
fn morph_dilate(mask: &[bool], width: usize, height: usize) -> Vec<bool> {
    let mut out = vec![false; mask.len()];
    for y in 1..height.saturating_sub(1) {
        for x in 1..width.saturating_sub(1) {
            if !mask[y * width + x] {
                continue;
            }
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let neighbor = (y as isize + dy) as usize * width + (x as isize + dx) as usize;
                    out[neighbor] = true;
                }
            }
        }
    }
    out
}

fn morph_open(mask: &[bool], width: usize, height: usize, iterations: usize) -> Vec<bool> {
    let mut current = mask.to_vec();
    for _ in 0..iterations {
        current = morph_dilate(&morph_erode(&current, width, height), width, height);
    }
    current
}

fn morph_close(mask: &[bool], width: usize, height: usize, iterations: usize) -> Vec<bool> {
    let mut current = mask.to_vec();
    for _ in 0..iterations {
        current = morph_erode(&morph_dilate(&current, width, height), width, height);
    }
    current
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

/// 对亮区做较强腐蚀，纸张这种厚实区域会保留下来，键盘按键/细线缆会消失。
fn core_roi(
    mask: &[bool],
    width: usize,
    height: usize,
    radius: usize,
) -> Option<(usize, usize, usize, usize)> {
    let mut core = mask.to_vec();
    for _ in 0..radius {
        core = morph_erode(&core, width, height);
    }
    let (_, components) = connected_components(&core, width, height);
    let component = components
        .iter()
        .filter(|component| component.size >= 64)
        .max_by_key(|component| component.size)?;
    let expand = radius as i32 + 3;
    let min_x = (component.min_x as i32 - expand).max(0) as usize;
    let min_y = (component.min_y as i32 - expand).max(0) as usize;
    let max_x = (component.max_x as i32 + expand).min(width as i32 - 1) as usize;
    let max_y = (component.max_y as i32 + expand).min(height as i32 - 1) as usize;
    Some((min_x, min_y, max_x, max_y))
}

/// 在一个可能混入键盘/线缆的连通域里，用行列投影找出最密实的矩形区域。
///
/// 高拍仪画面里的纸张通常是一块连续、各列/各行填充率都很高的区域；白键盘、
/// 白线缆即使和纸张相连，投影上也会在列方向出现明显低谷。
fn refine_component_box(
    labels: &[u32],
    label: u32,
    width: usize,
    height: usize,
    component: &Component,
) -> Option<(usize, usize, usize, usize)> {
    let (x0, y0, x1, y1) = (
        component.min_x,
        component.min_y,
        component.max_x.min(width - 1),
        component.max_y.min(height - 1),
    );
    if x1 <= x0 || y1 <= y0 {
        return None;
    }

    let mut columns = vec![0usize; x1 - x0 + 1];
    for y in y0..=y1 {
        for (offset, column) in columns.iter_mut().enumerate() {
            if labels[y * width + x0 + offset] == label {
                *column += 1;
            }
        }
    }
    let (run_start, run_end) = best_dense_run(&columns)?;
    let (min_x, max_x) = (x0 + run_start, x0 + run_end);

    let mut rows = vec![0usize; y1 - y0 + 1];
    for (offset, row) in rows.iter_mut().enumerate() {
        let y = y0 + offset;
        let mut current = 0usize;
        for x in min_x..=max_x {
            if labels[y * width + x] == label {
                current += 1;
                *row = (*row).max(current);
            } else {
                current = 0;
            }
        }
    }
    let (row_start, row_end) = best_dense_run(&rows)?;
    let (min_y, max_y) = (y0 + row_start, y0 + row_end);

    // 收缩后的区域必须仍然足够大，否则退回原连通域。
    if max_x - min_x + 1 < width / 10 || max_y - min_y + 1 < height / 10 {
        None
    } else {
        Some((min_x, min_y, max_x, max_y))
    }
}

/// 找出填充量最大的连续区间；低于峰值 25% 的列为低谷。
fn best_dense_run(counts: &[usize]) -> Option<(usize, usize)> {
    let peak = *counts.iter().max()?;
    if peak == 0 {
        return None;
    }
    let threshold = (peak * 50 / 100).max(2);
    let mut best: Option<(usize, usize, usize)> = None; // (sum, start, end)
    let mut start = None;
    for (index, &count) in counts.iter().enumerate() {
        if count >= threshold {
            start.get_or_insert(index);
        } else if let Some(run_start) = start.take() {
            let sum: usize = counts[run_start..index].iter().sum();
            if best.is_none_or(|(best_sum, _, _)| sum > best_sum) {
                best = Some((sum, run_start, index - 1));
            }
        }
    }
    if let Some(run_start) = start {
        let sum: usize = counts[run_start..].iter().sum();
        if best.is_none_or(|(best_sum, _, _)| sum > best_sum) {
            best = Some((sum, run_start, counts.len() - 1));
        }
    }
    best.map(|(_, start, end)| (start, end))
}

fn choose_component(
    components: &[Component],
    width: usize,
    height: usize,
    bright: bool,
) -> Option<&Component> {
    let total = width * height;
    let minimum = if bright { total * 2 / 100 } else { total / 20 };
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
            component.size >= minimum
                && component.size <= total * 80 / 100
                && box_area <= total * if bright { 99 } else { 85 } / 100
                && !touches_all
                && component.fill >= if bright { 0.30 } else { 0.45 }
                && box_width >= width / 8
                && box_height >= height / 8
                && box_width <= width * if bright { 100 } else { 95 } / 100
                && box_height <= height * if bright { 100 } else { 95 } / 100
        })
        .max_by(|a, b| {
            component_score(a, width, height, bright)
                .total_cmp(&component_score(b, width, height, bright))
        })
}

/// 候选评分：面积 × 填充率 × 形状 × 居中程度，并轻微惩罚贴边。
fn component_score(component: &Component, width: usize, height: usize, bright: bool) -> f64 {
    let box_width = (component.max_x - component.min_x + 1) as f64;
    let box_height = (component.max_y - component.min_y + 1) as f64;
    let aspect = box_width.min(box_height) / box_width.max(box_height).max(1.0);
    let center_x = (component.min_x + component.max_x) as f64 / 2.0;
    let center_y = (component.min_y + component.max_y) as f64 / 2.0;
    let dx = (center_x - width as f64 / 2.0) / (width as f64 / 2.0).max(1.0);
    let dy = (center_y - height as f64 / 2.0) / (height as f64 / 2.0).max(1.0);
    let center_distance = dx.hypot(dy).min(1.5);
    let center_score = (1.4 - center_distance * 0.5).max(0.4);
    let shape_score = 0.5 + aspect * 0.5;
    let mut score = component.size as f64 * component.fill * shape_score * center_score;
    if bright {
        let edges = usize::from(component.min_x <= 1)
            + usize::from(component.min_y <= 1)
            + usize::from(component.max_x + 2 >= width)
            + usize::from(component.max_y + 2 >= height);
        score *= 1.0 - 0.08 * edges as f64;
    }
    score
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
    roi: Option<(usize, usize, usize, usize)>,
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
    let small_height = labels.len() / small_width;
    for y in 0..height {
        let small_y = (y / step).min(small_height - 1);
        for x in 0..width {
            let small_x = (x / step).min(small_width - 1);
            if labels[small_y * small_width + small_x] != label {
                continue;
            }
            if let Some((min_x, min_y, max_x, max_y)) = roi {
                if small_x < min_x || small_x > max_x || small_y < min_y || small_y > max_y {
                    continue;
                }
            }
            let offset = (y * width + x) * 4;
            let pixel = [bgra[offset + 2], bgra[offset + 1], bgra[offset]];
            let keep = if use_color {
                distance_at(bgra, width, x, y, background) > COLOR_DISTANCE_PIXEL
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

    #[test]
    fn isolates_paper_connected_to_bright_clutter() {
        let width = 600usize;
        let height = 400usize;
        // 木桌背景 + 中央纸张。
        let mut data = canvas(width, height, [120, 95, 60, 255]);
        fill_rect(&mut data, width, 150, 100, 450, 330);
        // 右侧白色键盘，用一条细白线连到纸张，模拟真实杂物的桥接。
        fill_rect(&mut data, width, 480, 40, 580, 130);
        for y in 112..118 {
            for x in 450..480 {
                let offset = (y * width + x) * 4;
                data[offset..offset + 4].copy_from_slice(&[245, 245, 245, 255]);
            }
        }
        let corners = detect_document_corners(width as u32, height as u32, &data).expect("detect");
        let min_x = corners.iter().map(|point| point.0).fold(f64::MAX, f64::min);
        let max_x = corners.iter().map(|point| point.0).fold(f64::MIN, f64::max);
        let min_y = corners.iter().map(|point| point.1).fold(f64::MAX, f64::min);
        let max_y = corners.iter().map(|point| point.1).fold(f64::MIN, f64::max);
        assert!((140.0..=165.0).contains(&min_x), "min_x={min_x}");
        assert!((435.0..=465.0).contains(&max_x), "max_x={max_x}");
        assert!((90.0..=115.0).contains(&min_y), "min_y={min_y}");
        assert!((315.0..=340.0).contains(&max_y), "max_y={max_y}");
    }
}
