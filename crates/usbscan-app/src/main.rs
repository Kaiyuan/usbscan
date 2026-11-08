// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]

use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use slint::{Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};
use usbscan_capture as capture;
use usbscan_doc as doc;
use usbscan_export as export;
use usbscan_imaging as imaging;
use usbscan_naming as naming;
use usbscan_ocr as ocr;
use usbscan_win::config::Config;
use usbscan_win::recorder::Recorder;
use usbscan_win::theme;

slint::include_modules!();

const CAMERA_SOUND: &[u8] = include_bytes!("../../../assets/camera.mp3");
const RECORD_SOUND: &[u8] = include_bytes!("../../../assets/record.mp3");

struct Audio {
    _stream: OutputStream,
    handle: OutputStreamHandle,
}

impl Audio {
    fn new() -> Option<Self> {
        match OutputStream::try_default() {
            Ok((stream, handle)) => Some(Self {
                _stream: stream,
                handle,
            }),
            Err(err) => {
                eprintln!("audio output unavailable: {err}");
                None
            }
        }
    }

    fn play(&self, bytes: &'static [u8]) {
        let Ok(source) = Decoder::new(std::io::Cursor::new(bytes)) else {
            return;
        };
        let Ok(sink) = Sink::try_new(&self.handle) else {
            return;
        };
        sink.append(source);
        sink.detach();
    }
}

const DETECT_INTERVAL: Duration = Duration::from_millis(300);
const PREVIEW_MAX_SIDE: u32 = 1280;
const DETECT_MAX_SIDE: u32 = 480;

// ---------------------------------------------------------------------------
// 采集线程
// ---------------------------------------------------------------------------

struct WorkerShared {
    frame: Option<Arc<capture::Frame>>,
    sequence: u64,
    devices: Vec<capture::CameraInfo>,
    devices_version: u64,
    formats: Vec<(u32, u32)>,
    formats_version: u64,
    status: String,
    connected: bool,
    corners: Option<[(f64, f64); 4]>,
    corners_frame: Option<(u32, u32)>,
    frame_size: Option<(u32, u32)>,
    selected: Option<String>,
    selected_name: Option<String>,
    preferred_format: Option<(u32, u32)>,
    auto_crop: bool,
    restart: bool,
    last_detect: Option<Instant>,
    record_sink: Option<SyncSender<Arc<capture::Frame>>>,
}

impl Default for WorkerShared {
    fn default() -> Self {
        Self {
            frame: None,
            sequence: 0,
            devices: Vec::new(),
            devices_version: 0,
            formats: Vec::new(),
            formats_version: 0,
            status: "正在检测摄像头…".to_string(),
            connected: false,
            corners: None,
            corners_frame: None,
            frame_size: None,
            selected: None,
            selected_name: None,
            preferred_format: None,
            auto_crop: true,
            restart: false,
            last_detect: None,
            record_sink: None,
        }
    }
}

struct Worker {
    shared: Arc<Mutex<WorkerShared>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    fn start() -> Self {
        let shared = Arc::new(Mutex::new(WorkerShared::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_shared = Arc::clone(&shared);
        let thread_stop = Arc::clone(&stop);
        let thread = thread::spawn(move || worker_loop(&thread_shared, &thread_stop));
        Self {
            shared,
            stop,
            thread: Some(thread),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, WorkerShared> {
        self.shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn select_device(&self, id: String, name: String) {
        let mut shared = self.lock();
        shared.selected = Some(id);
        shared.selected_name = Some(name);
        shared.restart = true;
    }

    fn select_format(&self, format: Option<(u32, u32)>) {
        let mut shared = self.lock();
        shared.preferred_format = format;
        shared.restart = true;
    }

    fn refresh(&self) {
        self.lock().restart = true;
    }

    fn set_auto_crop(&self, enabled: bool) {
        self.lock().auto_crop = enabled;
    }

    fn set_record_sink(&self, sink: Option<SyncSender<Arc<capture::Frame>>>) {
        self.lock().record_sink = sink;
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

#[allow(clippy::too_many_lines)]
fn worker_loop(shared: &Arc<Mutex<WorkerShared>>, stop: &AtomicBool) {
    let _com = capture::init_thread();
    if let Err(err) = capture::ensure_platform() {
        set_status(shared, false, format!("Media Foundation 初始化失败：{err}"));
        return;
    }

    while !stop.load(Ordering::SeqCst) {
        {
            let mut guard = shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.frame = None;
            guard.corners = None;
            guard.restart = false;
        }
        let cameras = match capture::list_cameras() {
            Ok(cameras) => cameras,
            Err(err) => {
                set_status(shared, false, format!("枚举摄像头失败：{err}"));
                sleep_interruptible(stop, Duration::from_millis(1000));
                continue;
            }
        };
        publish_devices(shared, &cameras);

        let (selected, selected_name, preferred) = {
            let guard = shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (
                guard.selected.clone(),
                guard.selected_name.clone(),
                guard.preferred_format,
            )
        };
        if selected.is_some()
            && cameras
                .iter()
                .all(|camera| Some(&camera.id) != selected.as_ref())
        {
            set_status(
                shared,
                false,
                format!(
                    "等待摄像头：{}",
                    selected_name.unwrap_or_else(|| "已保存的设备".to_string())
                ),
            );
            sleep_interruptible(stop, Duration::from_millis(1000));
            continue;
        }
        let Some(info) = select_camera(&cameras, selected.as_deref(), selected_name.as_deref())
        else {
            set_status(
                shared,
                false,
                "未检测到摄像头，请插入 USB 摄像头".to_string(),
            );
            sleep_interruptible(stop, Duration::from_millis(1000));
            continue;
        };

        let mut camera = match capture::Camera::open(info, preferred) {
            Ok(camera) => camera,
            Err(err) => {
                set_status(shared, false, format!("无法打开摄像头：{err}"));
                sleep_interruptible(stop, Duration::from_millis(1000));
                continue;
            }
        };
        let formats = camera.formats();
        let (width, height) = camera.size();
        publish_formats(shared, &formats);
        {
            let mut guard = shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.selected = Some(info.id.clone());
            guard.selected_name = Some(info.name.clone());
            guard.connected = true;
            guard.status = format!("已连接 {}  {}×{}", info.name, width, height);
            guard.corners_frame = Some((width, height));
        }

        while !stop.load(Ordering::SeqCst) {
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
                    let frame = Arc::new(trim_frame(frame));
                    let (record_sink, auto_crop) = {
                        let mut guard = shared
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        if guard.frame_size != Some((frame.width, frame.height)) {
                            guard.frame_size = Some((frame.width, frame.height));
                            if guard.status.starts_with("已连接 ") {
                                if let Some(name) = &guard.selected_name {
                                    guard.status =
                                        format!("已连接 {name}  {}×{}", frame.width, frame.height);
                                }
                            }
                        }
                        guard.frame = Some(Arc::clone(&frame));
                        guard.sequence = guard.sequence.wrapping_add(1);
                        (guard.record_sink.clone(), guard.auto_crop)
                    };
                    if let Some(sink) = record_sink {
                        let _ = sink.try_send(Arc::clone(&frame));
                    }
                    if auto_crop {
                        maybe_detect(shared, &frame);
                    }
                }
                Err(capture::CaptureError::NoFrame) => {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(err) => {
                    {
                        let mut guard = shared
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        guard.frame = None;
                        guard.corners = None;
                        guard.connected = false;
                        guard.status = format!("摄像头错误：{err}");
                    }
                    sleep_interruptible(stop, Duration::from_millis(600));
                    break;
                }
            }
        }
    }
}

fn trim_frame(frame: capture::Frame) -> capture::Frame {
    let Some((_x, _y, width, height, data)) =
        imaging::trim_letterbox(frame.width, frame.height, &frame.data)
    else {
        return frame;
    };
    capture::Frame {
        width,
        height,
        format: frame.format,
        data,
        timestamp: frame.timestamp,
    }
}

fn maybe_detect(shared: &Arc<Mutex<WorkerShared>>, frame: &capture::Frame) {
    let now = Instant::now();
    {
        let mut guard = shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard
            .last_detect
            .is_some_and(|last| now.duration_since(last) < DETECT_INTERVAL)
        {
            return;
        }
        guard.last_detect = Some(now);
    }
    let scale = (f64::from(DETECT_MAX_SIDE) / f64::from(frame.width.max(1)))
        .min(f64::from(DETECT_MAX_SIDE) / f64::from(frame.height.max(1)))
        .min(1.0);
    let (small_width, small_height, small_data) = if scale < 1.0 {
        match imaging::thumbnail_bgra(
            frame.width,
            frame.height,
            &frame.data,
            DETECT_MAX_SIDE,
            DETECT_MAX_SIDE,
        ) {
            Ok(result) => result,
            Err(_) => return,
        }
    } else {
        (frame.width, frame.height, frame.data.clone())
    };
    let corners = imaging::detect_document_corners(small_width, small_height, &small_data);
    let mut guard = shared
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.corners_frame = Some((frame.width, frame.height));
    guard.corners = corners.map(|corners| {
        corners.map(|(x, y)| {
            (
                x * f64::from(frame.width) / f64::from(small_width.max(1)),
                y * f64::from(frame.height) / f64::from(small_height.max(1)),
            )
        })
    });
}

fn publish_devices(shared: &Arc<Mutex<WorkerShared>>, cameras: &[capture::CameraInfo]) {
    let mut guard = shared
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if guard.devices != cameras {
        guard.devices = cameras.to_vec();
        guard.devices_version = guard.devices_version.wrapping_add(1);
    }
}

fn publish_formats(shared: &Arc<Mutex<WorkerShared>>, formats: &[(u32, u32)]) {
    let mut guard = shared
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if guard.formats != formats {
        guard.formats = formats.to_vec();
        guard.formats_version = guard.formats_version.wrapping_add(1);
    }
}

fn set_status(shared: &Arc<Mutex<WorkerShared>>, connected: bool, status: String) {
    let mut guard = shared
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.connected = connected;
    guard.status = status;
}

fn select_camera<'a>(
    cameras: &'a [capture::CameraInfo],
    id: Option<&str>,
    name: Option<&str>,
) -> Option<&'a capture::CameraInfo> {
    id.and_then(|id| cameras.iter().find(|camera| camera.id == id))
        .or_else(|| name.and_then(|name| cameras.iter().find(|camera| camera.name == name)))
        .or_else(|| cameras.first())
}

fn sleep_interruptible(stop: &AtomicBool, duration: Duration) {
    let sleep_step = Duration::from_millis(50);
    let mut elapsed = Duration::ZERO;
    while elapsed < duration && !stop.load(Ordering::SeqCst) {
        thread::sleep(sleep_step);
        elapsed += sleep_step;
    }
}

// ---------------------------------------------------------------------------
// 应用状态
// ---------------------------------------------------------------------------

struct AppState {
    ui: slint::Weak<MainWindow>,
    worker: Worker,
    config: Config,
    session: doc::Session,
    store: doc::SessionStore,
    output_root: PathBuf,
    rule: naming::NamingRule,
    devices: Vec<capture::CameraInfo>,
    formats: Vec<(u32, u32)>,
    rows: Vec<usize>,
    selected_row: Option<usize>,
    thumb_cache: HashMap<u64, Image>,
    last_sequence: u64,
    last_devices_version: u64,
    last_formats_version: u64,
    last_status: String,
    last_auto_crop: bool,
    recording: Option<Recorder>,
    rename_row: Option<usize>,
    audio: Option<Audio>,
    timer: Option<slint::Timer>,
}

impl AppState {
    fn ui(&self) -> Option<MainWindow> {
        self.ui.upgrade()
    }

    fn frame(&self) -> Option<Arc<capture::Frame>> {
        self.worker.lock().frame.clone()
    }

    fn capture_feedback(&mut self) -> bool {
        if self.frame().is_none() {
            self.set_status("尚未取得画面，请稍候再拍");
            return false;
        }
        self.play_sound(CAMERA_SOUND);
        self.flash();
        self.show_toast("已拍照");
        true
    }

    fn capture_now(&mut self) {
        let Some(frame) = self.frame() else {
            self.set_status("尚未取得画面，请稍候再拍");
            return;
        };
        let ui = self.ui();
        let options = self.enhance_options();
        let enhanced = self.enhance_frame(&frame, options);
        let _ = ui;
        let path = self.new_image_path(enhanced.width, enhanced.height);
        if let Some(parent) = path.parent() {
            if let Err(err) = fs::create_dir_all(parent) {
                self.set_status(&format!("创建目录失败：{err}"));
                return;
            }
        }
        if let Err(err) = write_jpeg_atomic(
            &path,
            enhanced.width,
            enhanced.height,
            &enhanced.bgra,
            self.config.jpeg_quality.unwrap_or(92),
        ) {
            self.set_status(&format!("保存图片失败：{err}"));
            return;
        }
        let raw_relative = self.write_raw(&frame);
        let relative = path
            .strip_prefix(&self.output_root)
            .unwrap_or(&path)
            .to_path_buf();
        let seq = self.session.next_seq;
        let page = doc::Page {
            seq,
            path: relative,
            raw: raw_relative,
            width: enhanced.width,
            height: enhanced.height,
            captured_at: doc::now_unix(),
            device: self
                .worker
                .lock()
                .selected_name
                .clone()
                .or_else(|| Some("Web Camera".to_string())),
        };
        self.session.pages.push(page);
        self.session.next_seq = seq.saturating_add(1);
        self.session.updated_at = doc::now_unix();
        self.session.closed_cleanly = false;
        let _ = self.store.save(&self.session);
        self.rebuild_pages();
        self.selected_row = self.rows.len().checked_sub(1);
        if let Some(ui) = ui {
            ui.set_selected_page(self.selected_row.map_or(-1, |row| row as i32));
        }
        self.set_status(&format!(
            "已保存：{}（{}×{}）",
            path.file_name().unwrap_or_default().to_string_lossy(),
            enhanced.width,
            enhanced.height
        ));
    }

    fn enhance_frame(
        &self,
        frame: &capture::Frame,
        options: imaging::EnhanceOptions,
    ) -> imaging::EnhancedImage {
        let original = imaging::EnhancedImage {
            width: frame.width,
            height: frame.height,
            bgra: frame.data.clone(),
        };
        let (corners, corners_frame) = {
            let guard = self.worker.lock();
            (guard.corners, guard.corners_frame)
        };
        let use_corners = corners_frame == Some((frame.width, frame.height))
            && corners.is_some()
            && (options.auto_crop || options.perspective);
        let mut image = if use_corners {
            match imaging::perspective_from_corners(
                frame.width,
                frame.height,
                &frame.data,
                &corners.unwrap(),
            ) {
                Ok((width, height, bgra)) => imaging::EnhancedImage {
                    width,
                    height,
                    bgra,
                },
                Err(_) => original,
            }
        } else if options.is_active() {
            imaging::enhance(frame.width, frame.height, &frame.data, options).unwrap_or(original)
        } else {
            original
        };
        if use_corners && (options.remove_shadow || options.white_background) {
            let post = imaging::EnhanceOptions {
                remove_shadow: options.remove_shadow,
                white_background: options.white_background,
                strength: options.strength,
                ..imaging::EnhanceOptions::default()
            };
            if let Ok(processed) = imaging::enhance(image.width, image.height, &image.bgra, post) {
                image = processed;
            }
        }
        image
    }

    fn enhance_options(&self) -> imaging::EnhanceOptions {
        let ui = self.ui();
        let (auto_crop, deskew, perspective, remove_shadow, white_background, strength) = ui
            .as_ref()
            .map_or((true, true, true, false, false, 60.0), |ui| {
                (
                    ui.get_auto_crop(),
                    ui.get_auto_deskew(),
                    ui.get_perspective(),
                    ui.get_remove_shadow(),
                    ui.get_white_background(),
                    ui.get_strength(),
                )
            });
        imaging::EnhanceOptions {
            auto_crop,
            perspective: perspective || auto_crop,
            deskew,
            remove_shadow,
            white_background,
            strength: strength.round().clamp(0.0, 100.0) as u8,
        }
    }

    fn new_image_path(&self, width: u32, height: u32) -> PathBuf {
        let seq = self.session.next_seq;
        let device = self.worker.lock().selected_name.clone();
        let context = naming::NamingContext {
            now: naming::TimeStamp::now(),
            device: device.as_deref(),
            width: Some(width),
            height: Some(height),
            seq,
            ..naming::NamingContext::default()
        };
        let engine = naming::NamingEngine::default();
        let rendered = engine.render(&self.rule, &context).map_or_else(
            |_| {
                self.output_root.join(format!(
                    "扫描_{}.jpg",
                    naming::TimeStamp::now().format("yyyyMMdd_HHmmss")
                ))
            },
            |rendered| rendered.path(&self.output_root, "jpg"),
        );
        unique_path(&rendered)
    }

    fn new_import_path(&self) -> PathBuf {
        let now = naming::TimeStamp::now();
        unique_path(
            &self
                .output_root
                .join(format!("导入_{}.jpg", now.format("yyyyMMdd_HHmmss"))),
        )
    }

    fn write_raw(&mut self, frame: &capture::Frame) -> Option<PathBuf> {
        let seq = self.session.next_seq;
        let raw_dir = self.output_root.join(".usbscan").join("raw");
        let _ = fs::create_dir_all(&raw_dir);
        let name = format!("{seq:06}_{}.jpg", doc::now_unix());
        let path = raw_dir.join(name);
        if write_jpeg_atomic(
            &path,
            frame.width,
            frame.height,
            &frame.data,
            self.config.jpeg_quality.unwrap_or(92).min(92),
        )
        .is_ok()
        {
            path.strip_prefix(&self.output_root)
                .ok()
                .map(Path::to_path_buf)
        } else {
            None
        }
    }

    fn rebuild_pages(&mut self) {
        let Some(ui) = self.ui() else {
            return;
        };
        let mut items = Vec::with_capacity(self.session.pages.len());
        let mut rows = Vec::with_capacity(self.session.pages.len());
        let pages = self.session.pages.clone();
        for (index, page) in pages.iter().enumerate().rev() {
            rows.push(index);
            let title = page.path.file_name().map_or_else(
                || page.path.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            );
            items.push(PageItem {
                seq: page.seq as i32,
                title: title.into(),
                thumb: self.thumbnail(page),
            });
        }
        self.rows = rows;
        ui.set_pages(ModelRc::from(Rc::new(VecModel::from(items))));
        ui.set_page_count_text(format!("{} 页", self.session.pages.len()).into());
        let selected = self
            .selected_row
            .filter(|row| *row < self.rows.len())
            .map_or(-1, |row| row as i32);
        ui.set_selected_page(selected);
    }

    fn thumbnail(&mut self, page: &doc::Page) -> Image {
        if let Some(image) = self.thumb_cache.get(&page.seq) {
            return image.clone();
        }
        let path = self.output_root.join(&page.path);
        let image = imaging::load_bgra(&path)
            .ok()
            .and_then(|(width, height, data)| {
                imaging::thumbnail_bgra(width, height, &data, 144, 108).ok()
            })
            .map_or_else(Image::default, |(width, height, data)| {
                bgra_to_image(width, height, &data)
            });
        self.thumb_cache.insert(page.seq, image.clone());
        image
    }

    fn poll(&mut self) {
        let Some(ui) = self.ui() else {
            return;
        };
        let (
            sequence,
            frame,
            status,
            connected,
            devices,
            devices_version,
            formats,
            formats_version,
            corners,
            corners_frame,
        ) = {
            let guard = self.worker.lock();
            (
                guard.sequence,
                guard.frame.clone(),
                guard.status.clone(),
                guard.connected,
                guard.devices.clone(),
                guard.devices_version,
                guard.formats.clone(),
                guard.formats_version,
                guard.corners,
                guard.corners_frame,
            )
        };
        if devices_version != self.last_devices_version {
            self.last_devices_version = devices_version;
            self.devices = devices;
            let names: Vec<slint::SharedString> = self
                .devices
                .iter()
                .map(|camera| camera.name.clone().into())
                .collect();
            ui.set_device_options(ModelRc::from(Rc::new(VecModel::from(names))));
            ui.set_device_index(0);
        }
        if formats_version != self.last_formats_version {
            self.last_formats_version = formats_version;
            self.formats = formats;
            let mut names = vec![slint::SharedString::from("自动")];
            names.extend(
                self.formats
                    .iter()
                    .map(|(width, height)| format!("{width} × {height}").into()),
            );
            ui.set_format_options(ModelRc::from(Rc::new(VecModel::from(names))));
            let preferred = self.worker.lock().preferred_format;
            let index = preferred
                .and_then(|preferred| self.formats.iter().position(|format| *format == preferred))
                .map_or(0, |index| index + 1);
            ui.set_format_index(index as i32);
        }
        if status != self.last_status {
            self.last_status.clone_from(&status);
            ui.set_status_text(status.into());
        }
        ui.set_has_camera(connected);
        ui.set_recording(self.recording.is_some());
        let auto_crop = ui.get_auto_crop();
        if auto_crop != self.last_auto_crop {
            self.last_auto_crop = auto_crop;
            self.worker.set_auto_crop(auto_crop);
        }
        if sequence != self.last_sequence {
            self.last_sequence = sequence;
            if let Some(frame) = frame {
                let overlay = if auto_crop { corners } else { None };
                ui.set_preview_source(preview_image(&frame, overlay, corners_frame));
            }
        }
    }

    fn set_status(&self, message: &str) {
        if let Some(ui) = self.ui() {
            ui.set_status_text(message.into());
        }
    }

    fn play_sound(&mut self, bytes: &'static [u8]) {
        if self.audio.is_none() {
            self.audio = Audio::new();
        }
        if let Some(audio) = &self.audio {
            audio.play(bytes);
        }
    }

    fn flash(&self) {
        if let Some(ui) = self.ui() {
            ui.set_flash(true);
            let weak = ui.as_weak();
            slint::Timer::single_shot(Duration::from_millis(80), move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_flash(false);
                }
            });
        }
    }

    fn show_toast(&self, message: &str) {
        if let Some(ui) = self.ui() {
            ui.set_toast_text(message.into());
            ui.set_show_toast(true);
            let weak = ui.as_weak();
            slint::Timer::single_shot(Duration::from_millis(1600), move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_show_toast(false);
                }
            });
        }
    }

    fn select_device(&mut self, index: usize) {
        let Some(camera) = self.devices.get(index).cloned() else {
            return;
        };
        self.worker
            .select_device(camera.id.clone(), camera.name.clone());
        self.config.last_camera_id = Some(camera.id);
        self.config.last_camera_name = Some(camera.name.clone());
        self.config.save();
        self.set_status(&format!("正在切换到 {}…", camera.name));
    }

    fn select_format(&mut self, index: usize) {
        let format = index
            .checked_sub(1)
            .and_then(|index| self.formats.get(index).copied());
        self.worker.select_format(format);
        self.config.capture_width = format.and_then(|(width, _)| u16::try_from(width).ok());
        self.config.capture_height = format.and_then(|(_, height)| u16::try_from(height).ok());
        self.config.save();
        self.set_status(&format.map_or_else(
            || "分辨率：自动".to_string(),
            |(width, height)| format!("分辨率：{width} × {height}"),
        ));
    }

    fn select_fps(&mut self, index: usize) {
        let fps = match index {
            1 => 30,
            2 => 50,
            3 => 60,
            _ => 25,
        };
        self.config.record_fps = Some(fps as u8);
        self.config.save();
        self.set_status(&format!("录像帧率：{fps} fps"));
    }

    fn toggle_record(&mut self) {
        self.play_sound(RECORD_SOUND);
        if let Some(recorder) = self.recording.take() {
            self.worker.set_record_sink(None);
            match recorder.stop() {
                Ok(path) => {
                    self.set_status(&format!("录像已保存：{}", path.display()));
                    self.show_toast("录像已保存");
                }
                Err(err) => self.set_status(&format!("录像保存失败：{err}")),
            }
            if let Some(ui) = self.ui() {
                ui.set_recording(false);
            }
            return;
        }
        let Some(frame) = self.frame() else {
            self.set_status("尚未取得画面，无法开始录像");
            return;
        };
        let directory = self.output_root.join("录像");
        if let Err(err) = fs::create_dir_all(&directory) {
            self.set_status(&format!("创建录像目录失败：{err}"));
            return;
        }
        let name = format!(
            "录像_{}.mp4",
            naming::TimeStamp::now().format("yyyyMMdd_HHmmss")
        );
        let path = unique_path(&directory.join(name));
        let (record_width, record_height) = recording_size(frame.width, frame.height);
        let fps = u32::from(self.config.record_fps.unwrap_or(25).clamp(25, 60));
        let (sender, receiver) = sync_channel(4);
        let recorder = Recorder::start(path, record_width, record_height, fps, receiver);
        self.worker.set_record_sink(Some(sender));
        self.recording = Some(recorder);
        if let Some(ui) = self.ui() {
            ui.set_recording(true);
        }
        self.set_status(&format!(
            "录像中… {record_width}×{record_height} @ {fps} fps"
        ));
        self.show_toast("开始录像");
    }

    fn auto_fix_page(&mut self, row: usize, crop: bool) {
        let Some(index) = self.session_index(row) else {
            return;
        };
        let Some(page) = self.session.pages.get(index).cloned() else {
            return;
        };
        let path = self.output_root.join(&page.path);
        let (width, height, data) = match imaging::load_bgra(&path) {
            Ok(image) => image,
            Err(err) => {
                self.set_status(&format!("读取图片失败：{err}"));
                return;
            }
        };
        if imaging::detect_document_corners(width, height, &data).is_none() {
            self.set_status("未检测到可信的文档边界，已跳过");
            return;
        }
        let options = imaging::EnhanceOptions {
            auto_crop: crop,
            perspective: crop,
            deskew: !crop,
            strength: 60,
            ..imaging::EnhanceOptions::default()
        };
        let fixed = match imaging::enhance(width, height, &data, options) {
            Ok(image) => image,
            Err(err) => {
                self.set_status(&format!("自动处理失败：{err}"));
                return;
            }
        };
        if fixed.width == width && fixed.height == height && fixed.bgra == data {
            self.set_status("无需校正");
            return;
        }
        if let Err(err) = write_jpeg_atomic(
            &path,
            fixed.width,
            fixed.height,
            &fixed.bgra,
            self.config.jpeg_quality.unwrap_or(92),
        ) {
            self.set_status(&format!("写回失败：{err}"));
            return;
        }
        if let Some(page) = self.session.pages.get_mut(index) {
            page.width = fixed.width;
            page.height = fixed.height;
        }
        self.thumb_cache.remove(&page.seq);
        self.session.updated_at = doc::now_unix();
        let _ = self.store.save(&self.session);
        self.selected_row = Some(row);
        self.rebuild_pages();
        self.set_status(if crop {
            "已自动裁边并纠正畸变"
        } else {
            "已自动纠偏"
        });
    }

    fn rename_page(&mut self, row: usize) {
        let Some(index) = self.session_index(row) else {
            return;
        };
        let Some(page) = self.session.pages.get(index).cloned() else {
            return;
        };
        let current = page.path.file_name().map_or_else(
            || page.path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        self.rename_row = Some(row);
        if let Some(ui) = self.ui() {
            ui.set_rename_text(current.into());
            ui.set_show_rename(true);
        }
    }

    fn show_page_menu(&mut self, row: usize) {
        match usbscan_win::page_menu::show_page_menu() {
            usbscan_win::page_menu::PageMenuCommand::Rename => self.rename_page(row),
            usbscan_win::page_menu::PageMenuCommand::AutoCrop => self.auto_fix_page(row, true),
            usbscan_win::page_menu::PageMenuCommand::AutoDeskew => self.auto_fix_page(row, false),
            usbscan_win::page_menu::PageMenuCommand::Ocr => {
                self.selected_row = Some(row);
                self.ocr_page();
            }
            usbscan_win::page_menu::PageMenuCommand::ExportJpg => {
                self.selected_row = Some(row);
                self.export_jpg();
            }
            usbscan_win::page_menu::PageMenuCommand::Delete => self.delete_page(row),
            usbscan_win::page_menu::PageMenuCommand::None => {}
        }
    }

    fn confirm_rename(&mut self, input: &str) {
        let Some(row) = self.rename_row else {
            return;
        };
        let Some(index) = self.session_index(row) else {
            return;
        };
        let Some(page) = self.session.pages.get(index).cloned() else {
            return;
        };
        let old_path = self.output_root.join(&page.path);
        let stem = Path::new(input.trim())
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        if stem.is_empty()
            || stem.chars().any(|ch| {
                ch.is_control()
                    || matches!(ch, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
            })
        {
            self.set_status("文件名无效");
            return;
        }
        let extension = old_path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("jpg");
        let new_path = old_path.with_file_name(format!("{stem}.{extension}"));
        if new_path.exists() {
            self.set_status("同名文件已存在");
            return;
        }
        if let Err(err) = fs::rename(&old_path, &new_path) {
            self.set_status(&format!("重命名失败：{err}"));
            return;
        }
        for sidecar in ["txt", "ocr.tsv"] {
            let old = old_path.with_extension(sidecar);
            if old.exists() {
                let _ = fs::rename(old, new_path.with_extension(sidecar));
            }
        }
        if let Ok(relative) = new_path.strip_prefix(&self.output_root) {
            if let Some(page) = self.session.pages.get_mut(index) {
                page.path = relative.to_path_buf();
            }
        }
        self.thumb_cache.remove(&page.seq);
        self.session.updated_at = doc::now_unix();
        let _ = self.store.save(&self.session);
        if let Some(ui) = self.ui() {
            ui.set_show_rename(false);
        }
        self.rename_row = None;
        self.selected_row = Some(row);
        self.rebuild_pages();
        self.set_status("已重命名");
    }

    fn delete_page(&mut self, row: usize) {
        let Some(index) = self.session_index(row) else {
            return;
        };
        let Some(page) = self.session.pages.get(index).cloned() else {
            return;
        };
        let path = self.output_root.join(&page.path);
        let trash = self.output_root.join(".usbscan").join("trash");
        let _ = fs::create_dir_all(&trash);
        if path.exists() {
            let destination = unique_path(
                &trash.join(
                    page.path
                        .file_name()
                        .unwrap_or_else(|| std::ffi::OsStr::new("page.jpg")),
                ),
            );
            let _ = fs::rename(&path, destination);
        }
        self.session.pages.remove(index);
        self.thumb_cache.remove(&page.seq);
        self.session.updated_at = doc::now_unix();
        let _ = self.store.save(&self.session);
        self.selected_row = None;
        self.rebuild_pages();
        self.set_status("已移到回收目录");
    }

    fn import_images(&mut self) {
        let Some(files) = rfd::FileDialog::new()
            .add_filter("图片", &["jpg", "jpeg", "png", "bmp"])
            .pick_files()
        else {
            return;
        };
        for file in files {
            let Ok((width, height, data)) = imaging::load_bgra(&file) else {
                continue;
            };
            let path = self.new_import_path();
            if write_jpeg_atomic(
                &path,
                width,
                height,
                &data,
                self.config.jpeg_quality.unwrap_or(92),
            )
            .is_err()
            {
                continue;
            }
            let relative = path
                .strip_prefix(&self.output_root)
                .unwrap_or(&path)
                .to_path_buf();
            let seq = self.session.next_seq;
            self.session.pages.push(doc::Page {
                seq,
                path: relative,
                raw: None,
                width,
                height,
                captured_at: doc::now_unix(),
                device: None,
            });
            self.session.next_seq = seq.saturating_add(1);
        }
        self.session.updated_at = doc::now_unix();
        let _ = self.store.save(&self.session);
        self.rebuild_pages();
        self.set_status("图片导入完成");
    }

    fn export_pdf(&mut self) {
        if self.session.pages.is_empty() {
            self.set_status("没有可导出的页面");
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .set_file_name("usbscan.pdf")
            .add_filter("PDF", &["pdf"])
            .save_file()
        else {
            return;
        };
        let mut jpegs = Vec::new();
        for page in &self.session.pages {
            jpegs.push((
                fs::read(self.output_root.join(&page.path)).unwrap_or_default(),
                page.width,
                page.height,
            ));
        }
        let images: Vec<export::PdfImage<'_>> = jpegs
            .iter()
            .map(|(jpeg, width, height)| export::PdfImage {
                jpeg,
                width: *width,
                height: *height,
                text: &[],
            })
            .collect();
        let result = fs::File::create(&path)
            .and_then(|file| export::write_pdf(std::io::BufWriter::new(file), &images));
        match result {
            Ok(()) => self.set_status(&format!("已导出 PDF：{}", path.display())),
            Err(err) => self.set_status(&format!("导出 PDF 失败：{err}")),
        }
    }

    fn export_jpg(&mut self) {
        let Some(row) = self.selected_row else {
            self.set_status("请先选择页面");
            return;
        };
        let Some(index) = self.session_index(row) else {
            return;
        };
        let Some(page) = self.session.pages.get(index) else {
            return;
        };
        let source = self.output_root.join(&page.path);
        let Some(path) = rfd::FileDialog::new()
            .set_file_name("page.jpg")
            .add_filter("JPEG", &["jpg"])
            .save_file()
        else {
            return;
        };
        match fs::copy(source, &path) {
            Ok(_) => self.set_status(&format!("已导出：{}", path.display())),
            Err(err) => self.set_status(&format!("导出失败：{err}")),
        }
    }

    fn ocr_page(&mut self) {
        let Some(row) = self.selected_row else {
            self.set_status("请先选择页面");
            return;
        };
        let Some(index) = self.session_index(row) else {
            return;
        };
        let Some(page) = self.session.pages.get(index) else {
            return;
        };
        let Some(executable) =
            ocr::find_tesseract(self.config.tesseract_path.as_deref().map(Path::new))
        else {
            self.set_status("未找到 Tesseract，请安装或在设置中指定路径");
            return;
        };
        let language = self
            .config
            .ocr_language
            .clone()
            .unwrap_or_else(|| ocr::DEFAULT_LANGUAGE.to_string());
        let path = self.output_root.join(&page.path);
        match ocr::recognize(&executable, &path, &language) {
            Ok(text) => {
                let text_path = path.with_extension("txt");
                let _ = fs::write(&text_path, text.as_bytes());
                self.set_status(&format!("OCR 完成：{} 个字符", text.chars().count()));
            }
            Err(err) => self.set_status(&format!("OCR 失败：{err}")),
        }
    }

    fn apply_settings(&mut self) {
        let Some(ui) = self.ui() else {
            return;
        };
        self.rule.template = ui.get_file_template().to_string();
        self.rule.folder = ui.get_folder_template().to_string();
        self.config.file_template = Some(self.rule.template.clone());
        self.config.folder_template = Some(self.rule.folder.clone());
        self.config.auto_crop = Some(ui.get_auto_crop());
        self.config.deskew = Some(ui.get_auto_deskew());
        self.config.perspective = Some(ui.get_perspective());
        self.config.remove_shadow = Some(ui.get_remove_shadow());
        self.config.white_background = Some(ui.get_white_background());
        self.config.enhance_strength = Some(ui.get_strength().round() as u8);
        self.config.save();
        self.set_status("设置已保存");
    }

    fn browse_output(&mut self) {
        let Some(ui) = self.ui() else {
            return;
        };
        let Some(folder) = rfd::FileDialog::new().pick_folder() else {
            return;
        };
        if self.session.pages.is_empty() {
            self.output_root.clone_from(&folder);
            self.store = doc::SessionStore::new(folder.clone());
            self.config.output_root = Some(folder.clone());
            self.config.save();
            ui.set_output_dir(folder.to_string_lossy().to_string().into());
            self.set_status("输出目录已更新");
        } else {
            self.config.output_root = Some(folder);
            self.config.save();
            self.set_status("输出目录已保存，将在新文档中生效");
        }
    }

    fn session_index(&self, row: usize) -> Option<usize> {
        self.rows.get(row).copied()
    }

    fn shutdown(&mut self) {
        if let Some(recorder) = self.recording.take() {
            self.worker.set_record_sink(None);
            let _ = recorder.stop();
        }
        self.session.closed_cleanly = true;
        let _ = self.store.save(&self.session);
    }
}

// ---------------------------------------------------------------------------
// 启动与界面绑定
// ---------------------------------------------------------------------------

fn main() {
    if let Err(err) = run() {
        eprintln!("usbscan 启动失败：{err}");
        std::process::exit(1);
    }
}

#[allow(clippy::too_many_lines)]
fn run() -> Result<(), Box<dyn Error>> {
    let ui = MainWindow::new()?;
    let config = Config::load();
    let window_width = config.window_width.unwrap_or(1280).clamp(1100, 10000);
    let window_height = config.window_height.unwrap_or(800).clamp(680, 10000);
    let saved_position = match (config.window_x, config.window_y) {
        (Some(x), Some(y)) if x.abs() < 100_000 && y.abs() < 100_000 => Some((x, y)),
        _ => None,
    };
    ui.window().set_size(slint::LogicalSize::new(
        window_width as f32,
        window_height as f32,
    ));
    if let Some((x, y)) = saved_position {
        ui.window().set_position(slint::PhysicalPosition::new(x, y));
    }
    let output_root = config
        .output_root
        .clone()
        .filter(|path| path.is_absolute())
        .unwrap_or_else(default_output_root);
    let store = doc::SessionStore::new(output_root.clone());
    let mut session = store.load().ok().flatten().unwrap_or_default();
    session.closed_cleanly = false;
    let mut rule = naming::NamingRule::default();
    if let Some(template) = &config.file_template {
        rule.template.clone_from(template);
    }
    if let Some(folder) = &config.folder_template {
        rule.folder.clone_from(folder);
    }
    let dark = theme::system_uses_dark_mode();
    theme::init_process(dark);
    ui.set_dark(dark);
    ui.set_output_dir(output_root.to_string_lossy().to_string().into());
    ui.set_file_template(rule.template.clone().into());
    ui.set_folder_template(rule.folder.clone().into());
    ui.set_auto_crop(config.auto_crop.unwrap_or(true));
    ui.set_auto_deskew(config.deskew.unwrap_or(true));
    ui.set_perspective(config.perspective.unwrap_or(true));
    ui.set_remove_shadow(config.remove_shadow.unwrap_or(false));
    ui.set_white_background(config.white_background.unwrap_or(false));
    ui.set_strength(f32::from(config.enhance_strength.unwrap_or(60)));
    ui.set_fps_index(match config.record_fps.unwrap_or(25) {
        30 => 1,
        50 => 2,
        60 => 3,
        _ => 0,
    });

    let worker = Worker::start();
    {
        let mut shared = worker.lock();
        shared.selected.clone_from(&config.last_camera_id);
        shared.selected_name.clone_from(&config.last_camera_name);
        shared.preferred_format = match (config.capture_width, config.capture_height) {
            (Some(width), Some(height)) => Some((u32::from(width), u32::from(height))),
            _ => None,
        };
    }

    let state = std::rc::Rc::new(std::cell::RefCell::new(AppState {
        ui: ui.as_weak(),
        worker,
        config,
        session,
        store,
        output_root,
        rule,
        devices: Vec::new(),
        formats: Vec::new(),
        rows: Vec::new(),
        selected_row: None,
        thumb_cache: HashMap::new(),
        last_sequence: 0,
        last_devices_version: 0,
        last_formats_version: 0,
        last_status: String::new(),
        last_auto_crop: true,
        recording: None,
        rename_row: None,
        audio: None,
        timer: None,
    }));

    wire_callbacks(&ui, &state);
    {
        let mut app = state.borrow_mut();
        app.rebuild_pages();
        let last = app.rows.len().checked_sub(1);
        app.selected_row = last;
        if let Some(ui) = app.ui() {
            ui.set_selected_page(last.map_or(-1, |row| row as i32));
        }
        let weak = ui.as_weak();
        let timer_state = std::rc::Rc::clone(&state);
        let timer = slint::Timer::default();
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(40),
            move || {
                if weak.upgrade().is_some() {
                    timer_state.borrow_mut().poll();
                }
            },
        );
        app.timer = Some(timer);
    }

    ui.show()?;
    {
        let weak = ui.as_weak();
        let state_close = std::rc::Rc::clone(&state);
        ui.window().on_close_requested(move || {
            if let Some(ui) = weak.upgrade() {
                let mut app = state_close.borrow_mut();
                let scale = ui.window().scale_factor();
                if let Some((width, height)) = usbscan_win::window::client_size() {
                    app.config.window_width = Some((width as f32 / scale).round().max(1.0) as u32);
                    app.config.window_height =
                        Some((height as f32 / scale).round().max(1.0) as u32);
                }
                let position = ui.window().position();
                app.config.window_x = Some(position.x);
                app.config.window_y = Some(position.y);
                app.config.save();
            }
            slint::CloseRequestResponse::HideWindow
        });
    }
    let hwnd = usbscan_win::window::main_window();
    if !hwnd.is_null() {
        usbscan_win::theme::apply_window(hwnd, dark);
        usbscan_win::window::set_window_icon();
    }
    let apply_geometry = |ui: &MainWindow| {
        let scale = ui.window().scale_factor();
        let width = (window_width as f32 * scale).round().max(1.0) as u32;
        let height = (window_height as f32 * scale).round().max(1.0) as u32;
        let (x, y) = saved_position.unwrap_or_else(|| {
            let current = ui.window().position();
            (current.x, current.y)
        });
        let _ = usbscan_win::window::set_client_geometry(x, y, width, height);
    };
    apply_geometry(&ui);
    {
        let weak = ui.as_weak();
        slint::Timer::single_shot(Duration::from_millis(200), move || {
            if let Some(ui) = weak.upgrade() {
                let scale = ui.window().scale_factor();
                let width = (window_width as f32 * scale).round().max(1.0) as u32;
                let height = (window_height as f32 * scale).round().max(1.0) as u32;
                let (x, y) = saved_position.unwrap_or_else(|| {
                    let current = ui.window().position();
                    (current.x, current.y)
                });
                let _ = usbscan_win::window::set_client_geometry(x, y, width, height);
                usbscan_win::window::set_window_icon();
            }
        });
    }
    slint::run_event_loop()?;
    state.borrow_mut().shutdown();
    Ok(())
}

fn wire_callbacks(ui: &MainWindow, state: &std::rc::Rc<std::cell::RefCell<AppState>>) {
    {
        let state = std::rc::Rc::clone(state);
        ui.on_capture(move || {
            let should_capture = state.borrow_mut().capture_feedback();
            if should_capture {
                let state = std::rc::Rc::clone(&state);
                slint::Timer::single_shot(Duration::from_millis(90), move || {
                    state.borrow_mut().capture_now();
                });
            }
        });
    }
    {
        let state = std::rc::Rc::clone(state);
        ui.on_toggle_record(move || state.borrow_mut().toggle_record());
    }
    {
        let state = std::rc::Rc::clone(state);
        ui.on_device_changed(move |index| state.borrow_mut().select_device(index as usize));
    }
    {
        let state = std::rc::Rc::clone(state);
        ui.on_format_changed(move |index| state.borrow_mut().select_format(index as usize));
    }
    {
        let state = std::rc::Rc::clone(state);
        ui.on_fps_changed(move |index| state.borrow_mut().select_fps(index as usize));
    }
    {
        let state = std::rc::Rc::clone(state);
        ui.on_page_selected(move |row| state.borrow_mut().selected_row = Some(row as usize));
    }
    {
        let state = std::rc::Rc::clone(state);
        ui.on_rename_page(move |row| state.borrow_mut().rename_page(row as usize));
    }
    {
        let state = std::rc::Rc::clone(state);
        ui.on_page_menu(move |row| state.borrow_mut().show_page_menu(row as usize));
    }
    {
        let state = std::rc::Rc::clone(state);
        ui.on_delete_page(move |row| state.borrow_mut().delete_page(row as usize));
    }
    {
        let state = std::rc::Rc::clone(state);
        ui.on_auto_crop_page(move |row| state.borrow_mut().auto_fix_page(row as usize, true));
    }
    {
        let state = std::rc::Rc::clone(state);
        ui.on_auto_deskew_page(move |row| state.borrow_mut().auto_fix_page(row as usize, false));
    }
    {
        let state = std::rc::Rc::clone(state);
        ui.on_rename_confirm(move |text| state.borrow_mut().confirm_rename(text.as_str()));
    }
    {
        let state = std::rc::Rc::clone(state);
        ui.on_rename_cancel(move || {
            let mut app = state.borrow_mut();
            app.rename_row = None;
            if let Some(ui) = app.ui() {
                ui.set_show_rename(false);
            }
        });
    }
    {
        let state = std::rc::Rc::clone(state);
        ui.on_import_images(move || state.borrow_mut().import_images());
    }
    {
        let state = std::rc::Rc::clone(state);
        ui.on_export_pdf(move || state.borrow_mut().export_pdf());
    }
    {
        let state = std::rc::Rc::clone(state);
        ui.on_export_jpg(move || state.borrow_mut().export_jpg());
    }
    {
        let state = std::rc::Rc::clone(state);
        ui.on_ocr_page(move || state.borrow_mut().ocr_page());
    }
    {
        let state = std::rc::Rc::clone(state);
        ui.on_browse_output(move || state.borrow_mut().browse_output());
    }
    {
        let state = std::rc::Rc::clone(state);
        ui.on_apply_settings(move || state.borrow_mut().apply_settings());
    }
    {
        let state = std::rc::Rc::clone(state);
        ui.on_refresh_devices(move || {
            state.borrow_mut().worker.refresh();
            state.borrow_mut().set_status("正在重新检测设备…");
        });
    }
}

// ---------------------------------------------------------------------------
// 图像与文件辅助
// ---------------------------------------------------------------------------

fn preview_image(
    frame: &capture::Frame,
    corners: Option<[(f64, f64); 4]>,
    corners_frame: Option<(u32, u32)>,
) -> Image {
    let scale = (f64::from(PREVIEW_MAX_SIDE) / f64::from(frame.width.max(1)))
        .min(f64::from(PREVIEW_MAX_SIDE) / f64::from(frame.height.max(1)))
        .min(1.0);
    let (width, height, data) = if scale < 1.0 {
        imaging::thumbnail_bgra(
            frame.width,
            frame.height,
            &frame.data,
            PREVIEW_MAX_SIDE,
            PREVIEW_MAX_SIDE,
        )
        .unwrap_or_else(|_| (frame.width, frame.height, frame.data.clone()))
    } else {
        (frame.width, frame.height, frame.data.clone())
    };
    let mut image = bgra_to_buffer(width, height, &data);
    if let (Some(corners), Some((frame_width, frame_height))) = (corners, corners_frame) {
        let sx = f64::from(width) / f64::from(frame_width.max(1));
        let sy = f64::from(height) / f64::from(frame_height.max(1));
        let scaled = corners.map(|(x, y)| (x * sx, y * sy));
        draw_overlay(image.make_mut_bytes(), width, height, &scaled);
    }
    Image::from_rgba8(image)
}

fn bgra_to_buffer(width: u32, height: u32, data: &[u8]) -> SharedPixelBuffer<Rgba8Pixel> {
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(width, height);
    let count = width as usize * height as usize;
    let dst = buffer.make_mut_bytes();
    for index in 0..count {
        let src = index * 4;
        if src + 3 < data.len() {
            dst[index * 4] = data[src + 2];
            dst[index * 4 + 1] = data[src + 1];
            dst[index * 4 + 2] = data[src];
            dst[index * 4 + 3] = data[src + 3];
        }
    }
    buffer
}

fn bgra_to_image(width: u32, height: u32, data: &[u8]) -> Image {
    Image::from_rgba8(bgra_to_buffer(width, height, data))
}

fn draw_overlay(data: &mut [u8], width: u32, height: u32, corners: &[(f64, f64); 4]) {
    let accent = [10u8, 132, 255, 255];
    let halo = [255u8, 255, 255, 230];
    for index in 0..4 {
        let start = corners[index];
        let end = corners[(index + 1) % 4];
        draw_line(data, width, height, start, end, halo, 2);
    }
    for index in 0..4 {
        let start = corners[index];
        let end = corners[(index + 1) % 4];
        draw_line(data, width, height, start, end, accent, 1);
    }
    for (x, y) in corners {
        draw_handle(data, width, height, *x, *y, halo, 6);
    }
    for (x, y) in corners {
        draw_handle(data, width, height, *x, *y, accent, 4);
    }
}

fn draw_line(
    data: &mut [u8],
    width: u32,
    height: u32,
    start: (f64, f64),
    end: (f64, f64),
    color: [u8; 4],
    radius: i32,
) {
    let steps = (end.0 - start.0).abs().max((end.1 - start.1).abs()).ceil() as i32;
    for step in 0..=steps.max(1) {
        let t = f64::from(step) / f64::from(steps.max(1));
        let x = (start.0 + (end.0 - start.0) * t).round() as i32;
        let y = (start.1 + (end.1 - start.1) * t).round() as i32;
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                set_pixel(data, width, height, x + dx, y + dy, color);
            }
        }
    }
}

fn draw_handle(
    data: &mut [u8],
    width: u32,
    height: u32,
    x: f64,
    y: f64,
    color: [u8; 4],
    radius: i32,
) {
    let cx = x.round() as i32;
    let cy = y.round() as i32;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            set_pixel(data, width, height, cx + dx, cy + dy, color);
        }
    }
}

fn set_pixel(data: &mut [u8], width: u32, height: u32, x: i32, y: i32, color: [u8; 4]) {
    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        return;
    }
    let offset = (y as usize * width as usize + x as usize) * 4;
    if offset + 3 < data.len() {
        data[offset..offset + 4].copy_from_slice(&color);
    }
}

fn write_jpeg_atomic(
    path: &Path,
    width: u32,
    height: u32,
    bgra: &[u8],
    quality: u8,
) -> Result<(), String> {
    let temp = path.with_extension("tmp.jpg");
    let mut jpeg = Vec::new();
    imaging::encode_jpeg_bgra(width, height, bgra, quality, &mut jpeg)
        .map_err(|err| err.to_string())?;
    fs::write(&temp, jpeg).map_err(|err| err.to_string())?;
    fs::rename(&temp, path).map_err(|err| {
        let _ = fs::remove_file(&temp);
        err.to_string()
    })
}

fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path
        .file_stem()
        .map_or_else(|| "page".into(), std::ffi::OsStr::to_os_string);
    let extension = path
        .extension()
        .map_or_else(|| "jpg".into(), std::ffi::OsStr::to_os_string);
    for index in 2..1000 {
        let candidate = path.with_file_name(format!(
            "{}_{index}.{}",
            stem.to_string_lossy(),
            extension.to_string_lossy()
        ));
        if !candidate.exists() {
            return candidate;
        }
    }
    path.to_path_buf()
}

fn recording_size(width: u32, height: u32) -> (u32, u32) {
    let scale = (1920.0 / f64::from(width.max(1)))
        .min(1080.0 / f64::from(height.max(1)))
        .min(1.0);
    (
        (f64::from(width) * scale).round().max(1.0) as u32,
        (f64::from(height) * scale).round().max(1.0) as u32,
    )
}

fn default_output_root() -> PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("PUBLIC").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    home.join("Pictures").join("usbscan")
}
