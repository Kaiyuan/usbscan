// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! usbscan 的 Win32 封装层。
//!
//! 设计原则（见 `docs/02-架构设计.md`）：
//!
//! - 所有界面元素使用系统原生控件，不自绘控件外观；
//! - 自绘仅限预览画布与叠加层；
//! - 所有尺寸以 96 DPI 为基准，运行时按窗口 DPI 缩放。

// Win32 句柄本身就是裸指针，封装层把这些参数视为不透明句柄。
// 这些 lint 对 FFI 包装层是噪音，统一在 crate 级别关闭。
#![allow(
    clippy::not_unsafe_ptr_arg_deref,
    clippy::borrow_as_ptr,
    clippy::ptr_as_ptr,
    clippy::unnecessary_cast
)]

pub mod app;
pub mod capture_host;
pub mod config;
pub mod controls;
pub mod dpi;
pub mod error;
pub mod icons;
pub mod menu;
pub mod text;
pub mod theme;

pub use app::run;
pub use error::WinError;
