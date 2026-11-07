// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 录像线程：接收采集帧并用 Media Foundation 编码为 MP4/H.264。
//!
//! 采集线程只负责把最新帧放进有界队列；编码在独立线程完成，避免拖慢预览。
//! 输出帧率高于摄像头实际帧率时，重复上一帧补齐时间轴。

#![allow(clippy::cast_precision_loss)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use usbscan_capture as capture;
use usbscan_imaging as imaging;

/// 正在进行的录像。
pub struct Recorder {
    stop: Arc<AtomicBool>,
    frames: Arc<AtomicU64>,
    thread: Option<JoinHandle<Result<PathBuf, String>>>,
}

impl Recorder {
    /// 启动录像线程。`receiver` 由采集线程写入。
    pub fn start(
        path: PathBuf,
        width: u32,
        height: u32,
        fps: u32,
        receiver: Receiver<Arc<capture::Frame>>,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let frames = Arc::new(AtomicU64::new(0));
        let thread_stop = Arc::clone(&stop);
        let thread_frames = Arc::clone(&frames);
        let thread = thread::spawn(move || {
            encode_loop(
                &path,
                width,
                height,
                fps,
                &receiver,
                &thread_stop,
                &thread_frames,
            )
        });
        Self {
            stop,
            frames,
            thread: Some(thread),
        }
    }

    /// 已写入的帧数。
    #[must_use]
    pub fn frames(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }

    /// 停止并等待编码完成。
    pub fn stop(mut self) -> Result<PathBuf, String> {
        self.stop.store(true, Ordering::SeqCst);
        let Some(handle) = self.thread.take() else {
            return Err("录像线程已退出".to_string());
        };
        handle
            .join()
            .unwrap_or_else(|_| Err("录像线程异常退出".to_string()))
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

fn encode_loop(
    path: &Path,
    width: u32,
    height: u32,
    fps: u32,
    receiver: &Receiver<Arc<capture::Frame>>,
    stop: &AtomicBool,
    frames: &AtomicU64,
) -> Result<PathBuf, String> {
    let _com = capture::init_thread();
    capture::ensure_platform().map_err(|err| err.to_string())?;
    let mut writer =
        capture::Mp4Writer::create(path, width, height, fps).map_err(|err| err.to_string())?;

    let start = Instant::now();
    let frame_interval = 1.0 / f64::from(fps.max(1));
    let mut next_tick = 0u64;
    let mut last: Option<Vec<u8>> = None;

    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        match receiver.recv_timeout(Duration::from_millis(5)) {
            Ok(frame) => {
                last = Some(prepare_frame(&frame, width, height));
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        let elapsed = start.elapsed().as_secs_f64();
        // 编码落后太多时丢弃已经过期的重复帧，避免录像越拖越长。
        if next_tick as f64 * frame_interval + 1.0 < elapsed {
            next_tick = (elapsed / frame_interval).floor() as u64;
        }
        while next_tick as f64 * frame_interval <= elapsed {
            let Some(pixels) = last.as_deref() else {
                break;
            };
            writer.write_frame(pixels).map_err(|err| err.to_string())?;
            frames.fetch_add(1, Ordering::Relaxed);
            next_tick = next_tick.saturating_add(1);
        }
    }

    writer.finish().map_err(|err| err.to_string())
}

fn prepare_frame(frame: &capture::Frame, width: u32, height: u32) -> Vec<u8> {
    if frame.width == width && frame.height == height {
        return frame.data.clone();
    }
    imaging::thumbnail_bgra(frame.width, frame.height, &frame.data, width, height)
        .map_or_else(|_| frame.data.clone(), |(_, _, data)| data)
}
