// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 自动文档边界检测。
//!
//! 检测分三步：
//! 1. 把画面降采样到约 720 像素，生成“亮纸张”和“与背景有颜色差异”两套掩码；
//!    两套掩码分别找连通域，因此无论桌面比纸张暗、比纸张亮，还是纸张几乎铺满
//!    画面，都能得到候选区域。
//! 2. 对候选连通域取四个对角极值点作为粗四边形，再在原图分辨率上沿四条边做
//!    梯度搜索并拟合直线，得到像素级精度的边和角点。
//! 3. 用纸面覆盖率、边缘支持度和四边形几何约束剔除键盘、线缆、桌面纹理等
//!    误检；不足置信度时返回 `None`，宁可跳过也不错误裁剪。
//!
//! 另外提供基于投影剖面的纠偏角度估计，供“自动纠偏”在页面上没有明显边界
//! （例如已经裁过边）时使用。

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    clippy::trivially_copy_pass_by_ref,
    clippy::needless_range_loop,
    clippy::similar_names
)]

/// 降采样后的最长边。
const WORK_MAX_SIDE: usize = 720;
/// 连通域至少要占画面的 1.5%。
const MIN_COMPONENT_PERMILLE: usize = 15;
/// 亮纸张候选的纸面覆盖率下限。
const BRIGHT_COVERAGE: f64 = 0.78;
/// 颜色差异候选的纸面覆盖率下限。
const COLOR_COVERAGE: f64 = 0.68;
/// 四条边中平均有多少比例能对上真实梯度。
const MIN_EDGE_SUPPORT: f64 = 0.24;

struct Component {
    label: u32,
    size: usize,
    min_x: usize,
    min_y: usize,
    max_x: usize,
    max_y: usize,
    fill: f64,
}

/// 降采样工作区：保留原始 BGRA 引用、步长和灰度图。
struct Grid<'a> {
    bgra: &'a [u8],
    width: usize,
    height: usize,
    step: usize,
    width_small: usize,
    height_small: usize,
    gray: Vec<u8>,
}

impl<'a> Grid<'a> {
    fn new(width: usize, height: usize, bgra: &'a [u8]) -> Option<Self> {
        if width < 32 || height < 32 || bgra.len() < width * height * 4 {
            return None;
        }
        let step = width.max(height).div_ceil(WORK_MAX_SIDE).max(1);
        let width_small = (width / step).max(1);
        let height_small = (height / step).max(1);
        if width_small < 24 || height_small < 24 {
            return None;
        }
        let mut gray = vec![0u8; width_small * height_small];
        for y in 0..height_small {
            for x in 0..width_small {
                let offset = ((y * step) * width + x * step) * 4;
                gray[y * width_small + x] = u8::try_from(luminance([
                    bgra[offset + 2],
                    bgra[offset + 1],
                    bgra[offset],
                ]))
                .unwrap_or(255);
            }
        }
        Some(Self {
            bgra,
            width,
            height,
            step,
            width_small,
            height_small,
            gray,
        })
    }

    fn color_at(&self, x: usize, y: usize) -> [u8; 3] {
        let offset = ((y * self.step) * self.width + x * self.step) * 4;
        [
            self.bgra[offset + 2],
            self.bgra[offset + 1],
            self.bgra[offset],
        ]
    }

    fn offset_at(&self, x: usize, y: usize) -> usize {
        ((y * self.step) * self.width + x * self.step) * 4
    }
}

struct BorderStats {
    background: [u8; 3],
    /// 边框像素与背景色的中位偏差，用来决定颜色差异阈值。
    noise: i32,
    /// 边框里像纸张（明亮、低饱和）的采样比例。
    paper_fraction: f64,
}

fn border_stats(grid: &Grid<'_>) -> Option<BorderStats> {
    let mut colors = Vec::with_capacity(2 * (grid.width_small + grid.height_small));
    for x in 0..grid.width_small {
        colors.push(grid.color_at(x, 0));
        colors.push(grid.color_at(x, grid.height_small - 1));
    }
    for y in 1..grid.height_small.saturating_sub(1) {
        colors.push(grid.color_at(0, y));
        colors.push(grid.color_at(grid.width_small - 1, y));
    }
    if colors.is_empty() {
        return None;
    }
    let mut red: Vec<u8> = colors.iter().map(|color| color[0]).collect();
    let mut green: Vec<u8> = colors.iter().map(|color| color[1]).collect();
    let mut blue: Vec<u8> = colors.iter().map(|color| color[2]).collect();
    let background = [median(&mut red), median(&mut green), median(&mut blue)];
    let mut deviations: Vec<i32> = colors
        .iter()
        .map(|color| color_distance(color, &background))
        .collect();
    let noise = median_i32(&mut deviations);
    let paper = colors.iter().filter(|color| is_paper_like(**color)).count();
    Some(BorderStats {
        background,
        noise,
        paper_fraction: paper as f64 / colors.len() as f64,
    })
}

fn is_paper_like(color: [u8; 3]) -> bool {
    let saturation = u32::from(color[0].max(color[1]).max(color[2]))
        - u32::from(color[0].min(color[1]).min(color[2]));
    luminance(color) >= 185 && saturation <= 90
}

fn color_distance(a: &[u8; 3], b: &[u8; 3]) -> i32 {
    (i32::from(a[0]) - i32::from(b[0]))
        .abs()
        .max((i32::from(a[1]) - i32::from(b[1])).abs())
        .max((i32::from(a[2]) - i32::from(b[2])).abs())
}

struct Otsu {
    threshold: u8,
    bright_mean: f64,
}

/// Otsu 阈值；两个类别不够明显时返回 `None`。
fn otsu_threshold(gray: &[u8]) -> Option<Otsu> {
    let mut histogram = [0u32; 256];
    for &value in gray {
        histogram[value as usize] += 1;
    }
    let total = gray.len() as f64;
    let sum_all: f64 = histogram
        .iter()
        .enumerate()
        .map(|(value, &count)| value as f64 * f64::from(count))
        .sum();
    let mut weight_dark = 0.0f64;
    let mut sum_dark = 0.0f64;
    let mut best_variance = -1.0f64;
    let mut best_threshold = 0usize;
    for (value, &count) in histogram.iter().enumerate() {
        weight_dark += f64::from(count);
        if weight_dark == 0.0 {
            continue;
        }
        let weight_bright = total - weight_dark;
        if weight_bright <= 0.0 {
            break;
        }
        sum_dark += value as f64 * f64::from(count);
        let mean_dark = sum_dark / weight_dark;
        let mean_bright = (sum_all - sum_dark) / weight_bright;
        let variance = weight_dark * weight_bright * (mean_dark - mean_bright).powi(2);
        if variance > best_variance {
            best_variance = variance;
            best_threshold = value;
        }
    }
    if best_variance <= 0.0 {
        return None;
    }
    let dark: u32 = histogram[..=best_threshold].iter().sum();
    let bright: u32 = histogram[best_threshold + 1..].iter().sum();
    if dark < (gray.len() as u32 / 50) || bright < (gray.len() as u32 / 50) {
        return None;
    }
    let peak = histogram.iter().copied().max().unwrap_or(0);
    if dark < peak / 20 || bright < peak / 20 {
        return None;
    }
    let mean_dark = histogram[..=best_threshold]
        .iter()
        .enumerate()
        .map(|(value, &count)| value as f64 * f64::from(count))
        .sum::<f64>()
        / f64::from(dark);
    let mean_bright = histogram[best_threshold + 1..]
        .iter()
        .enumerate()
        .map(|(value, &count)| (value + best_threshold + 1) as f64 * f64::from(count))
        .sum::<f64>()
        / f64::from(bright);
    if mean_bright - mean_dark < 26.0 {
        return None;
    }
    Some(Otsu {
        threshold: u8::try_from(best_threshold).ok()?,
        bright_mean: mean_bright,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CoverageMode {
    /// 亮纸张：亮于阈值，或明显不同于背景的印刷内容。
    Bright,
    /// 颜色差异：明显不同于背景。
    Color,
    /// 背景本身就是纸色：接近背景或亮于纸张阈值。
    Similar,
}

struct MaskSpec {
    mask: Vec<bool>,
    mode: CoverageMode,
    /// 背景是纸色时，前景（桌面杂物）不能大面积贴边。
    require_inset: bool,
}

/// 生成候选掩码。
fn build_masks(grid: &Grid<'_>, border: &BorderStats, otsu: Option<&Otsu>) -> (Vec<MaskSpec>, i32) {
    let background_luminance = luminance(border.background) as i32;
    let mut diff_threshold = (border.noise * 3 + 22).clamp(36, 90);
    if background_luminance > 205 {
        diff_threshold = diff_threshold.min(40);
    }
    // 边框本身是纸色，且画面里没有明显更亮的纸张：文档就是背景色区域，
    // 桌面杂物反而是“前景”。这时颜色差异候选必须离画面边缘足够远。
    let paper_background = border.paper_fraction >= 0.5
        && background_luminance >= 180
        && border.noise <= 40
        && otsu.is_none_or(|otsu| otsu.bright_mean <= f64::from(background_luminance as u8) + 18.0);

    let mut masks = Vec::new();
    if let Some(otsu) = otsu {
        // 纸张比背景亮时，阈值不能低于“背景亮度 + 余量”，否则深色内容会把
        // Otsu 拉到桌面高度，整个桌面都会被当成纸张。
        let threshold = if paper_background {
            i32::from(otsu.threshold) - 8
        } else {
            (i32::from(otsu.threshold) - 8).max(background_luminance + 16)
        };
        if threshold <= 244 {
            let threshold = u8::try_from(threshold).unwrap_or(244);
            masks.push(MaskSpec {
                mask: grid.gray.iter().map(|&value| value >= threshold).collect(),
                mode: CoverageMode::Bright,
                require_inset: false,
            });
        }
    }
    if paper_background {
        let similar_threshold = (border.noise * 2 + 16).clamp(18, 34);
        masks.push(MaskSpec {
            mask: (0..grid.gray.len())
                .map(|index| {
                    let x = index % grid.width_small;
                    let y = index / grid.width_small;
                    let offset = grid.offset_at(x, y);
                    let color = [
                        grid.bgra[offset + 2],
                        grid.bgra[offset + 1],
                        grid.bgra[offset],
                    ];
                    color_distance(&color, &border.background) <= similar_threshold
                })
                .collect(),
            mode: CoverageMode::Similar,
            require_inset: false,
        });
    }
    masks.push(MaskSpec {
        mask: (0..grid.gray.len())
            .map(|index| {
                let x = index % grid.width_small;
                let y = index / grid.width_small;
                let offset = grid.offset_at(x, y);
                let color = [
                    grid.bgra[offset + 2],
                    grid.bgra[offset + 1],
                    grid.bgra[offset],
                ];
                color_distance(&color, &border.background) > diff_threshold
            })
            .collect(),
        mode: CoverageMode::Color,
        require_inset: paper_background,
    });
    (masks, diff_threshold)
}

/// 检测文档四角；顺序为左上、右上、右下、左下。
pub fn detect_document_corners(width: u32, height: u32, bgra: &[u8]) -> Option<[(f64, f64); 4]> {
    let grid = Grid::new(width as usize, height as usize, bgra)?;
    let border = border_stats(&grid)?;
    let otsu = otsu_threshold(&grid.gray);
    let (masks, diff_threshold) = build_masks(&grid, &border, otsu.as_ref());
    let mut best: Option<(f64, [(f64, f64); 4])> = None;
    for spec in masks {
        if let Some((score, corners)) = best_from_mask(
            &grid,
            &spec,
            border.background,
            otsu.as_ref(),
            diff_threshold,
        ) {
            if best
                .as_ref()
                .is_none_or(|(best_score, _)| score > *best_score)
            {
                best = Some((score, corners));
            }
        }
    }
    best.map(|(_, corners)| corners)
}

fn best_from_mask(
    grid: &Grid<'_>,
    spec: &MaskSpec,
    background: [u8; 3],
    otsu: Option<&Otsu>,
    diff_threshold: i32,
) -> Option<(f64, [(f64, f64); 4])> {
    let width = grid.width_small;
    let height = grid.height_small;
    // 整幅画面都属于同一类（例如纯白桌面）时没有边界可找。
    let raw_fraction =
        spec.mask.iter().filter(|&&value| value).count() as f64 / spec.mask.len() as f64;
    if raw_fraction >= 0.995 {
        return None;
    }
    let cleaned = morph_close(&morph_open(&spec.mask, width, height, 1), width, height, 2);
    let (labels, components) = connected_components(&cleaned, width, height);
    let total = width * height;
    let bright = spec.mode == CoverageMode::Bright;
    let mut ranked: Vec<&Component> = components
        .iter()
        .filter(|component| component_eligible(component, width, height, total))
        .filter(|component| !spec.require_inset || component_inset(component, width, height))
        .collect();
    ranked.sort_by(|a, b| {
        component_score(b, width, height, bright)
            .total_cmp(&component_score(a, width, height, bright))
    });

    let radius = (grid.step * 2 + 4).clamp(6, 28) as i32;
    let minimum_coverage = match spec.mode {
        CoverageMode::Bright => BRIGHT_COVERAGE,
        CoverageMode::Color => COLOR_COVERAGE,
        CoverageMode::Similar => 0.62,
    };
    let mut best: Option<(f64, [(f64, f64); 4])> = None;
    for component in ranked.iter().take(6) {
        // 同时尝试“整个连通域”和“投影收缩后的区域”，由边缘支持度决定哪个
        // 才是真正的纸张；这样既能排除连在一起的键盘，也不会误伤旋转页面。
        for roi in component_rois(&labels, component, width, height) {
            let Some(small) = quad_from_component(&labels, component.label, width, height, roi)
            else {
                continue;
            };
            let coarse = scale_quad(&small, grid.step, grid.width, grid.height);
            let (corners, support) =
                refine_quad(&coarse, grid.width, grid.height, grid.bgra, radius);
            if !valid_quad(&corners, grid.width, grid.height) {
                continue;
            }
            let coverage =
                quad_coverage(grid, &corners, &background, otsu, diff_threshold, spec.mode);
            if coverage < minimum_coverage || support < MIN_EDGE_SUPPORT {
                continue;
            }
            let mut score = component_score(component, width, height, bright)
                * (0.5 + 0.5 * coverage)
                * (0.5 + 0.5 * support);
            if bright {
                score *= 1.05;
            }
            if best
                .as_ref()
                .is_none_or(|(best_score, _)| score > *best_score)
            {
                best = Some((score, corners));
            }
        }
    }
    best
}

/// 连通域是否至少从三个方向贴住画面边缘。
fn component_inset(component: &Component, width: usize, height: usize) -> bool {
    let margin_x = (width / 100).max(1);
    let margin_y = (height / 100).max(1);
    let edges = usize::from(component.min_x <= margin_x)
        + usize::from(component.min_y <= margin_y)
        + usize::from(component.max_x + margin_x >= width)
        + usize::from(component.max_y + margin_y >= height);
    edges <= 1
}

fn component_eligible(component: &Component, width: usize, height: usize, total: usize) -> bool {
    let box_width = component.max_x - component.min_x + 1;
    let box_height = component.max_y - component.min_y + 1;
    component.size * 1000 >= total * MIN_COMPONENT_PERMILLE
        && component.size * 1000 <= total * 997
        && box_width >= width / 10
        && box_height >= height / 10
        && component.fill >= 0.18
}

/// 候选区域：连通域整体，以及混入键盘/线缆时投影收窄后的区域。
fn component_rois(
    labels: &[u32],
    component: &Component,
    width: usize,
    height: usize,
) -> Vec<Option<(usize, usize, usize, usize)>> {
    let margin_x = (width / 25).max(1);
    let margin_y = (height / 25).max(1);
    let near_full = component.min_x <= margin_x
        && component.min_y <= margin_y
        && component.max_x + margin_x >= width
        && component.max_y + margin_y >= height;
    if near_full {
        return vec![None];
    }
    let bbox = (
        component.min_x,
        component.min_y,
        component.max_x,
        component.max_y,
    );
    let mut rois = vec![Some(bbox)];
    if let Some(projected) = refine_component_box(labels, component.label, width, height, component)
    {
        if projected != bbox {
            rois.push(Some(projected));
        }
    }
    rois
}

/// 在一个可能混入键盘/线缆的连通域里，用行列投影找出最密实的矩形区域。
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

    if max_x - min_x + 1 < width / 10 || max_y - min_y + 1 < height / 10 {
        None
    } else {
        Some((min_x, min_y, max_x, max_y))
    }
}

/// 找出填充量最大的连续区间；低于峰值 50% 的位置视为低谷。
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

/// 取连通域的四个对角极值点，得到粗四边形。
fn quad_from_component(
    labels: &[u32],
    label: u32,
    width: usize,
    height: usize,
    roi: Option<(usize, usize, usize, usize)>,
) -> Option<[(f64, f64); 4]> {
    let (x0, y0, x1, y1) = roi.unwrap_or((0, 0, width - 1, height - 1));
    let mut min_sum = (0usize, 0usize);
    let mut max_sum = (0usize, 0usize);
    let mut min_diff = (0usize, 0usize);
    let mut max_diff = (0usize, 0usize);
    let mut min_sum_value = i64::MAX;
    let mut max_sum_value = i64::MIN;
    let mut min_diff_value = i64::MAX;
    let mut max_diff_value = i64::MIN;
    let mut promoted = 0usize;
    for y in y0..=y1.min(height - 1) {
        for x in x0..=x1.min(width - 1) {
            if labels[y * width + x] != label {
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
            if diff < min_diff_value {
                min_diff_value = diff;
                min_diff = (x, y);
            }
            if diff > max_diff_value {
                max_diff_value = diff;
                max_diff = (x, y);
            }
        }
    }
    if promoted < 48 {
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

fn scale_quad(
    small: &[(f64, f64); 4],
    step: usize,
    width: usize,
    height: usize,
) -> [(f64, f64); 4] {
    small.map(|(x, y)| {
        (
            (x * step as f64).clamp(0.0, width as f64 - 1.0),
            (y * step as f64).clamp(0.0, height as f64 - 1.0),
        )
    })
}

/// 在原始分辨率上沿四条边搜索梯度极大值，拟合直线后求交点。
fn refine_quad(
    coarse: &[(f64, f64); 4],
    width: usize,
    height: usize,
    bgra: &[u8],
    radius: i32,
) -> ([(f64, f64); 4], f64) {
    let mut lines = [(coarse[0], coarse[1]); 4];
    let mut supports = [0.0f64; 4];
    for index in 0..4 {
        let start = coarse[index];
        let end = coarse[(index + 1) % 4];
        // 页面被画面边缘裁掉时，这条边本来就不在画面里，直接信任粗定位。
        if edge_on_frame_border(start, end, width, height, radius) {
            supports[index] = 1.0;
            continue;
        }
        if let Some((refined_start, refined_end, support)) =
            refine_edge(start, end, radius, width, height, bgra)
        {
            lines[index] = (refined_start, refined_end);
            supports[index] = support;
        }
    }

    let mut corners = *coarse;
    for index in 0..4 {
        let previous = lines[(index + 3) % 4];
        let next = lines[index];
        if let Some(point) = line_intersection(previous.0, previous.1, next.0, next.1) {
            let margin_x = width as f64 * 0.2;
            let margin_y = height as f64 * 0.2;
            if point.0 > -margin_x
                && point.0 < width as f64 + margin_x
                && point.1 > -margin_y
                && point.1 < height as f64 + margin_y
            {
                corners[index] = point;
            }
        }
    }
    if !valid_quad(&corners, width, height) {
        corners = *coarse;
    }
    let support = supports.iter().sum::<f64>() / 4.0;
    (corners, support)
}

/// 这条边是否整体贴着画面边框（页面被裁掉一部分）。
fn edge_on_frame_border(
    start: (f64, f64),
    end: (f64, f64),
    width: usize,
    height: usize,
    radius: i32,
) -> bool {
    const SAMPLES: usize = 10;
    let limit = f64::from(radius) + 1.0;
    let mut top = 0usize;
    let mut bottom = 0usize;
    let mut left = 0usize;
    let mut right = 0usize;
    for index in 0..=SAMPLES {
        let t = index as f64 / SAMPLES as f64;
        let x = start.0 + (end.0 - start.0) * t;
        let y = start.1 + (end.1 - start.1) * t;
        if y <= limit {
            top += 1;
        }
        if y >= height as f64 - 1.0 - limit {
            bottom += 1;
        }
        if x <= limit {
            left += 1;
        }
        if x >= width as f64 - 1.0 - limit {
            right += 1;
        }
    }
    let needed = SAMPLES * 9 / 10;
    top >= needed || bottom >= needed || left >= needed || right >= needed
}

/// 拟合后的边：起点、终点和强边缘采样比例。
type RefinedEdge = ((f64, f64), (f64, f64), f64);

/// 沿一条边搜索法向梯度，返回拟合后的两端点和强边缘采样比例。
fn refine_edge(
    start: (f64, f64),
    end: (f64, f64),
    radius: i32,
    width: usize,
    height: usize,
    bgra: &[u8],
) -> Option<RefinedEdge> {
    let length = distance(start, end);
    if length < 24.0 {
        return None;
    }
    let tangent = ((end.0 - start.0) / length, (end.1 - start.1) / length);
    let normal = (-tangent.1, tangent.0);
    let samples = (length as usize / 6).clamp(24, 160);

    let mut offsets: Vec<(f64, f64, f64)> = Vec::with_capacity(samples);
    for index in 0..samples {
        let fraction = 0.04 + 0.92 * (index as f64 / (samples - 1) as f64);
        let base_x = start.0 + (end.0 - start.0) * fraction;
        let base_y = start.1 + (end.1 - start.1) * fraction;
        let mut best_offset = 0.0f64;
        let mut best_magnitude = -1.0f64;
        let mut found = false;
        for step in -radius..=radius {
            let offset = f64::from(step);
            let x = base_x + normal.0 * offset;
            let y = base_y + normal.1 * offset;
            let Some(plus) = luminance_bilinear(bgra, width, height, x + normal.0, y + normal.1)
            else {
                continue;
            };
            let Some(minus) = luminance_bilinear(bgra, width, height, x - normal.0, y - normal.1)
            else {
                continue;
            };
            let magnitude = (plus - minus).abs();
            if magnitude > best_magnitude {
                best_magnitude = magnitude;
                best_offset = offset;
                found = true;
            }
        }
        // 最大值贴着搜索边界，说明真实边缘可能在范围之外，丢弃该采样。
        if found && best_offset.abs() < f64::from(radius) - 0.5 {
            offsets.push((fraction, best_offset, best_magnitude));
        }
    }
    if offsets.len() < 8 {
        return None;
    }
    let maximum = offsets.iter().map(|sample| sample.2).fold(0.0f64, f64::max);
    let threshold = (maximum * 0.30).max(10.0);
    let strong: Vec<(f64, f64)> = offsets
        .iter()
        .filter(|sample| sample.2 >= threshold)
        .map(|sample| (sample.0, sample.1))
        .collect();
    if strong.len() * 5 < offsets.len() {
        return None;
    }
    let (slope, intercept) = robust_line_fit(&strong)?;
    let support = strong.len() as f64 / offsets.len() as f64;
    let adjusted_start = (
        start.0 + normal.0 * intercept,
        start.1 + normal.1 * intercept,
    );
    let adjusted_end = (
        end.0 + normal.0 * (intercept + slope),
        end.1 + normal.1 * (intercept + slope),
    );
    Some((adjusted_start, adjusted_end, support))
}

/// 最小二乘拟合 `offset = intercept + slope * fraction`，并用 MAD 剔除离群点。
fn robust_line_fit(points: &[(f64, f64)]) -> Option<(f64, f64)> {
    if points.len() < 3 {
        return None;
    }
    let mut inliers = points.to_vec();
    let mut fit = least_squares(&inliers);
    for _ in 0..3 {
        let residuals: Vec<f64> = inliers
            .iter()
            .map(|(t, offset)| offset - (fit.0 * t + fit.1))
            .collect();
        let mut absolute: Vec<f64> = residuals.iter().map(|value| value.abs()).collect();
        absolute.sort_by(f64::total_cmp);
        let median = absolute[absolute.len() / 2];
        let limit = (median * 3.0).max(1.5);
        let filtered: Vec<(f64, f64)> = inliers
            .iter()
            .zip(&residuals)
            .filter(|(_, residual)| residual.abs() <= limit)
            .map(|(point, _)| *point)
            .collect();
        if filtered.len() < 3 || filtered.len() == inliers.len() {
            break;
        }
        inliers = filtered;
        fit = least_squares(&inliers);
    }
    Some(fit)
}

fn least_squares(points: &[(f64, f64)]) -> (f64, f64) {
    let count = points.len() as f64;
    let sum_t: f64 = points.iter().map(|(t, _)| t).sum();
    let sum_o: f64 = points.iter().map(|(_, offset)| offset).sum();
    let sum_tt: f64 = points.iter().map(|(t, _)| t * t).sum();
    let sum_to: f64 = points.iter().map(|(t, offset)| t * offset).sum();
    let denominator = count * sum_tt - sum_t * sum_t;
    if denominator.abs() < 1e-9 {
        return (0.0, sum_o / count);
    }
    let slope = (count * sum_to - sum_t * sum_o) / denominator;
    let intercept = (sum_o - slope * sum_t) / count;
    (slope, intercept)
}

fn line_intersection(
    a1: (f64, f64),
    b1: (f64, f64),
    a2: (f64, f64),
    b2: (f64, f64),
) -> Option<(f64, f64)> {
    let r = (b1.0 - a1.0, b1.1 - a1.1);
    let s = (b2.0 - a2.0, b2.1 - a2.1);
    let denominator = r.0 * s.1 - r.1 * s.0;
    if denominator.abs() < 1e-6 {
        return None;
    }
    let dx = a2.0 - a1.0;
    let dy = a2.1 - a1.1;
    let t = (dx * s.1 - dy * s.0) / denominator;
    Some((a1.0 + r.0 * t, a1.1 + r.1 * t))
}

fn luminance_bilinear(bgra: &[u8], width: usize, height: usize, x: f64, y: f64) -> Option<f64> {
    if x < 0.0 || y < 0.0 || x >= width as f64 - 1.0 || y >= height as f64 - 1.0 {
        return None;
    }
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let fx = x - x0 as f64;
    let fy = y - y0 as f64;
    let value = |px: usize, py: usize| f64::from(pixel_luminance(bgra, width, px, py));
    let top = value(x0, y0) * (1.0 - fx) + value(x0 + 1, y0) * fx;
    let bottom = value(x0, y0 + 1) * (1.0 - fx) + value(x0 + 1, y0 + 1) * fx;
    Some(top * (1.0 - fy) + bottom * fy)
}

/// 采样四边形内部，估计有多少比例确实属于纸张或纸面内容。
fn quad_coverage(
    grid: &Grid<'_>,
    corners: &[(f64, f64); 4],
    background: &[u8; 3],
    otsu: Option<&Otsu>,
    diff_threshold: i32,
    mode: CoverageMode,
) -> f64 {
    const GRID: usize = 22;
    let width = grid.width;
    let height = grid.height;
    let mut inside = 0usize;
    let mut paper = 0usize;
    for v in 0..=GRID {
        for u in 0..=GRID {
            let fu = 0.03 + 0.94 * (u as f64 / GRID as f64);
            let fv = 0.03 + 0.94 * (v as f64 / GRID as f64);
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
            let color = [
                grid.bgra[offset + 2],
                grid.bgra[offset + 1],
                grid.bgra[offset],
            ];
            let distance = color_distance(&color, background);
            let bright_enough =
                otsu.is_some_and(|otsu| luminance(color) as i32 >= i32::from(otsu.threshold) - 24);
            let is_paper = match mode {
                CoverageMode::Bright => bright_enough || distance > diff_threshold * 3 / 5,
                CoverageMode::Color => distance > diff_threshold * 3 / 5,
                CoverageMode::Similar => distance <= 30 || bright_enough,
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

fn pixel_luminance(bgra: &[u8], width: usize, x: usize, y: usize) -> i32 {
    let offset = (y * width + x) * 4;
    (i32::from(bgra[offset + 2]) * 299
        + i32::from(bgra[offset + 1]) * 587
        + i32::from(bgra[offset]) * 114)
        / 1000
}

fn luminance(pixel: [u8; 3]) -> u32 {
    (u32::from(pixel[0]) * 299 + u32::from(pixel[1]) * 587 + u32::from(pixel[2]) * 114) / 1000
}

fn median(values: &mut [u8]) -> u8 {
    values.sort_unstable();
    values[values.len() / 2]
}

fn median_i32(values: &mut [i32]) -> i32 {
    values.sort_unstable();
    values[values.len() / 2]
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
    if area < total * 0.03 || area > total * 1.2 {
        return false;
    }
    let minimum = (width.min(height) as f64) * 0.08;
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
    // 退化/极锐的角多半是误检。
    for index in 0..4 {
        let previous = points[(index + 3) % 4];
        let current = points[index];
        let next = points[(index + 1) % 4];
        let a = (previous.0 - current.0, previous.1 - current.1);
        let b = (next.0 - current.0, next.1 - current.1);
        let length_a = a.0.hypot(a.1);
        let length_b = b.0.hypot(b.1);
        if length_a < 1.0 || length_b < 1.0 {
            return false;
        }
        let cosine = ((a.0 * b.0 + a.1 * b.1) / (length_a * length_b)).clamp(-1.0, 1.0);
        let angle = cosine.acos().to_degrees();
        if !(18.0..=162.0).contains(&angle) {
            return false;
        }
    }
    true
}

/// 估计把页面转正所需的旋转角度（度）；没有足够纹理时返回 `None`。
///
/// 使用行、列投影剖面的锐度搜索最佳角度。水平文本、表格线和纸张边缘都会
/// 在正确角度上让投影剖面出现尖峰，因此比只看纸张上下边的角度更稳定。
pub fn estimate_deskew(width: u32, height: u32, bgra: &[u8]) -> Option<f32> {
    estimate_deskew_in_quad(width as usize, height as usize, bgra, None)
}

pub(crate) fn estimate_deskew_in_quad(
    width: usize,
    height: usize,
    bgra: &[u8],
    corners: Option<&[(f64, f64); 4]>,
) -> Option<f32> {
    let grid = Grid::new(width, height, bgra)?;
    let small = grid.width_small;
    let tall = grid.height_small;
    let quad =
        corners.map(|corners| corners.map(|(x, y)| (x / grid.step as f64, y / grid.step as f64)));
    let mut points: Vec<(f32, f32, f32)> = Vec::new();
    for y in 1..tall.saturating_sub(1) {
        for x in 1..small.saturating_sub(1) {
            if let Some(quad) = quad.as_ref() {
                if !point_in_quad(x as f64, y as f64, quad) {
                    continue;
                }
            }
            let gx =
                i32::from(grid.gray[y * small + x + 1]) - i32::from(grid.gray[y * small + x - 1]);
            let gy = i32::from(grid.gray[(y + 1) * small + x])
                - i32::from(grid.gray[(y - 1) * small + x]);
            let weight = gx.abs() + gy.abs();
            if weight < 36 {
                continue;
            }
            points.push((x as f32, y as f32, weight as f32));
        }
    }
    if points.len() < 96 {
        return None;
    }
    if points.len() > 40_000 {
        let stride = points.len().div_ceil(40_000);
        let reduced: Vec<(f32, f32, f32)> = points.iter().step_by(stride).copied().collect();
        points = reduced;
    }

    let cx = small as f32 / 2.0;
    let cy = tall as f32 / 2.0;
    let mut row_bins = vec![0.0f32; tall + 2];
    let mut column_bins = vec![0.0f32; small + 2];
    let zero_score = projection_sharpness(&points, cx, cy, 0.0, &mut row_bins, &mut column_bins);
    let mut best_angle = 0.0f32;
    let mut best_score = 0.0f32;
    let mut angle = -22.0f32;
    while angle <= 22.0 {
        let score = projection_sharpness(&points, cx, cy, angle, &mut row_bins, &mut column_bins);
        if score > best_score {
            best_score = score;
            best_angle = angle;
        }
        angle += 0.5;
    }
    let coarse = best_angle;
    let mut angle = coarse - 0.5;
    while angle <= coarse + 0.5 {
        let score = projection_sharpness(&points, cx, cy, angle, &mut row_bins, &mut column_bins);
        if score > best_score {
            best_score = score;
            best_angle = angle;
        }
        angle += 0.05;
    }
    if best_score <= 1e-6 {
        return None;
    }
    // 搜索边界上的“最优角”通常说明画面里没有可靠的水平/垂直线。
    if best_angle.abs() > 21.0 {
        return None;
    }
    // 转正后投影没有明显变锐利，就不要为了 0.4° 的噪声旋转图片。
    if best_score <= zero_score * 1.02 {
        return Some(0.0);
    }
    Some(best_angle)
}

fn projection_sharpness(
    points: &[(f32, f32, f32)],
    cx: f32,
    cy: f32,
    angle: f32,
    row_bins: &mut [f32],
    column_bins: &mut [f32],
) -> f32 {
    row_bins.fill(0.0);
    column_bins.fill(0.0);
    let radians = angle.to_radians();
    let (sin, cos) = radians.sin_cos();
    for &(x, y, weight) in points {
        let dx = x - cx;
        let dy = y - cy;
        let row = dx * sin + dy * cos + cy;
        let column = dx * cos - dy * sin + cx;
        let row_index = row as isize;
        if row_index >= 0 && (row_index as usize) < row_bins.len() {
            row_bins[row_index as usize] += weight;
        }
        let column_index = column as isize;
        if column_index >= 0 && (column_index as usize) < column_bins.len() {
            column_bins[column_index as usize] += weight;
        }
    }
    normalized_variance(row_bins) + normalized_variance(column_bins)
}

fn normalized_variance(bins: &[f32]) -> f32 {
    let mean = bins.iter().sum::<f32>() / bins.len() as f32;
    if mean <= 0.0 {
        return 0.0;
    }
    bins.iter()
        .map(|value| (value - mean) * (value - mean))
        .sum::<f32>()
        / bins.len() as f32
        / (mean * mean)
}

fn point_in_quad(x: f64, y: f64, quad: &[(f64, f64); 4]) -> bool {
    let mut inside = false;
    let mut previous = 3;
    for current in 0..4 {
        let (xi, yi) = quad[current];
        let (xj, yj) = quad[previous];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        previous = current;
    }
    inside
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
        color: [u8; 4],
    ) {
        for y in top..bottom {
            for x in left..right {
                let offset = (y * width + x) * 4;
                data[offset..offset + 4].copy_from_slice(&color);
            }
        }
    }

    fn fill_polygon(data: &mut [u8], width: usize, points: &[(f64, f64); 4]) {
        let height = data.len() / width / 4;
        for y in 0..height {
            for x in 0..width {
                if !point_in_polygon(x as f64 + 0.5, y as f64 + 0.5, points) {
                    continue;
                }
                let offset = (y * width + x) * 4;
                data[offset..offset + 4].copy_from_slice(&[250, 250, 250, 255]);
            }
        }
    }

    fn point_in_polygon(x: f64, y: f64, points: &[(f64, f64); 4]) -> bool {
        let mut inside = false;
        let mut previous = 3;
        for current in 0..4 {
            let (xi, yi) = points[current];
            let (xj, yj) = points[previous];
            if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
                inside = !inside;
            }
            previous = current;
        }
        inside
    }

    fn corner_error(corners: &[(f64, f64); 4], expected: &[(f64, f64); 4]) -> f64 {
        corners
            .iter()
            .zip(expected)
            .map(|(actual, expected)| distance(*actual, *expected))
            .fold(0.0, f64::max)
    }

    fn min_max(corners: &[(f64, f64); 4]) -> (f64, f64, f64, f64) {
        (
            corners.iter().map(|point| point.0).fold(f64::MAX, f64::min),
            corners.iter().map(|point| point.0).fold(f64::MIN, f64::max),
            corners.iter().map(|point| point.1).fold(f64::MAX, f64::min),
            corners.iter().map(|point| point.1).fold(f64::MIN, f64::max),
        )
    }

    #[test]
    fn detects_document_despite_noise() {
        let width = 640usize;
        let height = 480usize;
        let mut data = canvas(width, height, [20, 20, 20, 255]);
        fill_rect(&mut data, width, 120, 90, 520, 390, [250, 250, 250, 255]);
        for index in 0..width * height {
            if index % 17 == 0 {
                let offset = index * 4;
                data[offset..offset + 4].copy_from_slice(&[240, 240, 240, 255]);
            }
        }
        let corners = detect_document_corners(width as u32, height as u32, &data).expect("detect");
        let (min_x, max_x, min_y, max_y) = min_max(&corners);
        assert!(min_x <= 124.0, "min_x={min_x}");
        assert!(max_x >= 516.0, "max_x={max_x}");
        assert!(min_y <= 94.0, "min_y={min_y}");
        assert!(max_y >= 386.0, "max_y={max_y}");
    }

    #[test]
    fn ignores_uniform_background() {
        let data = canvas(100, 100, [255, 255, 255, 255]);
        assert!(detect_document_corners(100, 100, &data).is_none());
    }

    #[test]
    fn detects_bright_paper_on_wood() {
        let width = 640usize;
        let height = 480usize;
        let mut data = canvas(width, height, [120, 95, 60, 255]);
        fill_rect(&mut data, width, 150, 90, 520, 400, [245, 245, 240, 255]);
        fill_rect(&mut data, width, 0, 0, 120, 90, [30, 30, 30, 255]);
        let corners = detect_document_corners(width as u32, height as u32, &data).expect("detect");
        let (min_x, max_x, min_y, max_y) = min_max(&corners);
        assert!(min_x <= 154.0, "min_x={min_x}");
        assert!(max_x >= 516.0, "max_x={max_x}");
        assert!(min_y <= 94.0, "min_y={min_y}");
        assert!(max_y >= 396.0, "max_y={max_y}");
    }

    #[test]
    fn detects_paper_nearly_filling_frame() {
        let width = 800usize;
        let height = 600usize;
        let mut data = canvas(width, height, [70, 60, 50, 255]);
        fill_rect(&mut data, width, 10, 8, 790, 592, [250, 250, 250, 255]);
        let corners = detect_document_corners(width as u32, height as u32, &data).expect("detect");
        let expected = [(10.0, 8.0), (789.0, 8.0), (789.0, 591.0), (10.0, 591.0)];
        let error = corner_error(&corners, &expected);
        assert!(error <= 6.0, "corners={corners:?} error={error}");
    }

    #[test]
    fn detects_rotated_paper_nearly_filling_frame() {
        let width = 800usize;
        let height = 600usize;
        let mut data = canvas(width, height, [70, 60, 50, 255]);
        let angle = 4.0f64.to_radians();
        let (sin, cos) = angle.sin_cos();
        let (cx, cy) = (400.0f64, 300.0f64);
        let (half_width, half_height) = (370.0f64, 270.0f64);
        let local = [
            (-half_width, -half_height),
            (half_width, -half_height),
            (half_width, half_height),
            (-half_width, half_height),
        ];
        let expected = local.map(|(x, y)| (cx + x * cos - y * sin, cy + x * sin + y * cos));
        fill_polygon(&mut data, width, &expected);
        let corners = detect_document_corners(width as u32, height as u32, &data).expect("detect");
        let error = corner_error(&corners, &expected);
        assert!(
            error <= 10.0,
            "corners={corners:?} expected={expected:?} error={error}"
        );
    }

    #[test]
    fn detects_noisy_rotated_paper_filling_the_photo() {
        let width = 1600usize;
        let height = 1200usize;
        let mut data = canvas(width, height, [0, 0, 0, 255]);
        let mut seed = 0x2545_f491_u32;
        for y in 0..height {
            for x in 0..width {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = ((seed >> 24) % 9) as i32 - 4;
                let shade = (x * 20 / width) as i32;
                let offset = (y * width + x) * 4;
                let value = |base: i32| u8::try_from((base + shade + noise).clamp(0, 255)).unwrap();
                data[offset..offset + 4].copy_from_slice(&[value(65), value(90), value(110), 255]);
            }
        }
        let angle = 1.5f64.to_radians();
        let (sin, cos) = angle.sin_cos();
        let (cx, cy) = (800.0f64, 600.0f64);
        let (half_width, half_height) = (776.0f64, 570.0f64);
        let local = [
            (-half_width, -half_height),
            (half_width, -half_height),
            (half_width, half_height),
            (-half_width, half_height),
        ];
        let expected = local.map(|(x, y)| (cx + x * cos - y * sin, cy + x * sin + y * cos));
        for y in 0..height {
            for x in 0..width {
                if !point_in_polygon(x as f64 + 0.5, y as f64 + 0.5, &expected) {
                    continue;
                }
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = ((seed >> 24) % 7) as i32 - 3;
                let offset = (y * width + x) * 4;
                let value = |base: i32| u8::try_from((base + noise).clamp(0, 255)).unwrap();
                data[offset..offset + 4].copy_from_slice(&[
                    value(240),
                    value(244),
                    value(245),
                    255,
                ]);
            }
        }
        let corners = detect_document_corners(width as u32, height as u32, &data).expect("detect");
        let error = corner_error(&corners, &expected);
        assert!(
            error <= 12.0,
            "corners={corners:?} expected={expected:?} error={error}"
        );
    }

    #[test]
    fn isolates_paper_connected_to_bright_clutter() {
        let width = 600usize;
        let height = 400usize;
        let mut data = canvas(width, height, [120, 95, 60, 255]);
        fill_rect(&mut data, width, 150, 100, 450, 330, [245, 245, 245, 255]);
        fill_rect(&mut data, width, 480, 40, 580, 130, [245, 245, 245, 255]);
        fill_rect(&mut data, width, 450, 112, 480, 118, [245, 245, 245, 255]);
        let corners = detect_document_corners(width as u32, height as u32, &data).expect("detect");
        let (min_x, max_x, min_y, max_y) = min_max(&corners);
        assert!((140.0..=165.0).contains(&min_x), "min_x={min_x}");
        assert!((435.0..=465.0).contains(&max_x), "max_x={max_x}");
        assert!((90.0..=115.0).contains(&min_y), "min_y={min_y}");
        assert!((315.0..=340.0).contains(&max_y), "max_y={max_y}");
    }

    #[test]
    fn detects_white_page_on_light_desk_with_dark_content() {
        let width = 640usize;
        let height = 480usize;
        let mut data = canvas(width, height, [205, 203, 200, 255]);
        fill_rect(&mut data, width, 90, 60, 560, 420, [247, 247, 245, 255]);
        // 页面里有一大块深色照片/标题，不能因此把页面拆成多个候选。
        fill_rect(&mut data, width, 150, 130, 330, 290, [40, 45, 60, 255]);
        fill_rect(&mut data, width, 150, 310, 500, 340, [55, 60, 70, 255]);
        let corners = detect_document_corners(width as u32, height as u32, &data).expect("detect");
        let expected = [(90.0, 60.0), (559.0, 60.0), (559.0, 419.0), (90.0, 419.0)];
        let error = corner_error(&corners, &expected);
        assert!(error <= 8.0, "corners={corners:?} error={error}");
    }

    #[test]
    fn detects_page_touching_frame_edges() {
        let width = 640usize;
        let height = 480usize;
        let mut data = canvas(width, height, [90, 80, 70, 255]);
        fill_rect(&mut data, width, 0, 0, 640, 440, [248, 248, 246, 255]);
        let corners = detect_document_corners(width as u32, height as u32, &data).expect("detect");
        let expected = [(0.0, 0.0), (639.0, 0.0), (639.0, 439.0), (0.0, 439.0)];
        let error = corner_error(&corners, &expected);
        assert!(error <= 8.0, "corners={corners:?} error={error}");
    }

    #[test]
    fn ignores_textured_desk() {
        let width = 640usize;
        let height = 480usize;
        let mut data = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for x in 0..width {
                let ripple = ((x as f32 * 0.33).sin() * 34.0) as i32;
                let grain = (y as i32 * 7 % 13) - 6;
                let value = u8::try_from((108 + ripple + grain).clamp(0, 255)).unwrap();
                data.extend_from_slice(&[
                    value,
                    value.saturating_sub(8),
                    value.saturating_sub(18),
                    255,
                ]);
            }
        }
        assert!(detect_document_corners(width as u32, height as u32, &data).is_none());
    }

    #[test]
    fn detects_trapezoid_on_light_desk() {
        let width = 800usize;
        let height = 600usize;
        let mut data = canvas(width, height, [190, 188, 185, 255]);
        let expected = [
            (180.0f64, 80.0f64),
            (600.0, 110.0),
            (650.0, 520.0),
            (140.0, 500.0),
        ];
        fill_polygon(&mut data, width, &expected);
        // 页面内的深色内容 + 传感器噪声。
        fill_rect(&mut data, width, 260, 200, 480, 380, [50, 55, 65, 255]);
        let mut seed = 0x9e37_79b9_u32;
        for pixel in data.chunks_exact_mut(4) {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = ((seed >> 25) % 7) as i32 - 3;
            for channel in 0..3 {
                pixel[channel] =
                    u8::try_from((i32::from(pixel[channel]) + noise).clamp(0, 255)).unwrap();
            }
        }
        let corners = detect_document_corners(width as u32, height as u32, &data).expect("detect");
        let error = corner_error(&corners, &expected);
        assert!(
            error <= 14.0,
            "corners={corners:?} expected={expected:?} error={error}"
        );
    }

    #[test]
    fn detects_low_contrast_rotated_page() {
        let width = 720usize;
        let height = 540usize;
        let mut data = canvas(width, height, [200, 198, 195, 255]);
        let angle = 10.0f64.to_radians();
        let (sin, cos) = angle.sin_cos();
        let (cx, cy) = (360.0f64, 270.0f64);
        let (half_width, half_height) = (250.0f64, 190.0f64);
        let local = [
            (-half_width, -half_height),
            (half_width, -half_height),
            (half_width, half_height),
            (-half_width, half_height),
        ];
        let expected = local.map(|(x, y)| (cx + x * cos - y * sin, cy + x * sin + y * cos));
        // 对比度只有约 28 的浅色纸张。
        for y in 0..height {
            for x in 0..width {
                if !point_in_polygon(x as f64 + 0.5, y as f64 + 0.5, &expected) {
                    continue;
                }
                let offset = (y * width + x) * 4;
                data[offset..offset + 4].copy_from_slice(&[228, 228, 225, 255]);
            }
        }
        let corners = detect_document_corners(width as u32, height as u32, &data).expect("detect");
        let error = corner_error(&corners, &expected);
        assert!(
            error <= 10.0,
            "corners={corners:?} expected={expected:?} error={error}"
        );
    }

    #[test]
    fn deskew_estimates_rotated_text_lines() {
        let width = 640u32;
        let height = 480u32;
        let mut data = canvas(width as usize, height as usize, [250, 250, 250, 255]);
        let angle = 7.0f64.to_radians();
        let (sin, cos) = angle.sin_cos();
        let (cx, cy) = (320.0f64, 240.0f64);
        for line in -4i32..=4 {
            let offset = f64::from(line) * 28.0;
            for t in 0..=280 {
                let local_x = -140.0 + f64::from(t);
                for local_y in 0..6 {
                    let ly = offset + f64::from(local_y);
                    let x = cx + local_x * cos - ly * sin;
                    let y = cy + local_x * sin + ly * cos;
                    if x < 0.0 || y < 0.0 || x >= f64::from(width) || y >= f64::from(height) {
                        continue;
                    }
                    let offset = (y as usize * width as usize + x as usize) * 4;
                    data[offset..offset + 4].copy_from_slice(&[30, 30, 30, 255]);
                }
            }
        }
        let angle = estimate_deskew(width, height, &data).expect("skew");
        assert!(
            (angle + 7.0).abs() < 1.5,
            "expected correction around -7°, got {angle}"
        );
    }
}
