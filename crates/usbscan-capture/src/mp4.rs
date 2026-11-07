// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! Media Foundation H.264 / MP4 录像写入器。
//!
//! 使用 Windows 自带的 H.264 编码器与 MP4 容器，不依赖外部 ffmpeg。
//! 输入为 top-down BGRA8（与采集层一致），输出为 `video/mp4; codecs=avc1`。

use std::path::{Path, PathBuf};
use std::ptr;

use windows::core::HSTRING;
use windows::Win32::Media::MediaFoundation::{
    eAVEncH264VProfile_High, IMFAttributes, IMFByteStream, IMFMediaBuffer, IMFSample,
    IMFSinkWriter, MFCreateAttributes, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample,
    MFCreateSinkWriterFromURL, MFMediaType_Video, MFTranscodeContainerType_MPEG4,
    MFVideoFormat_H264, MFVideoFormat_RGB32, MFVideoInterlace_Progressive, MF_MT_AVG_BITRATE,
    MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE,
    MF_MT_MAJOR_TYPE, MF_MT_MPEG2_PROFILE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE,
    MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_SINK_WRITER_DISABLE_THROTTLING,
    MF_TRANSCODE_CONTAINERTYPE,
};

use crate::error::CaptureError;

const HNS_PER_SECOND: i64 = 10_000_000;

/// 一个正在写入的 MP4 录像。
pub struct Mp4Writer {
    writer: IMFSinkWriter,
    stream: u32,
    width: u32,
    height: u32,
    duration: i64,
    next_time: i64,
    path: PathBuf,
    frames: u64,
}

impl Mp4Writer {
    /// 创建 MP4 文件并配置 H.264 编码。
    pub fn create(path: &Path, width: u32, height: u32, fps: u32) -> Result<Self, CaptureError> {
        if width == 0 || height == 0 || fps == 0 {
            return Err(CaptureError::BadFrame("录像尺寸或帧率无效".to_string()));
        }
        // SAFETY: Media Foundation 对象均在当前线程创建与使用。
        unsafe {
            let mut attributes: Option<IMFAttributes> = None;
            MFCreateAttributes(&raw mut attributes, 3)?;
            let attributes = attributes
                .ok_or_else(|| CaptureError::BadFrame("无法创建 MP4 写入属性".to_string()))?;
            attributes.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)?;
            attributes.SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1)?;
            let _ = attributes.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1);

            let writer = MFCreateSinkWriterFromURL(
                &HSTRING::from(path.to_string_lossy().as_ref()),
                None::<&IMFByteStream>,
                &attributes,
            )?;

            let output = MFCreateMediaType()?;
            output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            output.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
            output.SetUINT32(&MF_MT_AVG_BITRATE, bitrate(width, height, fps))?;
            output.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            output.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32)?;
            output.SetUINT64(&MF_MT_FRAME_SIZE, pack(width, height))?;
            output.SetUINT64(&MF_MT_FRAME_RATE, pack(fps, 1))?;
            output.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;

            let stream = writer.AddStream(&output)?;

            let input = MFCreateMediaType()?;
            input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
            input.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            input.SetUINT32(&MF_MT_DEFAULT_STRIDE, width.saturating_mul(4))?;
            input.SetUINT64(&MF_MT_FRAME_SIZE, pack(width, height))?;
            input.SetUINT64(&MF_MT_FRAME_RATE, pack(fps, 1))?;
            input.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
            writer.SetInputMediaType(stream, &input, None::<&IMFAttributes>)?;
            writer.BeginWriting()?;

            Ok(Self {
                writer,
                stream,
                width,
                height,
                duration: HNS_PER_SECOND / i64::from(fps.max(1)),
                next_time: 0,
                path: path.to_path_buf(),
                frames: 0,
            })
        }
    }

    /// 追加一帧 BGRA8；数据长度必须与创建时的尺寸一致。
    pub fn write_frame(&mut self, bgra: &[u8]) -> Result<(), CaptureError> {
        let expected = self.width as usize * self.height as usize * 4;
        if bgra.len() < expected {
            return Err(CaptureError::BadFrame(format!(
                "录像帧数据不足：需要 {expected} 字节，实际 {} 字节",
                bgra.len()
            )));
        }

        // SAFETY: 缓冲区、样本与写入器均在当前线程使用；每个 COM 调用都检查返回值。
        unsafe {
            let buffer: IMFMediaBuffer = MFCreateMemoryBuffer(
                u32::try_from(expected)
                    .map_err(|_| CaptureError::BadFrame("帧尺寸超出编码器范围".to_string()))?,
            )?;
            let mut data: *mut u8 = ptr::null_mut();
            buffer.Lock(&raw mut data, None, None)?;
            if data.is_null() {
                let _ = buffer.Unlock();
                return Err(CaptureError::BadFrame("录像缓冲区为空".to_string()));
            }
            ptr::copy_nonoverlapping(bgra.as_ptr(), data, expected);
            let _ = buffer.Unlock();
            buffer.SetCurrentLength(
                u32::try_from(expected)
                    .map_err(|_| CaptureError::BadFrame("帧尺寸超出编码器范围".to_string()))?,
            )?;

            let sample: IMFSample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            sample.SetSampleTime(self.next_time)?;
            sample.SetSampleDuration(self.duration)?;
            self.writer.WriteSample(self.stream, &sample)?;
            self.next_time = self.next_time.saturating_add(self.duration);
            self.frames = self.frames.saturating_add(1);
        }
        Ok(())
    }

    /// 完成写入并返回文件路径。
    pub fn finish(self) -> Result<PathBuf, CaptureError> {
        // SAFETY: writer 在完成前一直有效。
        unsafe {
            self.writer.Finalize()?;
        }
        Ok(self.path)
    }

    /// 已写入的帧数。
    #[must_use]
    pub const fn frames(&self) -> u64 {
        self.frames
    }
}

fn pack(high: u32, low: u32) -> u64 {
    (u64::from(high) << 32) | u64::from(low)
}

fn bitrate(width: u32, height: u32, fps: u32) -> u32 {
    let pixels = u64::from(width) * u64::from(height) * u64::from(fps);
    let estimate = pixels / 10;
    u32::try_from(estimate.clamp(2_000_000, 20_000_000)).unwrap_or(8_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_h264_mp4_container() {
        let _com = crate::init_thread();
        crate::ensure_platform().expect("media foundation");
        let path =
            std::env::temp_dir().join(format!("usbscan-mp4-test-{}.mp4", std::process::id()));
        let width = 64u32;
        let height = 64u32;
        let mut writer = match Mp4Writer::create(&path, width, height, 25) {
            Ok(writer) => writer,
            // 从非系统盘运行测试时，Windows 会拒绝 Media Foundation 编码器；
            // 与摄像头访问限制相同，不作为代码失败处理。
            Err(crate::CaptureError::Windows(err)) if err.code().0 as u32 == 0x8007_0005 => {
                eprintln!("skip: Media Foundation is blocked outside the system drive");
                return;
            }
            Err(err) => panic!("create writer: {err}"),
        };
        let frame = vec![128u8; width as usize * height as usize * 4];
        for _ in 0..10 {
            writer.write_frame(&frame).expect("write frame");
        }
        writer.finish().expect("finalize");
        let data = std::fs::read(&path).expect("read mp4");
        let _ = std::fs::remove_file(&path);
        assert!(data.len() > 128, "mp4 too small: {} bytes", data.len());
        assert_eq!(&data[4..8], b"ftyp");
    }
}
