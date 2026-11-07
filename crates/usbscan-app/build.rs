// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 把 `app.manifest` 嵌入可执行文件。
//!
//! 该清单提供三件关键的事：
//! 1. 启用 Common Controls v6 —— 控件才有现代外观；
//! 2. 声明 Per-Monitor V2 DPI 感知；
//! 3. 声明 longPathAware。

use std::path::Path;

fn main() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("app.manifest");
    if !manifest.exists() {
        println!("cargo:warning=app.manifest 不存在，跳过清单嵌入");
        return;
    }

    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
}
