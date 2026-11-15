// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 把 UI 使用的 Tabler SVG 图标预渲染成 PNG。
//!
//! 运行：`cargo run --example render_icons -p usbscan-app`
//! 输出到 `assets/icons/png/`。UI 只引用 PNG，避免在部分 Linux 桌面环境中
//! SVG 解码失败导致图标空白。

use std::fs;
use std::path::Path;

const SIZE: u32 = 96;
/// 烘焙到 PNG 的中性灰：即使某些渲染后端忽略 `colorize`，图标仍然可见。
const FALLBACK_COLOR: &str = "#8a8f98";

#[allow(clippy::cast_precision_loss)]
fn main() {
    let icons = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons");
    let svg_dir = icons.join("svg");
    let out_dir = icons.join("png");
    fs::create_dir_all(&out_dir).expect("create png output directory");

    let mut entries: Vec<_> = fs::read_dir(&svg_dir)
        .expect("read svg directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("svg"))
        .collect();
    entries.sort();

    for path in entries {
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .expect("svg file name");
        // 为特殊图标烘焙接近实际用途的颜色：相机在蓝色主按钮上，停止键和
        // 更新提示在状态栏中。即使后端忽略 colorize，这些图标仍然清晰。
        let fallback = match name {
            "camera" => "#ffffff",
            "player-stop" => "#ff453a",
            "alert-triangle" => "#f5b800",
            _ => FALLBACK_COLOR,
        };
        let svg = fs::read_to_string(&path)
            .expect("read svg")
            .replace("currentColor", fallback);
        // 不加载系统字体：图标本身不包含文字。
        let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default())
            .unwrap_or_else(|err| panic!("parse {}: {err}", path.display()));
        let size = tree.size();
        let scale = SIZE as f32 / size.width().max(size.height());
        let mut pixmap = resvg::tiny_skia::Pixmap::new(SIZE, SIZE).expect("create pixmap");
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
        let target = out_dir.join(format!("{name}.png"));
        pixmap.save_png(&target).expect("save png");
        println!("{}", target.display());
    }
}
