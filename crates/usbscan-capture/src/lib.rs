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
mod mp4;

pub use error::CaptureError;
pub use frame::{Frame, PixelFormat};
pub use mf::{ensure_platform, guid_eq, init_thread, CameraInfo, ComGuard, MfCamera};
pub use mp4::Mp4Writer;

/// 统一摄像头后端。
pub enum Camera {
    MediaFoundation(MfCamera),
    DirectShow(dshow::DShowCamera),
}

impl Camera {
    /// 打开摄像头：先尝试 MF，失败后自动降级到 DirectShow。
    pub fn open(info: &CameraInfo, preferred: Option<(u32, u32)>) -> Result<Self, CaptureError> {
        if info.id.starts_with("dshow:") {
            return dshow::DShowCamera::open(info, preferred).map(Self::DirectShow);
        }
        match MfCamera::open(info, preferred) {
            Ok(camera) => Ok(Self::MediaFoundation(camera)),
            Err(mf_error) => match dshow::DShowCamera::open(info, preferred) {
                Ok(camera) => Ok(Self::DirectShow(camera)),
                Err(dshow_error) => Err(CaptureError::BackendFailure {
                    media_foundation: Box::new(mf_error),
                    direct_show: Box::new(dshow_error),
                }),
            },
        }
    }

    /// 当前设备原生支持的分辨率（去重、按面积降序）。
    #[must_use]
    pub fn formats(&self) -> Vec<(u32, u32)> {
        match self {
            Self::MediaFoundation(camera) => camera.formats().to_vec(),
            Self::DirectShow(camera) => camera.formats(),
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
/// 同一个物理设备可能同时出现在两套枚举结果中；优先保留 MF 条目。
pub fn list_cameras() -> Result<Vec<CameraInfo>, CaptureError> {
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

/// 从设备符号链接/显示名中提取稳定的 PNP 设备键。
///
/// MF 使用 `\\?\usb#vid_...`，DirectShow 使用
/// `@device:pnp:\\?\usb#vid_...`；去掉前缀和接口 GUID 后可以直接比较。
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

/// 进程级初始化：确保 MF 已就绪。
pub fn ensure_backends() -> Result<(), CaptureError> {
    ensure_platform()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn pnp_key_ignores_virtual_devices() {
        assert_eq!(
            pnp_key(r"@device:pnp:\\?\usb#vid_1234#5&abc&0&1#{guid}"),
            Some("usb#vid_1234#5&abc&0&1".to_string())
        );
        assert!(pnp_key("@device:sw:{860BB310-5D01-11D0-BD3B-00A0C911CE86}\\{A3FCE0F5}").is_none());
    }
}
