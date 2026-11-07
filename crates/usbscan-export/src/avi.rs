// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 轻量 Motion-JPEG AVI 写入器。
//!
//! 直接复用摄像头已有的 JPEG 编码，不依赖系统编码器；生成标准 AVI，
//! 可在 Windows 播放器、VLC、ffmpeg 中播放（无音轨）。

use std::fs::{File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// 一帧 JPEG 的 AVI 写入器。
pub struct AviWriter {
    file: File,
    path: PathBuf,
    frames: u32,
    riff_size: u64,
    avih_frames: u64,
    strh_length: u64,
    movi_size: u64,
    movi_fourcc: u64,
    index: Vec<(u32, u32)>,
    finished: bool,
}

impl AviWriter {
    /// 创建 AVI 并写入文件头。
    pub fn create(path: &Path, width: u32, height: u32, fps: u32) -> io::Result<Self> {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .read(true)
            .write(true)
            .open(path)?;
        file.write_all(b"RIFF")?;
        let riff_size = position(&mut file)?;
        write_u32(&mut file, 0)?;
        file.write_all(b"AVI ")?;

        file.write_all(b"LIST")?;
        let hdrl_size = position(&mut file)?;
        write_u32(&mut file, 0)?;
        file.write_all(b"hdrl")?;

        file.write_all(b"avih")?;
        write_u32(&mut file, 56)?;
        write_u32(
            &mut file,
            (1_000_000 / u64::from(fps.max(1)))
                .try_into()
                .unwrap_or(100_000),
        )?;
        write_u32(&mut file, 0)?; // max bytes per second
        write_u32(&mut file, 0)?; // padding
        write_u32(&mut file, 0x10)?; // AVIF_HASINDEX
        let avih_frames = position(&mut file)?;
        write_u32(&mut file, 0)?;
        write_u32(&mut file, 0)?; // initial frames
        write_u32(&mut file, 1)?; // streams
        write_u32(&mut file, 0)?; // suggested buffer
        write_u32(&mut file, width)?;
        write_u32(&mut file, height)?;
        for _ in 0..4 {
            write_u32(&mut file, 0)?;
        }

        file.write_all(b"LIST")?;
        let strl_size = position(&mut file)?;
        write_u32(&mut file, 0)?;
        file.write_all(b"strl")?;

        file.write_all(b"strh")?;
        write_u32(&mut file, 56)?;
        file.write_all(b"vids")?;
        file.write_all(b"MJPG")?;
        write_u32(&mut file, 0)?; // flags
        write_u16(&mut file, 0)?; // priority
        write_u16(&mut file, 0)?; // language
        write_u32(&mut file, 0)?; // initial frames
        write_u32(&mut file, 1)?; // scale
        write_u32(&mut file, fps.max(1))?; // rate
        write_u32(&mut file, 0)?; // start
        let strh_length = position(&mut file)?;
        write_u32(&mut file, 0)?;
        write_u32(&mut file, 0)?; // suggested buffer
        write_u32(&mut file, 0xFFFF_FFFF)?; // quality
        write_u32(&mut file, 0)?; // sample size
        write_u16(&mut file, 0)?;
        write_u16(&mut file, 0)?;
        write_u16(&mut file, u16::try_from(width).unwrap_or(0))?;
        write_u16(&mut file, u16::try_from(height).unwrap_or(0))?;

        file.write_all(b"strf")?;
        write_u32(&mut file, 40)?;
        write_u32(&mut file, 40)?; // biSize
        write_u32(&mut file, width)?;
        write_u32(&mut file, height)?;
        write_u16(&mut file, 1)?;
        write_u16(&mut file, 24)?;
        file.write_all(b"MJPG")?;
        write_u32(&mut file, 0)?;
        for _ in 0..4 {
            write_u32(&mut file, 0)?;
        }

        let end = position(&mut file)?;
        patch_u32(&mut file, strl_size, (end - strl_size - 4) as u32)?;
        patch_u32(&mut file, hdrl_size, (end - hdrl_size - 4) as u32)?;

        file.write_all(b"LIST")?;
        let movi_size = position(&mut file)?;
        write_u32(&mut file, 0)?;
        let movi_fourcc = position(&mut file)?;
        file.write_all(b"movi")?;

        Ok(Self {
            file,
            path: path.to_path_buf(),
            frames: 0,
            riff_size,
            avih_frames,
            strh_length,
            movi_size,
            movi_fourcc,
            index: Vec::new(),
            finished: false,
        })
    }

    /// 追加一帧 JPEG。
    pub fn add_frame(&mut self, jpeg: &[u8]) -> io::Result<()> {
        let chunk_start = position(&mut self.file)?;
        let offset = (chunk_start - self.movi_fourcc) as u32;
        self.file.write_all(b"00dc")?;
        write_u32(&mut self.file, jpeg.len() as u32)?;
        self.file.write_all(jpeg)?;
        if jpeg.len() % 2 == 1 {
            self.file.write_all(&[0])?;
        }
        self.index.push((offset, jpeg.len() as u32));
        self.frames = self.frames.saturating_add(1);
        Ok(())
    }

    /// 写入索引并结束文件。
    pub fn finish(mut self) -> io::Result<PathBuf> {
        if self.finished {
            return Ok(self.path.clone());
        }
        self.finished = true;
        let end = position(&mut self.file)?;
        patch_u32(
            &mut self.file,
            self.movi_size,
            (end - self.movi_size - 4) as u32,
        )?;
        self.file.write_all(b"idx1")?;
        write_u32(&mut self.file, (self.index.len() * 16) as u32)?;
        for (offset, size) in &self.index {
            self.file.write_all(b"00dc")?;
            write_u32(&mut self.file, 0x10)?; // AVIIF_KEYFRAME
            write_u32(&mut self.file, *offset)?;
            write_u32(&mut self.file, *size)?;
        }
        let file_end = position(&mut self.file)?;
        patch_u32(&mut self.file, self.riff_size, (file_end - 8) as u32)?;
        patch_u32(&mut self.file, self.avih_frames, self.frames)?;
        patch_u32(&mut self.file, self.strh_length, self.frames)?;
        self.file.flush()?;
        self.file.sync_all()?;
        Ok(self.path.clone())
    }

    /// 当前已写入帧数。
    #[must_use]
    pub const fn frames(&self) -> u32 {
        self.frames
    }
}

fn position(file: &mut File) -> io::Result<u64> {
    file.stream_position()
}

fn write_u32(file: &mut File, value: u32) -> io::Result<()> {
    file.write_all(&value.to_le_bytes())
}

fn write_u16(file: &mut File, value: u16) -> io::Result<()> {
    file.write_all(&value.to_le_bytes())
}

fn patch_u32(file: &mut File, offset: u64, value: u32) -> io::Result<()> {
    let end = position(file)?;
    file.seek(SeekFrom::Start(offset))?;
    file.write_all(&value.to_le_bytes())?;
    file.seek(SeekFrom::Start(end))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_riff_avi_with_two_frames() {
        let directory =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-tmp");
        std::fs::create_dir_all(&directory).expect("dir");
        let path = directory.join(format!("usbscan-avi-test-{}.avi", std::process::id()));
        let mut writer = AviWriter::create(&path, 320, 240, 10).expect("create");
        writer
            .add_frame(&[0xFF, 0xD8, 1, 2, 3, 0xFF, 0xD9])
            .expect("frame1");
        writer
            .add_frame(&[0xFF, 0xD8, 4, 5, 6, 0xFF, 0xD9])
            .expect("frame2");
        let finished = writer.finish().expect("finish");
        let data = std::fs::read(&finished).expect("read");
        assert_eq!(&data[0..4], b"RIFF");
        assert_eq!(&data[8..12], b"AVI ");
        assert!(data.windows(4).any(|window| window == b"MJPG"));
        assert!(data.windows(4).any(|window| window == b"idx1"));
        assert!(data.windows(4).filter(|window| *window == b"00dc").count() >= 4);
        let _ = std::fs::remove_file(finished);
    }
}
