// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! DirectShow 兜底采集后端。
//!
//! Media Foundation 对部分老设备、采集卡和虚拟摄像头支持不完整。这里用
//! DirectShow + Sample Grabber 读取 RGB24 帧，再转换成统一的 BGRA8。
//! Sample Grabber 不在 `windows` crate 的元数据里，因此本文件手工声明它
//! 的 COM 接口，只使用缓冲区轮询模式（不需要回调对象）。

#![allow(clippy::borrow_as_ptr)]

use std::ffi::c_void;
use std::ptr;

use windows::core::{Interface, BOOL, GUID, HSTRING, PWSTR};
use windows::Win32::Graphics::Gdi::BITMAPINFOHEADER;
use windows::Win32::Media::DirectShow::{
    IBaseFilter, ICaptureGraphBuilder2, ICreateDevEnum, IEnumPins, IGraphBuilder, IMediaControl,
    IPin, PINDIR_INPUT, PINDIR_OUTPUT,
};
use windows::Win32::Media::MediaFoundation::{
    CLSID_CaptureGraphBuilder2, CLSID_FilterGraph, CLSID_SystemDeviceEnum,
    CLSID_VideoInputDeviceCategory, FORMAT_VideoInfo, FORMAT_VideoInfo2, MEDIATYPE_Video,
    AM_MEDIA_TYPE, MEDIASUBTYPE_RGB24, PIN_CATEGORY_CAPTURE, PIN_CATEGORY_PREVIEW, VIDEOINFOHEADER,
    VIDEOINFOHEADER2,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoTaskMemFree, IBindCtx, IEnumMoniker, IMoniker, CLSCTX_INPROC_SERVER,
};

use crate::error::CaptureError;
use crate::frame::{Frame, PixelFormat};
use crate::mf::CameraInfo;

const CLSID_SAMPLE_GRABBER: GUID = GUID::from_u128(0xC1F400A0_3F08_11D3_9F0B_006008039E37);
const CLSID_NULL_RENDERER: GUID = GUID::from_u128(0xC1F400A4_3F08_11D3_9F0B_006008039E37);
const ISAMPLEGRABBER_IID: u128 = 0x6b652fff_11fe_4fce_92ad_0266b5d7c487;

#[repr(transparent)]
#[derive(Clone, PartialEq, Eq)]
pub struct ISampleGrabber(windows::core::IUnknown);

unsafe impl windows::core::Interface for ISampleGrabber {
    type Vtable = ISampleGrabber_Vtbl;
    const IID: GUID = GUID::from_u128(ISAMPLEGRABBER_IID);
}

impl std::fmt::Debug for ISampleGrabber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ISampleGrabber")
            .field(&windows::core::Interface::as_raw(self))
            .finish()
    }
}

#[repr(C)]
#[allow(non_snake_case)]
pub struct ISampleGrabber_Vtbl {
    pub base__: windows::core::IUnknown_Vtbl,
    pub SetOneShot:
        unsafe extern "system" fn(*mut c_void, windows::core::BOOL) -> windows::core::HRESULT,
    pub SetMediaType:
        unsafe extern "system" fn(*mut c_void, *const AM_MEDIA_TYPE) -> windows::core::HRESULT,
    pub GetConnectedMediaType:
        unsafe extern "system" fn(*mut c_void, *mut AM_MEDIA_TYPE) -> windows::core::HRESULT,
    pub SetBufferSamples:
        unsafe extern "system" fn(*mut c_void, windows::core::BOOL) -> windows::core::HRESULT,
    pub GetCurrentBuffer:
        unsafe extern "system" fn(*mut c_void, *mut i32, *mut u8) -> windows::core::HRESULT,
    pub GetCurrentSample:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> windows::core::HRESULT,
    pub SetCallback:
        unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> windows::core::HRESULT,
}

impl ISampleGrabber {
    unsafe fn set_one_shot(&self, value: bool) -> windows::core::Result<()> {
        unsafe {
            (windows::core::Interface::vtable(self).SetOneShot)(
                windows::core::Interface::as_raw(self),
                BOOL::from(value),
            )
            .ok()
        }
    }

    unsafe fn set_media_type(&self, media_type: &AM_MEDIA_TYPE) -> windows::core::Result<()> {
        unsafe {
            (windows::core::Interface::vtable(self).SetMediaType)(
                windows::core::Interface::as_raw(self),
                ptr::from_ref(media_type),
            )
            .ok()
        }
    }

    unsafe fn connected_media_type(&self) -> windows::core::Result<AM_MEDIA_TYPE> {
        unsafe {
            let mut media_type = AM_MEDIA_TYPE::default();
            (windows::core::Interface::vtable(self).GetConnectedMediaType)(
                windows::core::Interface::as_raw(self),
                &mut media_type,
            )
            .ok()?;
            Ok(media_type)
        }
    }

    unsafe fn set_buffer_samples(&self, value: bool) -> windows::core::Result<()> {
        unsafe {
            (windows::core::Interface::vtable(self).SetBufferSamples)(
                windows::core::Interface::as_raw(self),
                BOOL::from(value),
            )
            .ok()
        }
    }

    unsafe fn current_buffer(&self, size: &mut i32, buffer: *mut u8) -> windows::core::Result<()> {
        unsafe {
            (windows::core::Interface::vtable(self).GetCurrentBuffer)(
                windows::core::Interface::as_raw(self),
                size,
                buffer,
            )
            .ok()
        }
    }
}

/// 枚举 DirectShow 视频输入设备。
pub fn list_cameras() -> Result<Vec<CameraInfo>, CaptureError> {
    let mut cameras = Vec::new();
    let enumerator: ICreateDevEnum = unsafe {
        CoCreateInstance(
            &CLSID_SystemDeviceEnum,
            None::<&windows::core::IUnknown>,
            CLSCTX_INPROC_SERVER,
        )?
    };
    let mut class_enum: Option<IEnumMoniker> = None;
    // SAFETY: 标准 DirectShow 设备枚举。
    unsafe {
        enumerator.CreateClassEnumerator(&CLSID_VideoInputDeviceCategory, &mut class_enum, 0)?;
    }
    let Some(class_enum) = class_enum else {
        return Ok(cameras);
    };

    let mut index = 0usize;
    loop {
        let mut monikers = [None];
        let mut fetched = 0u32;
        // SAFETY: 输出数组与计数指针均有效。
        let result = unsafe { class_enum.Next(&mut monikers, Some(&mut fetched)) };
        if result.0 < 0 || fetched == 0 {
            break;
        }
        let Some(moniker) = monikers[0].take() else {
            break;
        };
        let display = display_name(&moniker).unwrap_or_default();
        let name = if display.is_empty() || display.starts_with("@device:") {
            format!("DirectShow 摄像头 {}", index + 1)
        } else {
            display
        };
        cameras.push(CameraInfo {
            id: format!("dshow:{index}:{name}"),
            name,
        });
        index += 1;
    }
    Ok(cameras)
}

fn display_name(moniker: &IMoniker) -> Option<String> {
    // SAFETY: GetDisplayName 返回 CoTaskMemAlloc 分配的内存。
    unsafe {
        let pointer: PWSTR = moniker
            .GetDisplayName(None::<&IBindCtx>, None::<&IMoniker>)
            .ok()?;
        if pointer.is_null() {
            return None;
        }
        let text = pointer.to_string().ok();
        CoTaskMemFree(Some(pointer.0.cast()));
        text
    }
}

/// 已打开的 DirectShow 摄像头。
pub struct DShowCamera {
    info: CameraInfo,
    #[allow(dead_code)]
    graph: IGraphBuilder,
    control: IMediaControl,
    grabber: ISampleGrabber,
    width: u32,
    height: u32,
    stride: usize,
    bottom_up: bool,
    buffer: Vec<u8>,
}

impl DShowCamera {
    /// 打开 DirectShow 摄像头。
    pub fn open(info: &CameraInfo, _preferred: Option<(u32, u32)>) -> Result<Self, CaptureError> {
        let moniker =
            find_moniker(info).ok_or_else(|| CaptureError::DeviceNotFound(info.name.clone()))?;
        // SAFETY: 标准 DirectShow 图构建流程；接口句柄由返回值持有。
        unsafe {
            let source: IBaseFilter = moniker.BindToObject(None::<&IBindCtx>, None::<&IMoniker>)?;
            let graph: IGraphBuilder = CoCreateInstance(
                &CLSID_FilterGraph,
                None::<&windows::core::IUnknown>,
                CLSCTX_INPROC_SERVER,
            )?;
            let builder: ICaptureGraphBuilder2 = CoCreateInstance(
                &CLSID_CaptureGraphBuilder2,
                None::<&windows::core::IUnknown>,
                CLSCTX_INPROC_SERVER,
            )?;
            builder.SetFiltergraph(&graph)?;
            graph.AddFilter(&source, &HSTRING::from("usbscan-source"))?;

            let grabber_unknown: windows::core::IUnknown = CoCreateInstance(
                &CLSID_SAMPLE_GRABBER,
                None::<&windows::core::IUnknown>,
                CLSCTX_INPROC_SERVER,
            )?;
            let grabber_filter: IBaseFilter = grabber_unknown.cast()?;
            let grabber: ISampleGrabber = grabber_unknown.cast()?;
            graph.AddFilter(&grabber_filter, &HSTRING::from("usbscan-grabber"))?;

            let null_renderer: IBaseFilter = CoCreateInstance(
                &CLSID_NULL_RENDERER,
                None::<&windows::core::IUnknown>,
                CLSCTX_INPROC_SERVER,
            )?;
            graph.AddFilter(&null_renderer, &HSTRING::from("usbscan-null"))?;

            let media_type = AM_MEDIA_TYPE {
                majortype: MEDIATYPE_Video,
                subtype: MEDIASUBTYPE_RGB24,
                formattype: FORMAT_VideoInfo,
                ..AM_MEDIA_TYPE::default()
            };
            grabber.set_media_type(&media_type)?;
            grabber.set_one_shot(false)?;
            grabber.set_buffer_samples(true)?;

            let mut render = builder.RenderStream(
                Some(ptr::from_ref(&PIN_CATEGORY_CAPTURE)),
                ptr::from_ref(&MEDIATYPE_Video),
                &source,
                None::<&IBaseFilter>,
                Some(&grabber_filter),
            );
            if render.is_err() {
                // 部分设备只暴露 Preview 引脚。
                render = builder.RenderStream(
                    Some(ptr::from_ref(&PIN_CATEGORY_PREVIEW)),
                    ptr::from_ref(&MEDIATYPE_Video),
                    &source,
                    None::<&IBaseFilter>,
                    Some(&grabber_filter),
                );
            }
            render.map_err(|err| CaptureError::Stage {
                step: "连接采集图",
                source: Box::new(err.into()),
            })?;

            let output_pin = first_pin(&grabber_filter, PINDIR_OUTPUT)?;
            let input_pin = first_pin(&null_renderer, PINDIR_INPUT)?;
            graph.Connect(&output_pin, &input_pin)?;

            let mut connected = grabber.connected_media_type()?;
            let (width, height, bottom_up) = parse_video_info(&connected)?;
            if !connected.pbFormat.is_null() {
                CoTaskMemFree(Some(connected.pbFormat.cast()));
            }
            let _ = std::mem::ManuallyDrop::take(&mut connected.pUnk);

            let control: IMediaControl = graph.cast()?;
            control.Run()?;

            Ok(Self {
                info: info.clone(),
                graph,
                control,
                grabber,
                width,
                height,
                stride: ((width as usize * 3 + 3) & !3).max(width as usize * 3),
                bottom_up,
                buffer: Vec::new(),
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

    /// DirectShow 兜底路径只报告当前实际分辨率。
    #[must_use]
    pub fn formats(&self) -> Vec<(u32, u32)> {
        vec![(self.width, self.height)]
    }

    /// 阻塞读取最新一帧。
    pub fn read_frame(&mut self) -> Result<Frame, CaptureError> {
        // SAFETY: Sample Grabber 缓冲区尺寸/内容由 COM 对象维护。
        let mut size = 0i32;
        unsafe {
            self.grabber
                .current_buffer(&mut size, ptr::null_mut())
                .map_err(|_| CaptureError::NoFrame)?;
            if size <= 0 {
                return Err(CaptureError::NoFrame);
            }
            if self.buffer.len() < size as usize {
                self.buffer.resize(size as usize, 0);
            }
            self.grabber
                .current_buffer(&mut size, self.buffer.as_mut_ptr())
                .map_err(|_| CaptureError::NoFrame)?;
            if size <= 0 {
                return Err(CaptureError::NoFrame);
            }
        }

        let width = self.width as usize;
        let height = self.height as usize;
        let mut data = vec![0u8; width * height * 4];
        let source = &self.buffer[..(size as usize).min(self.buffer.len())];
        for row in 0..height {
            let source_row = if self.bottom_up {
                height - 1 - row
            } else {
                row
            };
            let start = source_row * self.stride;
            for column in 0..width {
                let offset = start + column * 3;
                let destination = (row * width + column) * 4;
                if offset + 2 < source.len() {
                    data[destination] = source[offset + 2];
                    data[destination + 1] = source[offset + 1];
                    data[destination + 2] = source[offset];
                    data[destination + 3] = 255;
                } else {
                    data[destination..destination + 4].copy_from_slice(&[0, 0, 0, 255]);
                }
            }
        }
        Ok(Frame {
            width: self.width,
            height: self.height,
            format: PixelFormat::Bgra8,
            data,
            timestamp: 0,
        })
    }
}

impl Drop for DShowCamera {
    fn drop(&mut self) {
        // SAFETY: 图与控制接口由本对象持有。
        unsafe {
            let _ = self.control.Stop();
        }
    }
}

fn find_moniker(info: &CameraInfo) -> Option<IMoniker> {
    let enumerator: ICreateDevEnum = unsafe {
        CoCreateInstance(
            &CLSID_SystemDeviceEnum,
            None::<&windows::core::IUnknown>,
            CLSCTX_INPROC_SERVER,
        )
        .ok()?
    };
    let mut class_enum: Option<IEnumMoniker> = None;
    // SAFETY: 标准设备枚举。
    unsafe {
        enumerator
            .CreateClassEnumerator(&CLSID_VideoInputDeviceCategory, &mut class_enum, 0)
            .ok()?;
    }
    let class_enum = class_enum?;
    let mut index = 0usize;
    loop {
        let mut monikers = [None];
        let mut fetched = 0u32;
        // SAFETY: 输出数组有效。
        let result = unsafe { class_enum.Next(&mut monikers, Some(&mut fetched)) };
        if result.0 < 0 || fetched == 0 {
            return None;
        }
        let moniker = monikers[0].take()?;
        let display = display_name(&moniker).unwrap_or_default();
        let wanted = crate::pnp_key(&info.id).or_else(|| crate::pnp_key(&info.name));
        if info.id == format!("dshow:{index}:{display}")
            || info.name == display
            || wanted
                .as_ref()
                .is_some_and(|key| crate::pnp_key(&display).as_ref() == Some(key))
        {
            return Some(moniker);
        }
        index += 1;
    }
}

fn first_pin(
    filter: &IBaseFilter,
    direction: windows::Win32::Media::DirectShow::PIN_DIRECTION,
) -> Result<IPin, CaptureError> {
    // SAFETY: 枚举 pins；只读取接口。
    unsafe {
        let pins: IEnumPins = filter.EnumPins()?;
        loop {
            let mut pin = [None];
            let mut fetched = 0u32;
            let result = pins.Next(&mut pin, Some(&mut fetched));
            if result.0 < 0 || fetched == 0 {
                return Err(CaptureError::BadFrame("找不到可用的引脚".to_string()));
            }
            if let Some(pin) = pin[0].take() {
                if pin.QueryDirection()? == direction {
                    return Ok(pin);
                }
            }
        }
    }
}

fn parse_video_info(media_type: &AM_MEDIA_TYPE) -> Result<(u32, u32, bool), CaptureError> {
    if media_type.pbFormat.is_null() {
        return Err(CaptureError::BadFrame(
            "DirectShow 没有返回视频格式".to_string(),
        ));
    }
    let header: &BITMAPINFOHEADER = if media_type.formattype == FORMAT_VideoInfo {
        // SAFETY: pbFormat 由 GetConnectedMediaType 分配并至少容纳 VIDEOINFOHEADER。
        unsafe { &(*(media_type.pbFormat.cast::<VIDEOINFOHEADER>())).bmiHeader }
    } else if media_type.formattype == FORMAT_VideoInfo2 {
        // SAFETY: 同上，VIDEOINFOHEADER2 的 bmiHeader 位置。
        unsafe { &(*(media_type.pbFormat.cast::<VIDEOINFOHEADER2>())).bmiHeader }
    } else {
        return Err(CaptureError::BadFrame(
            "DirectShow 返回了未知的视频格式".to_string(),
        ));
    };
    if header.biWidth <= 0 || header.biHeight == 0 {
        return Err(CaptureError::BadFrame(
            "DirectShow 返回的尺寸无效".to_string(),
        ));
    }
    let width = header.biWidth as u32;
    let height = header.biHeight.unsigned_abs();
    Ok((width, height, header.biHeight > 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_grabber_iid_is_correct() {
        let iid = <ISampleGrabber as windows::core::Interface>::IID;
        assert_eq!(iid, GUID::from_u128(ISAMPLEGRABBER_IID));
    }

    #[test]
    fn parses_video_info_header() {
        let mut info: VIDEOINFOHEADER = unsafe { std::mem::zeroed() };
        info.bmiHeader.biWidth = 1280;
        info.bmiHeader.biHeight = -720;
        let media_type = AM_MEDIA_TYPE {
            formattype: FORMAT_VideoInfo,
            pbFormat: (&raw mut info).cast(),
            ..AM_MEDIA_TYPE::default()
        };
        let (width, height, bottom_up) = parse_video_info(&media_type).expect("info");
        assert_eq!((width, height), (1280, 720));
        assert!(!bottom_up);
    }
}
