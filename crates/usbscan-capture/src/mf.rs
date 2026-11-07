// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! Media Foundation 采集后端。
//!
//! 只使用 Media Foundation（Win10 1607+ 自带），不依赖厂商 SDK。
//! 枚举、打开、取帧都在调用方线程执行；本模块内部不创建线程。

use std::ptr;

use windows::core::{GUID, PWSTR};
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, IMFAttributes, IMFMediaSource, IMFMediaType, IMFSample, IMFSourceReader,
    MFCreateAttributes, MFCreateMediaType, MFCreateSourceReaderFromMediaSource,
    MFEnumDeviceSources, MFMediaType_Video, MFVideoFormat_RGB32, MFVideoInterlace_Progressive,
    MFSTARTUP_LITE, MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, MF_MT_ALL_SAMPLES_INDEPENDENT,
    MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE,
    MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED, MF_SOURCE_READERF_ENDOFSTREAM,
    MF_SOURCE_READERF_ERROR, MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_VERSION,
};
use windows::Win32::System::Com::{
    CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_MULTITHREADED,
};

use crate::error::CaptureError;
use crate::frame::Frame;

/// 一台摄像头。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraInfo {
    /// 稳定标识：设备符号链接（无法取得时退化为 `名称#枚举序号`）。
    pub id: String,
    /// 系统显示的友好名。
    pub name: String,
}

/// 采集线程的 COM 初始化守卫。
///
/// 在采集线程上创建，析构时自动 `CoUninitialize`。
pub struct ComGuard {
    uninit: bool,
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.uninit {
            // SAFETY: 与成功/半成功的 CoInitializeEx 成对调用。
            unsafe { CoUninitialize() };
        }
    }
}

/// 为当前线程初始化 COM（多线程单元）。
#[must_use]
pub fn init_thread() -> ComGuard {
    // SAFETY: 标准 COM 初始化；失败时不做反初始化。
    let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    ComGuard { uninit: hr.is_ok() }
}

/// 进程级 Media Foundation 初始化；可重复调用。
pub fn ensure_platform() -> Result<(), CaptureError> {
    use std::sync::OnceLock;

    static PLATFORM: OnceLock<i32> = OnceLock::new();
    let code = *PLATFORM.get_or_init(|| {
        // SAFETY: MFStartup 是进程级引用计数初始化。
        match unsafe {
            windows::Win32::Media::MediaFoundation::MFStartup(MF_VERSION, MFSTARTUP_LITE)
        } {
            Ok(()) => 0,
            Err(err) => err.code().0,
        }
    });
    if code < 0 {
        Err(CaptureError::Hresult(code))
    } else {
        Ok(())
    }
}

/// 枚举所有视频采集设备。
pub fn list_cameras() -> Result<Vec<CameraInfo>, CaptureError> {
    let activates = enumerate_activates()?;
    let mut cameras = Vec::with_capacity(activates.len());
    for (index, act) in activates.iter().enumerate() {
        let name = friendly_name(act)?;
        // 优先使用符号链接：设备顺序变化或热插拔后仍能认得同一台摄像头。
        let id = symbolic_link(act).unwrap_or_else(|| format!("{name}#{index}"));
        cameras.push(CameraInfo { id, name });
    }
    Ok(cameras)
}

/// 已打开的摄像头，可循环取帧。
pub struct MfCamera {
    info: CameraInfo,
    reader: IMFSourceReader,
    width: u32,
    height: u32,
    stride: i32,
    /// 是否已跳过第一帧（传感器刚上电时常见暗帧）。
    warmup: bool,
}

impl MfCamera {
    /// 打开指定摄像头并请求 BGRA 输出。
    pub fn open(info: &CameraInfo) -> Result<Self, CaptureError> {
        ensure_platform()?;
        let activates = enumerate_activates()?;
        // 先按符号链接精确匹配；老设备或驱动不提供链接时按名称回退。
        let activate = activates
            .iter()
            .find(|act| symbolic_link(act).as_deref() == Some(info.id.as_str()))
            .or_else(|| {
                activates
                    .iter()
                    .find(|act| friendly_name(act).is_ok_and(|name| name == info.name))
            })
            .ok_or_else(|| CaptureError::DeviceNotFound(info.name.clone()))?;

        // SAFETY: activate 来自本函数的枚举结果，类型正确。
        unsafe {
            let source: IMFMediaSource = activate.ActivateObject()?;
            let reader = MFCreateSourceReaderFromMediaSource(&source, None::<&IMFAttributes>)?;

            reader.SetStreamSelection(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32, true)?;

            let media_type = MFCreateMediaType()?;
            media_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            media_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
            media_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            media_type.SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 1)?;
            reader.SetCurrentMediaType(
                MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
                None,
                &media_type,
            )?;

            let current =
                reader.GetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32)?;
            let (width, height) = frame_size(&current)?;
            let stride = current
                .GetUINT32(&MF_MT_DEFAULT_STRIDE)
                .map_or((width * 4) as i32, |v| v as i32);

            Ok(Self {
                info: info.clone(),
                reader,
                width,
                height,
                stride,
                warmup: false,
            })
        }
    }

    #[must_use]
    pub fn info(&self) -> &CameraInfo {
        &self.info
    }

    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// 阻塞读取下一帧。
    pub fn read_frame(&mut self) -> Result<Frame, CaptureError> {
        // 丢弃第一帧，避免某些模组刚打开时的全黑帧。
        if !self.warmup {
            self.warmup = true;
            let _ = self.read_frame_inner();
        }
        self.read_frame_inner()
    }

    fn read_frame_inner(&mut self) -> Result<Frame, CaptureError> {
        let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        let mut flags = 0u32;
        let mut timestamp = 0i64;
        let mut sample: Option<IMFSample> = None;

        // SAFETY: reader 有效；所有输出指针指向栈上变量。
        unsafe {
            self.reader.ReadSample(
                stream,
                0,
                None,
                Some(&raw mut flags),
                Some(&raw mut timestamp),
                Some(&raw mut sample),
            )?;
        }

        if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
            return Err(CaptureError::EndOfStream);
        }
        if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
            return Err(CaptureError::BadFrame("采集流报告错误".to_string()));
        }

        // 分辨率/格式变化时重新读取实际输出格式。
        if flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32 != 0 {
            // SAFETY: reader 有效。
            let current = unsafe { self.reader.GetCurrentMediaType(stream) }?;
            let (width, height) = frame_size(&current)?;
            self.width = width;
            self.height = height;
            self.stride = unsafe { current.GetUINT32(&MF_MT_DEFAULT_STRIDE) }
                .map_or((width * 4) as i32, |v| v as i32);
        }

        let sample = sample.ok_or(CaptureError::NoFrame)?;

        // SAFETY: sample 有效；Lock/Unlock 成对调用。
        unsafe {
            let buffer = sample.ConvertToContiguousBuffer()?;
            let mut data: *mut u8 = ptr::null_mut();
            let mut length = 0u32;
            buffer.Lock(&raw mut data, None, Some(&raw mut length))?;
            if data.is_null() {
                let _ = buffer.Unlock();
                return Err(CaptureError::BadFrame("缓冲区为空".to_string()));
            }
            let slice = std::slice::from_raw_parts(data, length as usize);
            let frame = Frame::from_bgra(self.width, self.height, self.stride, slice, timestamp);
            let _ = buffer.Unlock();
            frame
        }
    }
}

fn frame_size(media_type: &IMFMediaType) -> Result<(u32, u32), CaptureError> {
    // SAFETY: media_type 有效；返回的 u64 高 32 位是宽，低 32 位是高。
    let packed = unsafe { media_type.GetUINT64(&MF_MT_FRAME_SIZE) }?;
    Ok(((packed >> 32) as u32, (packed & 0xFFFF_FFFF) as u32))
}

fn enumerate_activates() -> Result<Vec<IMFActivate>, CaptureError> {
    ensure_platform()?;
    // SAFETY: 标准 MF 设备枚举流程；数组由 CoTaskMemAlloc 分配，使用后释放。
    unsafe {
        let mut attributes: Option<IMFAttributes> = None;
        MFCreateAttributes(&raw mut attributes, 2)?;
        let attributes = attributes.ok_or(CaptureError::NoDevices)?;
        attributes.SetGUID(
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
        )?;

        let mut raw: *mut Option<IMFActivate> = ptr::null_mut();
        let mut count = 0u32;
        MFEnumDeviceSources(&attributes, &raw mut raw, &raw mut count)?;

        let mut out = Vec::new();
        if !raw.is_null() {
            let slice = std::slice::from_raw_parts(raw, count as usize);
            for activate in slice.iter().flatten() {
                out.push(activate.clone());
            }
            CoTaskMemFree(Some(raw.cast()));
        }
        Ok(out)
    }
}

fn friendly_name(activate: &IMFActivate) -> Result<String, CaptureError> {
    Ok(activate_string(activate, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME)?.unwrap_or_default())
}

/// 读取设备的符号链接；驱动不提供时返回 `None`。
fn symbolic_link(activate: &IMFActivate) -> Option<String> {
    activate_string(
        activate,
        &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
    )
    .ok()
    .flatten()
    .filter(|link| !link.is_empty())
}

fn activate_string(activate: &IMFActivate, key: &GUID) -> Result<Option<String>, CaptureError> {
    // SAFETY: GetAllocatedString 分配的内存由 CoTaskMemFree 释放。
    unsafe {
        let mut ptr = PWSTR::null();
        let mut len = 0u32;
        activate.GetAllocatedString(key, &raw mut ptr, &raw mut len)?;
        if ptr.is_null() {
            return Ok(None);
        }
        let value = String::from_utf16_lossy(std::slice::from_raw_parts(ptr.0, len as usize));
        CoTaskMemFree(Some(ptr.0.cast()));
        Ok(Some(value))
    }
}

/// 判断某个 GUID 是否等于已知值，供调试与测试使用。
#[must_use]
pub fn guid_eq(a: &GUID, b: &GUID) -> bool {
    a == b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn com_guard_can_be_created() {
        let guard = init_thread();
        drop(guard);
    }

    #[test]
    fn platform_initializes() {
        ensure_platform().expect("MFStartup");
    }

    #[test]
    fn enumeration_does_not_fail() {
        // 没有摄像头时返回空列表，而不是错误。
        let cameras = list_cameras().expect("enumerate");
        for camera in cameras {
            assert_ne!(camera.name, "");
            assert_ne!(camera.id, "");
        }
    }
}
