// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! `usbscan` 摄像头采集抽象层。
//!
//! 当前提供 Media Foundation 后端（Win10 1607+ / Win11 自带）。
//! `DirectShow` 兜底后端在 M1-3 加入，接口保持不变。

mod error;
mod frame;
mod mf;

pub use error::CaptureError;
pub use frame::{Frame, PixelFormat};
pub use mf::{ensure_platform, guid_eq, init_thread, list_cameras, CameraInfo, ComGuard, MfCamera};
