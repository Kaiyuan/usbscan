// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 录像线程：接收采集帧并编码为 MP4/H.264。
//!
//! **Windows**：使用 Media Foundation (`Mp4Writer`)，无需外部依赖。
//!
//! **Linux / macOS**：调用系统 `ffmpeg`，通过标准输入喂入 BGRA 原始帧，
//! 输出 H.264 MP4。未安装 ffmpeg 时由上层提示安装命令。

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
    ///
    /// 非 Windows 平台需要系统 `ffmpeg`；可用性由 [`recording_supported`] 判断。
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

/// 当前平台是否具备录像能力。
///
/// Windows 使用系统 Media Foundation，始终可用；其它平台取决于是否安装 `ffmpeg`。
#[must_use]
pub fn recording_supported() -> bool {
    #[cfg(windows)]
    {
        true
    }
    #[cfg(not(windows))]
    {
        ffmpeg::command().is_some()
    }
}

/// 录像文件扩展名。所有平台统一输出 MP4。
#[must_use]
pub const fn recording_extension() -> &'static str {
    "mp4"
}

/// 编码器抽象：Windows 用 Media Foundation，其它平台用 ffmpeg 子进程。
trait FrameWriter {
    fn write_frame(&mut self, bgra: &[u8]) -> Result<(), String>;
    fn finish(self: Box<Self>) -> Result<(), String>;
}

#[cfg(windows)]
struct MediaFoundationWriter(capture::Mp4Writer);

#[cfg(windows)]
impl FrameWriter for MediaFoundationWriter {
    fn write_frame(&mut self, bgra: &[u8]) -> Result<(), String> {
        self.0.write_frame(bgra).map_err(|err| err.to_string())
    }

    fn finish(self: Box<Self>) -> Result<(), String> {
        self.0.finish().map(|_| ()).map_err(|err| err.to_string())
    }
}

#[cfg(windows)]
fn open_writer(
    path: &Path,
    width: u32,
    height: u32,
    fps: u32,
) -> Result<Box<dyn FrameWriter>, String> {
    let _com = capture::init_thread();
    capture::ensure_platform().map_err(|err| err.to_string())?;
    let writer =
        capture::Mp4Writer::create(path, width, height, fps).map_err(|err| err.to_string())?;
    Ok(Box::new(MediaFoundationWriter(writer)))
}

#[cfg(not(windows))]
mod ffmpeg {
    //! 把 BGRA 原始帧通过标准输入交给 `ffmpeg`，由它编码成 H.264 MP4。

    use std::ffi::OsString;
    use std::io::Write as _;
    use std::path::Path;
    use std::process::{Child, ChildStdin, Command, Stdio};

    /// 在 PATH 与常见 Homebrew/MacPorts 目录中查找 `ffmpeg`。
    pub fn command() -> Option<OsString> {
        if let Some(path) = std::env::var_os("PATH") {
            for directory in std::env::split_paths(&path) {
                let candidate = directory.join("ffmpeg");
                if candidate.is_file() {
                    return Some(candidate.into_os_string());
                }
            }
        }
        // 从 Finder / 桌面图标启动的 macOS 应用拿不到 Homebrew 的 PATH。
        for candidate in [
            "/opt/homebrew/bin/ffmpeg",
            "/usr/local/bin/ffmpeg",
            "/opt/local/bin/ffmpeg",
            "/usr/bin/ffmpeg",
        ] {
            let path = Path::new(candidate);
            if path.is_file() {
                return Some(path.as_os_str().to_os_string());
            }
        }
        None
    }

    /// 一个以管道方式运行的 ffmpeg 编码会话。
    pub struct Encoder {
        child: Child,
        stdin: Option<ChildStdin>,
    }

    impl Encoder {
        /// 启动 ffmpeg。输出为 H.264 + yuv420p，并启用 faststart 便于播放器边下边播。
        pub fn create(path: &Path, width: u32, height: u32, fps: u32) -> Result<Self, String> {
            let executable = command().ok_or_else(|| "未找到 ffmpeg".to_string())?;
            let size = format!("{width}x{height}");
            let rate = fps.max(1).to_string();
            let mut child = Command::new(executable)
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-y",
                    "-f",
                    "rawvideo",
                    "-pix_fmt",
                    "bgra",
                    "-s",
                    &size,
                    "-r",
                    &rate,
                    "-i",
                    "pipe:0",
                    "-an",
                    "-c:v",
                    "libx264",
                    "-preset",
                    "veryfast",
                    // libx264 的 yuv420p 要求宽高为偶数；摄像头常见奇数分辨率时兜底裁 1px。
                    "-vf",
                    "scale=trunc(iw/2)*2:trunc(ih/2)*2",
                    "-pix_fmt",
                    "yuv420p",
                    "-movflags",
                    "+faststart",
                ])
                .arg(path)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|err| format!("启动 ffmpeg 失败：{err}"))?;
            let stdin = child
                .stdin
                .take()
                .ok_or_else(|| "无法打开 ffmpeg 标准输入".to_string())?;
            Ok(Self {
                child,
                stdin: Some(stdin),
            })
        }

        pub fn write_frame(&mut self, bgra: &[u8]) -> Result<(), String> {
            let stdin = self
                .stdin
                .as_mut()
                .ok_or_else(|| "ffmpeg 输入已关闭".to_string())?;
            stdin
                .write_all(bgra)
                .map_err(|err| format!("写入 ffmpeg 失败：{err}"))
        }

        pub fn finish(mut self) -> Result<(), String> {
            drop(self.stdin.take());
            let output = self
                .child
                .wait_with_output()
                .map_err(|err| format!("等待 ffmpeg 退出失败：{err}"))?;
            if output.status.success() {
                return Ok(());
            }
            let stderr = String::from_utf8_lossy(&output.stderr);
            let detail = stderr.trim();
            if detail.is_empty() {
                Err(format!(
                    "ffmpeg 退出码：{}",
                    output.status.code().unwrap_or(-1)
                ))
            } else {
                Err(format!("ffmpeg：{detail}"))
            }
        }
    }
}

#[cfg(not(windows))]
impl FrameWriter for ffmpeg::Encoder {
    fn write_frame(&mut self, bgra: &[u8]) -> Result<(), String> {
        ffmpeg::Encoder::write_frame(self, bgra)
    }

    fn finish(self: Box<Self>) -> Result<(), String> {
        ffmpeg::Encoder::finish(*self)
    }
}

#[cfg(not(windows))]
fn open_writer(
    path: &Path,
    width: u32,
    height: u32,
    fps: u32,
) -> Result<Box<dyn FrameWriter>, String> {
    let encoder = ffmpeg::Encoder::create(path, width, height, fps)?;
    Ok(Box::new(encoder))
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
    let mut writer = open_writer(path, width, height, fps)?;

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
        if next_tick as f64 * frame_interval + 1.0 < elapsed {
            next_tick = (elapsed / frame_interval).floor() as u64;
        }
        while next_tick as f64 * frame_interval <= elapsed {
            let Some(pixels) = last.as_deref() else {
                break;
            };
            writer.write_frame(pixels)?;
            frames.fetch_add(1, Ordering::Relaxed);
            next_tick = next_tick.saturating_add(1);
        }
    }

    writer.finish()?;
    Ok(path.to_path_buf())
}

fn prepare_frame(frame: &capture::Frame, width: u32, height: u32) -> Vec<u8> {
    if frame.width == width && frame.height == height {
        return frame.data.clone();
    }
    imaging::resize_bgra(frame.width, frame.height, &frame.data, width, height)
        .map_or_else(|_| frame.data.clone(), |(_, _, data)| data)
}
