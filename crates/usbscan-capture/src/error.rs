// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 采集错误。

use std::fmt;

/// 摄像头采集失败的原因。
#[derive(Debug)]
pub enum CaptureError {
    /// Windows / Media Foundation 返回的错误。
    Windows(windows::core::Error),
    /// 直接携带 HRESULT（用于延迟初始化）。
    Hresult(i32),
    /// 系统中没有任何视频采集设备。
    NoDevices,
    /// 设备已断开或无法再次打开。
    DeviceNotFound(String),
    /// 设备提前结束（通常意味着被拔出）。
    EndOfStream,
    /// 本次没有取到图像（格式切换中）。
    NoFrame,
    /// 帧数据尺寸与声明不符。
    BadFrame(String),
    /// 线程已请求停止。
    Stopped,
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Windows(err) => write!(f, "{err}"),
            Self::Hresult(code) => write!(f, "Media Foundation 错误 0x{code:08X}"),
            Self::NoDevices => write!(f, "未检测到摄像头"),
            Self::DeviceNotFound(name) => write!(f, "摄像头不可用：{name}"),
            Self::EndOfStream => write!(f, "采集流已结束"),
            Self::NoFrame => write!(f, "暂无图像"),
            Self::BadFrame(detail) => write!(f, "图像数据异常：{detail}"),
            Self::Stopped => write!(f, "采集已停止"),
        }
    }
}

impl std::error::Error for CaptureError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Windows(err) => Some(err),
            _ => None,
        }
    }
}

impl From<windows::core::Error> for CaptureError {
    fn from(err: windows::core::Error) -> Self {
        Self::Windows(err)
    }
}
