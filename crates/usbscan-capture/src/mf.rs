// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! Media Foundation 采集后端。
//!
//! 只使用 Media Foundation（Win10 1607+ 自带），不依赖厂商 SDK。
//! 枚举、打开、取帧都在调用方线程执行；本模块内部不创建线程。

use std::ptr;

use std::cmp::Reverse;

use windows::core::{GUID, HSTRING, PWSTR};
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, IMFAttributes, IMFMediaSource, IMFMediaType, IMFSample, IMFSourceReader,
    MFCreateAttributes, MFCreateDeviceSource, MFCreateMediaType,
    MFCreateSourceReaderFromMediaSource, MFEnumDeviceSources, MFMediaType_Video,
    MFVideoFormat_RGB32, MFVideoInterlace_Progressive, MFSTARTUP_FULL,
    MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, MF_MT_DEFAULT_STRIDE,
    MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE,
    MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MF_READWRITE_DISABLE_CONVERTERS,
    MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED, MF_SOURCE_READERF_ENDOFSTREAM,
    MF_SOURCE_READERF_ERROR, MF_SOURCE_READER_ALL_STREAMS,
    MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING,
    MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_VERSION,
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
            windows::Win32::Media::MediaFoundation::MFStartup(MF_VERSION, MFSTARTUP_FULL)
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
    let activates = stage("枚举摄像头", enumerate_activates())?;
    let mut cameras = Vec::with_capacity(activates.len());
    for (index, act) in activates.iter().enumerate() {
        // 个别驱动不提供 FriendlyName，不能因为它让整份设备列表消失。
        let name = friendly_name(act)
            .ok()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| format!("摄像头 {}", index + 1));
        // 优先使用符号链接：设备顺序变化或热插拔后仍能认得同一台摄像头。
        let id = symbolic_link(act).unwrap_or_else(|| format!("{name}#{index}"));
        cameras.push(CameraInfo { id, name });
    }
    Ok(cameras)
}

/// 激活动设备；个别虚拟摄像头对 `ActivateObject` 返回 `E_INVALIDARG`，
/// 此时改用符号链接重新创建设备源。
fn activate_media_source(activate: &IMFActivate) -> Result<IMFMediaSource, CaptureError> {
    // SAFETY: activate 由本模块枚举得到，类型正确。
    match unsafe { activate.ActivateObject() } {
        Ok(source) => Ok(source),
        Err(first) => {
            let Some(link) = symbolic_link(activate) else {
                return Err(activation_error(first));
            };
            let mut attributes: Option<IMFAttributes> = None;
            stage("创建重试属性", unsafe {
                MFCreateAttributes(&raw mut attributes, 2)
            })?;
            let Some(attributes) = attributes else {
                return Err(activation_error(first));
            };
            // SAFETY: attributes 有效；设置失败时下面的创建会返回原始错误。
            unsafe {
                let _ = attributes.SetGUID(
                    &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                    &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
                );
                let _ = attributes.SetString(
                    &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
                    &HSTRING::from(link),
                );
                match MFCreateDeviceSource(&attributes) {
                    Ok(source) => Ok(source),
                    Err(_) => Err(activation_error(first)),
                }
            }
        }
    }
}

fn activation_error(err: windows::core::Error) -> CaptureError {
    // 虚拟摄像头（OBS、手机投屏等）常见：枚举成功但 ActivateObject 返回
    // E_INVALIDARG。这里给出比“参数错误”更可操作的提示。
    let source = if err.code().0 as u32 == 0x8007_0057 {
        CaptureError::BadFrame(
            "该摄像头无法被 Media Foundation 激活（0x80070057），可能是虚拟摄像头或驱动仅支持其他采集框架"
                .to_string(),
        )
    } else {
        CaptureError::Windows(err)
    };
    CaptureError::Stage {
        step: "激活摄像头",
        source: Box::new(source),
    }
}

/// 创建 Source Reader 并协商 RGB32 输出。
///
/// `advanced = false` 时启用旧版 YUV→RGB32 转换路径，用于高级视频处理
/// 被驱动拒绝（例如 `E_INVALIDARG`）时的回退。
fn open_reader(
    source: &IMFMediaSource,
    advanced: bool,
) -> Result<(IMFSourceReader, u32, u32, i32), CaptureError> {
    // SAFETY: source 有效；所有 COM 调用均检查返回值。
    unsafe {
        let mut attributes: Option<IMFAttributes> = None;
        stage("创建采集属性", MFCreateAttributes(&raw mut attributes, 2))?;
        let attributes =
            attributes.ok_or_else(|| CaptureError::BadFrame("无法创建采集属性".to_string()))?;
        if advanced {
            stage(
                "启用视频处理",
                attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1),
            )?;
        } else {
            stage(
                "启用兼容处理",
                attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1),
            )?;
        }
        // 视频处理要求允许插入转换器（默认即为 FALSE，这里显式设置）。
        let _ = attributes.SetUINT32(&MF_READWRITE_DISABLE_CONVERTERS, 0);

        let reader = stage(
            "创建采集器",
            MFCreateSourceReaderFromMediaSource(source, Some(&attributes)),
        )?;
        let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        // 先关闭全部流，再只打开视频流，避免音频/元数据流干扰格式协商。
        // 某些虚拟摄像头不支持通配选择，失败时继续即可。
        let _ = reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false);
        stage("选择视频流", reader.SetStreamSelection(stream, true))?;

        // 逐个尝试原生格式，优先 1080p 以内的最高分辨率。
        // 直接给 RGB32 而不指定帧尺寸，部分驱动会返回 E_INVALIDARG。
        let formats = native_formats(&reader);
        let mut configured = false;
        for native in preferred_formats(&formats) {
            let Ok(media_type) = build_rgb32_type(Some(native)) else {
                continue;
            };
            if reader
                .SetCurrentMediaType(stream, None, &media_type)
                .is_ok()
            {
                configured = true;
                break;
            }
        }
        if !configured {
            // 原生尺寸都被拒绝时，尝试常见的 720p 输出尺寸；
            // Video Processor 可以负责缩放。
            let fallback = NativeFormat {
                width: 1280,
                height: 720,
                frame_rate: None,
                pixel_aspect: None,
            };
            if let Ok(media_type) = build_rgb32_type(Some(&fallback)) {
                configured = reader
                    .SetCurrentMediaType(stream, None, &media_type)
                    .is_ok();
            }
        }
        if !configured {
            // 驱动没有报告原生格式，或所有带尺寸的组合都被拒绝；
            // 退回最小 RGB32 请求，让 Source Reader 自行协商。
            let media_type = stage("创建 RGB32 类型", build_rgb32_type(None))?;
            stage(
                "设置 RGB32 输出",
                reader.SetCurrentMediaType(stream, None, &media_type),
            )?;
        }

        let current = stage("读取输出格式", reader.GetCurrentMediaType(stream))?;
        let subtype = current
            .GetGUID(&MF_MT_SUBTYPE)
            .unwrap_or(GUID::from_u128(0));
        if subtype != MFVideoFormat_RGB32 {
            return Err(CaptureError::UnsupportedFormat(subtype));
        }
        let (width, height) = stage("读取帧尺寸", frame_size(&current))?;
        let stride = current
            .GetUINT32(&MF_MT_DEFAULT_STRIDE)
            .map_or((width * 4) as i32, |v| v as i32);
        Ok((reader, width, height, stride))
    }
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
        let activates = stage("枚举摄像头", enumerate_activates())?;
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

        let source = activate_media_source(activate)?;
        // 先启用高级视频处理；失败时退回旧版 YUV→RGB32 转换路径。
        let (reader, width, height, stride) =
            open_reader(&source, true).or_else(|_| open_reader(&source, false))?;

        Ok(Self {
            info: info.clone(),
            reader,
            width,
            height,
            stride,
            warmup: false,
        })
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

/// 给错误附加“失败发生在哪一步”，便于用户反馈和日志定位。
fn stage<T, E>(step: &'static str, result: Result<T, E>) -> Result<T, CaptureError>
where
    E: Into<CaptureError>,
{
    result.map_err(|source| CaptureError::Stage {
        step,
        source: Box::new(source.into()),
    })
}

/// 摄像头原生输出格式。
struct NativeFormat {
    width: u32,
    height: u32,
    frame_rate: Option<u64>,
    pixel_aspect: Option<u64>,
}

/// 帧率/像素比以 `(分子 << 32) | 分母` 存储；0 值会让部分驱动拒绝输出类型。
fn valid_ratio(ratio: u64) -> bool {
    (ratio >> 32) != 0 && (ratio & 0xFFFF_FFFF) != 0
}

/// 枚举摄像头的原生媒体类型。
fn native_formats(reader: &IMFSourceReader) -> Vec<NativeFormat> {
    let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
    let mut formats = Vec::new();
    for index in 0..64u32 {
        // SAFETY: reader 有效；索引越界时返回错误，直接结束枚举。
        let Ok(media_type) = (unsafe { reader.GetNativeMediaType(stream, index) }) else {
            break;
        };
        let Ok((width, height)) = frame_size(&media_type) else {
            continue;
        };
        if width == 0 || height == 0 {
            continue;
        }
        // SAFETY: media_type 有效；帧率/像素比可缺失或为 0，需要过滤掉。
        let frame_rate = unsafe { media_type.GetUINT64(&MF_MT_FRAME_RATE) }
            .ok()
            .filter(|value| valid_ratio(*value));
        let pixel_aspect = unsafe { media_type.GetUINT64(&MF_MT_PIXEL_ASPECT_RATIO) }
            .ok()
            .filter(|value| valid_ratio(*value));
        formats.push(NativeFormat {
            width,
            height,
            frame_rate,
            pixel_aspect,
        });
    }
    formats
}

/// 优先顺序：1080p 以内从高到低，然后才是超过 1080p 的格式（从小到大）。
fn preferred_formats(formats: &[NativeFormat]) -> Vec<&NativeFormat> {
    let mut within: Vec<_> = formats
        .iter()
        .filter(|format| format.width <= 1920 && format.height <= 1080)
        .collect();
    within.sort_by_key(|format| Reverse(u64::from(format.width) * u64::from(format.height)));

    let mut over: Vec<_> = formats
        .iter()
        .filter(|format| format.width > 1920 || format.height > 1080)
        .collect();
    over.sort_by_key(|format| u64::from(format.width) * u64::from(format.height));

    within.extend(over);
    within
}

/// 构造 RGB32 输出类型；`native` 提供帧尺寸与帧率。
fn build_rgb32_type(native: Option<&NativeFormat>) -> Result<IMFMediaType, CaptureError> {
    // SAFETY: 创建并配置媒体类型，所有返回值均检查。
    unsafe {
        let media_type = MFCreateMediaType()?;
        media_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        media_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
        media_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        if let Some(native) = native {
            let size = (u64::from(native.width) << 32) | u64::from(native.height);
            media_type.SetUINT64(&MF_MT_FRAME_SIZE, size)?;
            if let Some(rate) = native.frame_rate {
                media_type.SetUINT64(&MF_MT_FRAME_RATE, rate)?;
            }
            if let Some(aspect) = native.pixel_aspect {
                media_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, aspect)?;
            }
        }
        Ok(media_type)
    }
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

    #[test]
    fn rgb32_type_carries_native_size() {
        ensure_platform().expect("MFStartup");
        let native = NativeFormat {
            width: 1280,
            height: 720,
            frame_rate: Some((30u64 << 32) | 0x1),
            pixel_aspect: None,
        };
        let media_type = build_rgb32_type(Some(&native)).expect("type");
        // SAFETY: media_type 由本测试创建且有效。
        unsafe {
            let subtype = media_type.GetGUID(&MF_MT_SUBTYPE).expect("subtype");
            assert!(guid_eq(&subtype, &MFVideoFormat_RGB32));
            assert_eq!(
                media_type.GetUINT64(&MF_MT_FRAME_SIZE).expect("size"),
                (1280u64 << 32) | 0x02D0
            );
            assert_eq!(
                media_type.GetUINT64(&MF_MT_FRAME_RATE).expect("rate"),
                (30u64 << 32) | 0x1
            );
        }
    }

    #[test]
    fn prefers_1080p_before_4k() {
        let formats = [
            NativeFormat {
                width: 640,
                height: 480,
                frame_rate: None,
                pixel_aspect: None,
            },
            NativeFormat {
                width: 3840,
                height: 2160,
                frame_rate: None,
                pixel_aspect: None,
            },
            NativeFormat {
                width: 1920,
                height: 1080,
                frame_rate: None,
                pixel_aspect: None,
            },
            NativeFormat {
                width: 1280,
                height: 720,
                frame_rate: None,
                pixel_aspect: None,
            },
        ];
        let order: Vec<_> = preferred_formats(&formats)
            .iter()
            .map(|format| (format.width, format.height))
            .collect();
        assert_eq!(
            order,
            vec![(1920, 1080), (1280, 720), (640, 480), (3840, 2160)]
        );
    }
}
