// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! `usbscan` 摄像头采集抽象层。
//!
//! Media Foundation 是主后端；DirectShow 用于 MF 枚举/打开失败时的兜底。
//! 两个后端都输出统一的 top-down BGRA8 帧。

#![allow(clippy::doc_markdown)]

mod dshow;
mod error;
mod frame;
mod mf;

pub use error::CaptureError;
pub use frame::{Frame, PixelFormat};
pub use mf::{ensure_platform, guid_eq, init_thread, CameraInfo, ComGuard, MfCamera};

/// 统一摄像头后端。
pub enum Camera {
    MediaFoundation(MfCamera),
    DirectShow(dshow::DShowCamera),
}

impl Camera {
    /// 打开摄像头：先尝试 MF，失败后自动降级到 DirectShow。
    pub fn open(info: &CameraInfo) -> Result<Self, CaptureError> {
        if info.id.starts_with("dshow:") {
            return dshow::DShowCamera::open(info).map(Self::DirectShow);
        }
        match MfCamera::open(info) {
            Ok(camera) => Ok(Self::MediaFoundation(camera)),
            Err(mf_error) => match dshow::DShowCamera::open(info) {
                Ok(camera) => Ok(Self::DirectShow(camera)),
                Err(dshow_error) => Err(CaptureError::BackendFailure {
                    media_foundation: Box::new(mf_error),
                    direct_show: Box::new(dshow_error),
                }),
            },
        }
    }

    #[must_use]
    pub fn info(&self) -> &CameraInfo {
        match self {
            Self::MediaFoundation(camera) => camera.info(),
            Self::DirectShow(camera) => camera.info(),
        }
    }

    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        match self {
            Self::MediaFoundation(camera) => camera.size(),
            Self::DirectShow(camera) => camera.size(),
        }
    }

    /// 阻塞读取下一帧。
    pub fn read_frame(&mut self) -> Result<Frame, CaptureError> {
        match self {
            Self::MediaFoundation(camera) => camera.read_frame(),
            Self::DirectShow(camera) => camera.read_frame(),
        }
    }

    /// 当前实际使用的后端名称。
    #[must_use]
    pub const fn backend(&self) -> &'static str {
        match self {
            Self::MediaFoundation(_) => "Media Foundation",
            Self::DirectShow(_) => "DirectShow",
        }
    }
}

/// 合并 MF 与 DirectShow 的设备列表。
///
/// 同名设备只保留一条；MF 的稳定 ID 优先。
pub fn list_cameras() -> Result<Vec<CameraInfo>, CaptureError> {
    let mut cameras = mf::list_cameras().unwrap_or_default();
    let dshow_cameras = dshow::list_cameras().unwrap_or_default();
    for camera in dshow_cameras {
        if !cameras
            .iter()
            .any(|existing| existing.name == camera.name || existing.id == camera.id)
        {
            cameras.push(camera);
        }
    }
    Ok(cameras)
}

/// 进程级初始化：确保 MF 已就绪。
pub fn ensure_backends() -> Result<(), CaptureError> {
    ensure_platform()?;
    Ok(())
}
