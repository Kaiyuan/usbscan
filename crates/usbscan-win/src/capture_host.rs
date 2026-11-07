// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 采集线程与 UI 之间的桥接。
//!
//! 采集线程独立于消息循环：它负责枚举设备、打开摄像头、循环取帧，
//! 只通过共享状态 + `PostMessage` 与 UI 通信，绝不直接操作窗口。

use std::fs;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use usbscan_capture as capture;
use windows_sys::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

/// 采集线程通知 UI：有新帧。
pub const WM_APP_FRAME: u32 = 0x8000 + 1;
/// 采集线程通知 UI：状态变化。
pub const WM_APP_STATUS: u32 = 0x8000 + 2;
/// 采集线程通知 UI：设备列表变化。
pub const WM_APP_DEVICES: u32 = 0x8000 + 3;

/// 摄像头连接状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// 正在枚举或重试。
    Connecting,
    /// 枚举结果为空。
    NoDevices,
    /// 已打开，正在取帧。
    Ready {
        id: String,
        name: String,
        width: u32,
        height: u32,
    },
    /// 已选设备当前不在线，等待重新插入。
    Waiting { name: String },
    /// 打开或取帧失败。
    Error(String),
}

/// 采集线程与 UI 共享的数据。
#[derive(Default)]
pub struct Shared {
    /// 最新一帧。用 `Arc` 共享，预览与落盘都不必复制像素。
    pub frame: Option<Arc<capture::Frame>>,
    pub status: Option<Status>,
    /// 当前帧序号，用于预览判断是否重绘。
    pub sequence: u64,
    /// 是否已投递 `WM_APP_FRAME`；用于合并高频帧通知。
    pub frame_pending: bool,
    /// 最近一次枚举到的设备列表。
    pub devices: Vec<capture::CameraInfo>,
    /// 设备列表版本号；UI 只在版本变化时重建下拉框。
    pub devices_version: u64,
    /// 用户选择的设备；`None` 表示自动选择第一台。
    pub selected: Option<String>,
    /// 用户选择的设备名，用于"等待设备"提示。
    pub selected_name: Option<String>,
    /// 重启请求：选择设备、热插拔后由 UI 置位。
    restart: bool,
}

/// 采集宿主：持有线程句柄与共享状态。
pub struct CaptureHost {
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl CaptureHost {
    #[must_use]
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Mutex::new(Shared::default())),
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        }
    }

    /// 启动采集线程。
    ///
    /// `hwnd` 以 `isize` 传入，避免裸指针跨线程的类型约束；
    /// 调用方必须保证窗口在线程退出之后才销毁。
    pub fn start(&mut self, hwnd: isize) {
        if self.thread.is_some() {
            return;
        }
        self.stop.store(false, Ordering::SeqCst);
        let shared = Arc::clone(&self.shared);
        let stop = Arc::clone(&self.stop);
        self.thread = Some(thread::spawn(move || worker(hwnd, &shared, &stop)));
    }

    /// 请求停止并等待线程退出。
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }

    /// 读取共享状态；锁中毒时继续使用内部数据。
    pub fn shared(&self) -> MutexGuard<'_, Shared> {
        self.shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// 取最近一帧（用于预览与拍摄落盘）。
    #[must_use]
    pub fn latest_frame(&self) -> Option<Arc<capture::Frame>> {
        self.shared().frame.clone()
    }

    /// 选择要打开的摄像头；`None` 表示自动选择第一台。
    pub fn select(&self, id: Option<String>, name: Option<String>) {
        let mut shared = self.shared();
        shared.selected = id;
        shared.selected_name = name;
        shared.restart = true;
    }

    /// 请求采集线程重新枚举设备（设备热插拔后调用）。
    pub fn refresh(&self) {
        self.shared().restart = true;
    }
}

impl Default for CaptureHost {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for CaptureHost {
    fn drop(&mut self) {
        self.stop();
    }
}

#[allow(clippy::too_many_lines)]
fn worker(hwnd_value: isize, shared: &Arc<Mutex<Shared>>, stop: &Arc<AtomicBool>) {
    let _com = capture::init_thread();
    let hwnd = hwnd_value as HWND;

    if let Err(err) = capture::ensure_platform() {
        set_status(shared, stop, hwnd, Status::Error(err.to_string()));
        return;
    }

    while !stop.load(Ordering::SeqCst) {
        // 重试前清空旧帧，避免界面继续显示已经断开的摄像头画面。
        // 同时消费重启请求：本轮的枚举就是为它服务的，避免重复重开。
        {
            let mut guard = shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.frame = None;
            guard.restart = false;
        }

        let cameras = match capture::list_cameras() {
            Ok(list) => {
                publish_devices(shared, stop, hwnd, &list);
                list
            }
            Err(err) => {
                set_status(shared, stop, hwnd, Status::Error(err.to_string()));
                sleep_interruptible(stop, Duration::from_millis(1000));
                continue;
            }
        };

        let (selected, selected_name) = {
            let guard = shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (guard.selected.clone(), guard.selected_name.clone())
        };

        // 优先精确匹配稳定 ID；旧版配置里保存的是 `名称#序号`，
        // 或设备链接在系统更新后变化时，再按名称找回同一台设备。
        let candidates = choose_candidates(&cameras, selected.as_deref(), selected_name.as_deref());

        let Some(&first) = candidates.first() else {
            // 用户已经指定设备时，不擅自打开另一台摄像头，等待它重新上线。
            if selected.is_some() {
                let name = selected_name
                    .or(selected)
                    .unwrap_or_else(|| "摄像头".to_string());
                set_status(shared, stop, hwnd, Status::Waiting { name });
            } else {
                set_status(shared, stop, hwnd, Status::NoDevices);
            }
            sleep_interruptible(stop, Duration::from_millis(1000));
            continue;
        };

        let mut opened = None;
        let mut last_error = None;
        for info in &candidates {
            match capture::Camera::open(info) {
                Ok(camera) => {
                    opened = Some((camera, *info));
                    break;
                }
                Err(err) => last_error = Some(err),
            }
        }

        let Some((mut camera, info)) = opened else {
            let message = last_error.map_or_else(
                || format!("无法打开摄像头：{}", first.name),
                |err| err.to_string(),
            );
            set_status(shared, stop, hwnd, Status::Error(message));
            sleep_interruptible(stop, Duration::from_millis(1000));
            continue;
        };
        log_line(&format!("使用 {} 打开 {}", camera.backend(), info.name));

        // 记录实际打开的稳定 ID，后续重启或热插拔时不必再靠名称猜测。
        {
            let mut guard = shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if guard.selected.as_deref() != Some(info.id.as_str()) {
                guard.selected = Some(info.id.clone());
                guard.selected_name = Some(info.name.clone());
            }
        }

        let (width, height) = camera.size();
        set_status(
            shared,
            stop,
            hwnd,
            Status::Ready {
                id: info.id.clone(),
                name: info.name.clone(),
                width,
                height,
            },
        );

        while !stop.load(Ordering::SeqCst) {
            // 选择设备或设备热插拔后，关闭当前流并回到枚举阶段。
            let restart = {
                let mut guard = shared
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                std::mem::take(&mut guard.restart)
            };
            if restart {
                break;
            }

            match camera.read_frame() {
                Ok(frame) => {
                    let should_post = {
                        let mut guard = shared
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        guard.frame = Some(Arc::new(frame));
                        guard.sequence = guard.sequence.wrapping_add(1);
                        // 上一帧的 WM_APP_FRAME 还没处理时不再重复投递，
                        // 避免高帧率下消息队列堆积、界面来回闪。
                        let should_post = !guard.frame_pending;
                        guard.frame_pending = true;
                        should_post
                    };
                    if should_post {
                        // SAFETY: 窗口在 CaptureHost::stop() 之前保持有效。
                        unsafe {
                            PostMessageW(hwnd, WM_APP_FRAME, 0, 0);
                        }
                    }
                }
                Err(err) => {
                    // 格式切换或流事件期间可能短暂没有样本，保持上一帧继续读，
                    // 不要因此清空预览并重开设备。
                    if matches!(err, capture::CaptureError::NoFrame) {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    let message = err.to_string();
                    {
                        let mut guard = shared
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        guard.frame = None;
                    }
                    set_status(shared, stop, hwnd, Status::Error(message));
                    sleep_interruptible(stop, Duration::from_millis(600));
                    break;
                }
            }
        }
    }
}

fn publish_devices(
    shared: &Arc<Mutex<Shared>>,
    stop: &AtomicBool,
    hwnd: HWND,
    cameras: &[capture::CameraInfo],
) {
    let changed = {
        let mut guard = shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.devices.as_slice() == cameras {
            false
        } else {
            guard.devices = cameras.to_vec();
            guard.devices_version = guard.devices_version.wrapping_add(1);
            true
        }
    };
    if changed {
        let summary = cameras
            .iter()
            .map(|camera| format!("{} [{}]", camera.name, camera.id))
            .collect::<Vec<_>>()
            .join("; ");
        log_line(&format!("设备列表（{}）：{}", cameras.len(), summary));
    }
    if changed && !stop.load(Ordering::SeqCst) {
        // SAFETY: 窗口在 CaptureHost::stop() 之前保持有效。
        unsafe {
            PostMessageW(hwnd, WM_APP_DEVICES, 0, 0);
        }
    }
}

fn set_status(shared: &Arc<Mutex<Shared>>, stop: &Arc<AtomicBool>, hwnd: HWND, status: Status) {
    if stop.load(Ordering::SeqCst) {
        return;
    }
    let changed = {
        let guard = shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.status.as_ref() != Some(&status)
    };
    if !changed {
        return;
    }
    log_status(&status);
    {
        let mut guard = shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.status = Some(status);
    }
    // SAFETY: 窗口在 CaptureHost::stop() 之前保持有效。
    unsafe {
        PostMessageW(hwnd, WM_APP_STATUS, 0 as WPARAM, 0 as LPARAM);
    }
}

/// 把状态变化追加到 `%APPDATA%\usbscan\usbscan.log`。
///
/// 日志只记录设备状态，不含画面或用户文档内容，便于用户反馈
/// "摄像头打不开"时定位是权限、占用还是枚举问题。
fn log_status(status: &Status) {
    log_line(&format!("{status:?}"));
}

fn log_line(text: &str) {
    let Some(appdata) = std::env::var_os("APPDATA") else {
        return;
    };
    let dir = PathBuf::from(appdata).join("usbscan");
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("usbscan.log");
    if fs::metadata(&path).is_ok_and(|metadata| metadata.len() > 1_000_000) {
        let _ = fs::remove_file(&path);
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let line = format!("[{timestamp}] {text}\n");
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = file.write_all(line.as_bytes());
    }
}

/// 根据保存的选择挑出本次要尝试的设备。
///
/// 精确 ID 优先；旧配置或设备链接变化时按名称回退；没有选择时依次尝试全部。
fn choose_candidates<'a>(
    cameras: &'a [capture::CameraInfo],
    selected: Option<&str>,
    selected_name: Option<&str>,
) -> Vec<&'a capture::CameraInfo> {
    let mut candidates = Vec::new();
    if let Some(id) = selected {
        if let Some(camera) = cameras.iter().find(|camera| camera.id == id) {
            candidates.push(camera);
        } else if let Some(name) = selected_name {
            candidates.extend(cameras.iter().filter(|camera| camera.name == name));
        }
    } else {
        candidates.extend(cameras.iter());
    }
    candidates
}

fn sleep_interruptible(stop: &AtomicBool, duration: Duration) {
    const STEP: Duration = Duration::from_millis(50);
    let mut waited = Duration::ZERO;
    while waited < duration {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        thread::sleep(STEP);
        waited += STEP;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_stores_device_and_requests_restart() {
        let host = CaptureHost::new();
        host.select(
            Some("usb#vid_1234".to_string()),
            Some("测试摄像头".to_string()),
        );

        let shared = host.shared();
        assert_eq!(shared.selected.as_deref(), Some("usb#vid_1234"));
        assert_eq!(shared.selected_name.as_deref(), Some("测试摄像头"));
        assert!(shared.restart);
    }

    #[test]
    fn refresh_only_requests_restart() {
        let host = CaptureHost::new();
        host.refresh();

        let shared = host.shared();
        assert!(shared.restart);
        assert!(shared.selected.is_none());
    }

    fn sample_cameras() -> Vec<capture::CameraInfo> {
        vec![
            capture::CameraInfo {
                id: "link-a".to_string(),
                name: "摄像头 A".to_string(),
            },
            capture::CameraInfo {
                id: "link-b".to_string(),
                name: "摄像头 B".to_string(),
            },
        ]
    }

    #[test]
    fn exact_id_wins() {
        let cameras = sample_cameras();
        let chosen = choose_candidates(&cameras, Some("link-b"), Some("摄像头 A"));
        assert_eq!(chosen.len(), 1);
        assert_eq!(chosen[0].name, "摄像头 B");
    }

    #[test]
    fn stale_id_falls_back_to_name() {
        let cameras = sample_cameras();
        // 旧版配置保存的是 `名称#序号`，新版本使用符号链接。
        let chosen = choose_candidates(&cameras, Some("摄像头 A#0"), Some("摄像头 A"));
        assert_eq!(chosen.len(), 1);
        assert_eq!(chosen[0].id, "link-a");
    }

    #[test]
    fn no_selection_tries_every_camera() {
        let cameras = sample_cameras();
        assert_eq!(choose_candidates(&cameras, None, None).len(), 2);
    }

    #[test]
    fn missing_device_stays_empty() {
        let cameras = sample_cameras();
        let chosen = choose_candidates(&cameras, Some("gone"), Some("不存在的摄像头"));
        let empty: Vec<&capture::CameraInfo> = Vec::new();
        assert_eq!(chosen, empty);
    }
}
