// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! `usbscan` 摄像头采集抽象层。
//!
//! **Windows**：Media Foundation 是主后端；DirectShow 用于 MF 枚举/打开失败时的兜底。
//!
//! **Linux / macOS**：使用 `nokhwa` 统一后端（Linux: V4L2；macOS: AVFoundation）。
//!
//! 所有后端都输出统一的 top-down BGRA8 帧。

#![allow(clippy::doc_markdown)]

mod error;
mod frame;

#[cfg(windows)]
mod dshow;
#[cfg(windows)]
mod mf;
#[cfg(windows)]
mod mp4;

#[cfg(not(windows))]
mod unix;

pub use error::CaptureError;
pub use frame::{Frame, PixelFormat};

// --- Windows 专属重导出 ---
#[cfg(windows)]
pub use mf::{ensure_platform, guid_eq, init_thread, CameraInfo, ComGuard, MfCamera};
#[cfg(windows)]
pub use mp4::Mp4Writer;

// --- Unix 专属重导出 ---
#[cfg(not(windows))]
pub use mf_stub::CameraInfo;
#[cfg(not(windows))]
pub use unix::{ensure_platform, init_thread, ComGuard};

// --- 非 Windows 下 CameraInfo 存根（结构与 Windows 版保持一致）---
#[cfg(not(windows))]
mod mf_stub {
    /// 一台摄像头的基本信息。
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct CameraInfo {
        /// 稳定标识（如 `"unix:0"`）。
        pub id: String,
        /// 系统显示的友好名。
        pub name: String,
    }
}

// ---------------------------------------------------------------------------
// 统一摄像头后端枚举
// ---------------------------------------------------------------------------

/// 统一摄像头后端。
pub enum Camera {
    #[cfg(windows)]
    MediaFoundation(mf::MfCamera),
    #[cfg(windows)]
    DirectShow(dshow::DShowCamera),
    #[cfg(not(windows))]
    Unix(unix::UnixCamera),
}

impl Camera {
    /// 打开摄像头。
    ///
    /// 所有后端都请求设备的原生最高分辨率；照片和录像的输出尺寸由上层缩放。
    /// `_preferred` 仅为兼容旧调用保留，不再影响采集分辨率。
    ///
    /// **Windows**：先尝试 MF，失败后自动降级到 DirectShow。
    ///
    /// **Linux / macOS**：通过 nokhwa 后端（V4L2 / AVFoundation）直接打开。
    pub fn open(info: &CameraInfo, _preferred: Option<(u32, u32)>) -> Result<Self, CaptureError> {
        #[cfg(windows)]
        {
            if info.id.starts_with("dshow:") {
                return dshow::DShowCamera::open(info).map(Self::DirectShow);
            }
            match mf::MfCamera::open(info) {
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
        #[cfg(not(windows))]
        {
            unix::UnixCamera::open(info).map(Self::Unix)
        }
    }

    /// 当前设备原生支持的分辨率（去重、按面积降序）。
    #[must_use]
    pub fn formats(&self) -> Vec<(u32, u32)> {
        match self {
            #[cfg(windows)]
            Self::MediaFoundation(camera) => camera.formats().to_vec(),
            #[cfg(windows)]
            Self::DirectShow(camera) => camera.formats(),
            #[cfg(not(windows))]
            Self::Unix(camera) => camera.formats(),
        }
    }

    /// 当前设备在原生最高分辨率下支持的帧率（去重、升序）。
    #[must_use]
    pub fn fps_rates(&self) -> Vec<u32> {
        match self {
            #[cfg(windows)]
            Self::MediaFoundation(camera) => camera.fps_rates(),
            #[cfg(windows)]
            Self::DirectShow(camera) => camera.fps_rates(),
            #[cfg(not(windows))]
            Self::Unix(camera) => camera.fps_rates(),
        }
    }

    #[must_use]
    pub fn info(&self) -> &CameraInfo {
        match self {
            #[cfg(windows)]
            Self::MediaFoundation(camera) => camera.info(),
            #[cfg(windows)]
            Self::DirectShow(camera) => camera.info(),
            #[cfg(not(windows))]
            Self::Unix(camera) => camera.info(),
        }
    }

    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        match self {
            #[cfg(windows)]
            Self::MediaFoundation(camera) => camera.size(),
            #[cfg(windows)]
            Self::DirectShow(camera) => camera.size(),
            #[cfg(not(windows))]
            Self::Unix(camera) => camera.size(),
        }
    }

    /// 阻塞读取下一帧。
    pub fn read_frame(&mut self) -> Result<Frame, CaptureError> {
        match self {
            #[cfg(windows)]
            Self::MediaFoundation(camera) => camera.read_frame(),
            #[cfg(windows)]
            Self::DirectShow(camera) => camera.read_frame(),
            #[cfg(not(windows))]
            Self::Unix(camera) => camera.read_frame(),
        }
    }

    /// 当前实际使用的后端名称。
    #[must_use]
    pub const fn backend(&self) -> &'static str {
        match self {
            #[cfg(windows)]
            Self::MediaFoundation(_) => "Media Foundation",
            #[cfg(windows)]
            Self::DirectShow(_) => "DirectShow",
            #[cfg(not(windows))]
            Self::Unix(_) => "nokhwa",
        }
    }
}

// ---------------------------------------------------------------------------
// 枚举设备
// ---------------------------------------------------------------------------

/// 枚举系统中所有视频采集设备。
///
/// **Windows**：合并 MF 与 DirectShow 的设备列表（优先保留 MF 条目）。
///
/// **Linux / macOS**：通过 nokhwa 枚举 V4L2 / AVFoundation 设备。
pub fn list_cameras() -> Result<Vec<CameraInfo>, CaptureError> {
    #[cfg(windows)]
    {
        let mut cameras = mf::list_cameras().unwrap_or_default();
        let dshow_cameras = dshow::list_cameras().unwrap_or_default();
        for camera in dshow_cameras {
            let key = pnp_key(&camera.name);
            let duplicate = cameras.iter().any(|existing| {
                existing.name == camera.name
                    || existing.id == camera.id
                    || key
                        .as_ref()
                        .is_some_and(|key| pnp_key(&existing.id).as_ref() == Some(key))
            });
            if !duplicate {
                cameras.push(camera);
            }
        }
        Ok(cameras)
    }
    #[cfg(not(windows))]
    {
        unix::list_cameras()
    }
}

/// 进程级初始化：确保采集后端已就绪。
pub fn ensure_backends() -> Result<(), CaptureError> {
    ensure_platform()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Windows 专属：PNP 键提取（仅 Windows 编译）
// ---------------------------------------------------------------------------

/// 从设备符号链接/显示名中提取稳定的 PNP 设备键。（仅 Windows）
#[cfg(windows)]
pub(crate) fn pnp_key(value: &str) -> Option<String> {
    let value = value
        .split_once("@device:pnp:")
        .map_or(value, |(_, rest)| rest);
    if value.contains("@device:sw:") {
        return None;
    }
    let value = value.strip_prefix(r"\\?\").unwrap_or(value);
    let end = value.find("#{").unwrap_or(value.len());
    let key = &value[..end];
    if key.is_empty() {
        None
    } else {
        Some(key.to_ascii_lowercase())
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::*;

    #[cfg(windows)]
    #[test]
    fn pnp_key_matches_media_foundation_and_direct_show_forms() {
        let mf = r"\\?\usb#vid_cb07&pid_1bcf&mi_00#a&3a4e7912&0&0000#{e5323777-f976-4f5b-9b55-b94699c46e44}\global";
        let dshow = r"@device:pnp:\\?\usb#vid_cb07&pid_1bcf&mi_00#a&3a4e7912&0&0000#{65e8773d-8f56-11d0-a3b9-00a0c9223196}\global";
        assert_eq!(pnp_key(mf), pnp_key(dshow));
        assert_eq!(
            pnp_key(mf).as_deref(),
            Some("usb#vid_cb07&pid_1bcf&mi_00#a&3a4e7912&0&0000")
        );
    }

    #[cfg(windows)]
    #[test]
    fn pnp_key_ignores_virtual_devices() {
        assert_eq!(
            pnp_key(r"@device:pnp:\\?\usb#vid_1234#5&abc&0&1#{guid}"),
            Some("usb#vid_1234#5&abc&0&1".to_string())
        );
        assert!(pnp_key("@device:sw:{860BB310-5D01-11D0-BD3B-00A0C911CE86}\\{A3FCE0F5}").is_none());
    }
}
