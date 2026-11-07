// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 采集错误。

use std::fmt;

use windows::core::GUID;

/// 摄像头采集失败的原因。
#[derive(Debug)]
pub enum CaptureError {
    /// Windows / Media Foundation 返回的错误。
    Windows(windows::core::Error),
    /// 直接携带 HRESULT（用于延迟初始化）。
    Hresult(i32),
    /// 在某个具体步骤失败，保留上下文便于定位。
    Stage {
        step: &'static str,
        source: Box<Self>,
    },
    /// Media Foundation 与 DirectShow 均未成功打开设备。
    BackendFailure {
        media_foundation: Box<Self>,
        direct_show: Box<Self>,
    },
    /// 系统中没有任何视频采集设备。
    NoDevices,
    /// 设备已断开或无法再次打开。
    DeviceNotFound(String),
    /// 摄像头无法输出 RGB32。
    UnsupportedFormat(GUID),
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
            Self::Windows(err) => {
                // 把这些常见 HRESULT 翻译成用户能直接处理的中文提示。
                match err.code().0 as u32 {
                    0x8007_0005 => write!(
                        f,
                        "相机访问被拒绝：请在“设置 → 隐私和安全性 → 相机”中允许桌面应用访问"
                    ),
                    0xC00D_3704 => write!(
                        f,
                        "摄像头无法启动：设备可能被其他程序占用，或驱动不支持所需格式"
                    ),
                    0xC00D_3EA3 => write!(
                        f,
                        "摄像头已被其他程序抢占，请关闭占用摄像头的软件后按 F5 重试"
                    ),
                    0x8007_0057 => write!(f, "参数错误（0x80070057）：摄像头驱动拒绝了请求的格式"),
                    0x8000_4002 => write!(
                        f,
                        "该摄像头不支持当前采集接口（可能仅支持 Windows 相机框架或厂商 SDK）"
                    ),
                    _ => write!(f, "{err}"),
                }
            }
            Self::Hresult(code) => write!(f, "Media Foundation 错误 0x{code:08X}"),
            Self::Stage { step, source } => write!(f, "{step}失败：{source}"),
            Self::BackendFailure {
                media_foundation,
                direct_show,
            } => write!(
                f,
                "Media Foundation 与 DirectShow 均无法打开摄像头（MF：{media_foundation}；DShow：{direct_show}）"
            ),
            Self::NoDevices => write!(f, "未检测到摄像头"),
            Self::DeviceNotFound(name) => write!(f, "摄像头不可用：{name}"),
            Self::UnsupportedFormat(subtype) => {
                write!(f, "摄像头无法输出 RGB32（实际格式 {subtype:?}）")
            }
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
            Self::Stage { source, .. } => Some(source.as_ref()),
            Self::BackendFailure {
                media_foundation, ..
            } => Some(media_foundation.as_ref()),
            _ => None,
        }
    }
}

impl From<windows::core::Error> for CaptureError {
    fn from(err: windows::core::Error) -> Self {
        Self::Windows(err)
    }
}
