// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! usbscan 的 Win32 封装层。
//!
//! 设计原则（见 `docs/02-架构设计.md`）：
//!
//! - 所有界面元素使用系统原生控件，不自绘控件外观；
//! - 自绘仅限预览画布与叠加层；
//! - 所有尺寸以 96 DPI 为基准，运行时按窗口 DPI 缩放。
//!
//! **非 Windows 平台**：本 crate 编译为空壳，仅导出跨平台必需的接口存根
//! （`paths` 和 `config`），UI 完全由 Slint（`usbscan-app`）驱动。

// Win32 句柄本身就是裸指针，封装层把这些参数视为不透明句柄。
#![cfg_attr(
    windows,
    allow(
        clippy::not_unsafe_ptr_arg_deref,
        clippy::borrow_as_ptr,
        clippy::ptr_as_ptr,
        clippy::unnecessary_cast
    )
)]

// ---------------------------------------------------------------------------
// Windows：完整 Win32 UI 层
// ---------------------------------------------------------------------------

#[cfg(windows)]
pub mod annotate;
#[cfg(windows)]
pub mod app;
#[cfg(windows)]
pub mod capture_host;
pub mod config;
#[cfg(windows)]
pub mod controls;
#[cfg(windows)]
pub mod corners;
#[cfg(windows)]
pub mod crop;
#[cfg(windows)]
pub mod dpi;
#[cfg(windows)]
pub mod error;
#[cfg(windows)]
pub mod export_dialog;
#[cfg(windows)]
pub mod icons;
#[cfg(windows)]
pub mod label;
#[cfg(windows)]
pub mod menu;
#[cfg(windows)]
pub mod menubar;
#[cfg(windows)]
pub mod ocr_viewer;
pub mod page_menu;
pub mod paths;
#[cfg(windows)]
pub mod print;
#[cfg(windows)]
pub mod print_preview;
pub mod recorder;
#[cfg(windows)]
pub mod settings_dialog;
pub mod shell;
#[cfg(windows)]
pub mod text;
#[cfg(windows)]
pub mod text_edit;
pub mod theme;
#[cfg(windows)]
pub mod toolbar;
pub mod window;

#[cfg(windows)]
pub use app::run;
#[cfg(windows)]
pub use error::WinError;

// ---------------------------------------------------------------------------
// 非 Windows：Slint 路径（run() 存根，UI 由 usbscan-app 的 Slint 代码直接驱动）
// ---------------------------------------------------------------------------

/// 非 Windows 平台上的应用入口存根。
///
/// 实际 UI 完全在 `usbscan-app/src/main.rs` 中通过 Slint 驱动，
/// 此函数仅供编译检查，永远不应被调用。
#[cfg(not(windows))]
pub fn run() -> Result<(), String> {
    unreachable!("非 Windows 平台不使用 usbscan-win::run()")
}
