// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! Unix 摄像头采集后端（Linux V4L2 / macOS AVFoundation）。
//!
//! 通过 `nokhwa` crate 统一包装，向上层提供与 Windows 后端相同的接口：
//! - `list_cameras()` → `Vec<CameraInfo>`
//! - `UnixCamera::open()` → 阻塞读帧 `read_frame() → Frame`
//!
//! 输出格式统一为 top-down BGRA8。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use nokhwa::pixel_format::RgbAFormat;
use nokhwa::utils::{
    ApiBackend, CameraIndex, CameraInfo as NokhwaCameraInfo, RequestedFormat,
    RequestedFormatType, Resolution,
};
use nokhwa::Camera as NokhwaCamera;

use crate::error::CaptureError;
use crate::frame::{Frame, PixelFormat};
use crate::mf::CameraInfo;

// ---------------------------------------------------------------------------
// 枚举设备
// ---------------------------------------------------------------------------

/// 枚举系统中所有视频采集设备。
pub fn list_cameras() -> Result<Vec<CameraInfo>, CaptureError> {
    let backend = preferred_backend();
    let devices = nokhwa::query(backend)
        .map_err(|err| CaptureError::Backend(err.to_string()))?;
    Ok(devices.into_iter().map(nokhwa_info_to_camera_info).collect())
}

fn nokhwa_info_to_camera_info(info: NokhwaCameraInfo) -> CameraInfo {
    let id = format!("unix:{}", info.index());
    let name = info.human_name().to_string();
    CameraInfo { id, name }
}

/// 把 `CameraInfo.id`（如 `"unix:0"` 或 `"unix:/dev/video0"`）还原为 nokhwa `CameraIndex`。
fn parse_index(id: &str) -> Option<CameraIndex> {
    let idx_str = id.strip_prefix("unix:")?;
    if let Ok(idx) = idx_str.parse::<u32>() {
        Some(CameraIndex::Index(idx))
    } else {
        Some(CameraIndex::String(idx_str.to_string()))
    }
}

// ---------------------------------------------------------------------------
// 摄像头句柄
// ---------------------------------------------------------------------------

/// Unix 摄像头：持有 nokhwa 相机实例，并缓存当前分辨率。
pub struct UnixCamera {
    inner: Arc<Mutex<NokhwaCamera>>,
    info: CameraInfo,
    width: u32,
    height: u32,
}

impl UnixCamera {
    /// 打开摄像头。`preferred` 为期望分辨率，不支持时退回到设备默认值。
    pub fn open(info: &CameraInfo, preferred: Option<(u32, u32)>) -> Result<Self, CaptureError> {
        let index = parse_index(&info.id).ok_or_else(|| {
            CaptureError::DeviceNotFound(format!("无法解析摄像头 ID：{}", info.id))
        })?;

        let requested = preferred.map_or(
            RequestedFormat::new::<RgbAFormat>(RequestedFormatType::AbsoluteHighestFrameRate),
            |(w, h)| {
                RequestedFormat::new::<RgbAFormat>(RequestedFormatType::Closest(
                    nokhwa::utils::CameraFormat::new(
                        Resolution::new(w, h),
                        nokhwa::utils::FrameFormat::RAWRGB,
                        30,
                    ),
                ))
            },
        );

        let mut camera = NokhwaCamera::new(index, requested)
            .map_err(|err| CaptureError::Backend(err.to_string()))?;

        camera
            .open_stream()
            .map_err(|err| CaptureError::Backend(err.to_string()))?;

        let res = camera.resolution();
        let width = res.width_x;
        let height = res.height_y;

        Ok(Self {
            inner: Arc::new(Mutex::new(camera)),
            info: info.clone(),
            width,
            height,
        })
    }

    /// 当前设备支持的分辨率列表（去重，按面积降序）。
    pub fn formats(&self) -> Vec<(u32, u32)> {
        let camera = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Ok(formats) = camera.compatible_camera_formats() else {
            return vec![(self.width, self.height)];
        };
        let mut seen = Vec::new();
        let mut res_list: Vec<(u32, u32)> = formats
            .into_iter()
            .map(|f| (f.resolution().width_x, f.resolution().height_y))
            .filter(|r| {
                if seen.contains(r) {
                    false
                } else {
                    seen.push(*r);
                    true
                }
            })
            .collect();
        res_list.sort_by_key(|&(w, h)| std::cmp::Reverse(w * h));
        res_list
    }

    pub fn info(&self) -> &CameraInfo {
        &self.info
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// 阻塞读取下一帧（BGRA8，top-down）。
    pub fn read_frame(&mut self) -> Result<Frame, CaptureError> {
        let frame = {
            let mut camera = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            camera
                .frame()
                .map_err(|err| CaptureError::Backend(err.to_string()))?
        };

        let (width, height) = (frame.width(), frame.height());
        // nokhwa 解码为 RGBA；我们需要 BGRA（蓝/红通道互换）。
        let rgba_buf: Vec<u8> = frame
            .decode_image::<RgbAFormat>()
            .map_err(|err| CaptureError::Backend(err.to_string()))?
            .into_raw();

        let mut bgra = rgba_buf;
        // RGBA → BGRA：交换 R(0) 与 B(2)
        for chunk in bgra.chunks_exact_mut(4) {
            chunk.swap(0, 2);
        }

        Ok(Frame {
            width,
            height,
            format: PixelFormat::Bgra8,
            data: bgra,
            timestamp: 0,
        })
    }
}

// ---------------------------------------------------------------------------
// 辅助
// ---------------------------------------------------------------------------

/// 选择当前平台最合适的 nokhwa 后端。
fn preferred_backend() -> ApiBackend {
    #[cfg(target_os = "linux")]
    {
        ApiBackend::Video4Linux
    }
    #[cfg(target_os = "macos")]
    {
        ApiBackend::AVFoundation
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        ApiBackend::Auto
    }
}

// ---------------------------------------------------------------------------
// COM / 平台初始化占位（非 Windows 时为空操作）
// ---------------------------------------------------------------------------

/// Unix 下不需要 COM 初始化，返回一个空守卫。
pub struct ComGuard;

pub fn init_thread() -> ComGuard {
    ComGuard
}

pub fn ensure_platform() -> Result<(), CaptureError> {
    Ok(())
}
