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

mod i18n;
mod tesseract_install;
mod update;

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
const PREVIEW_MAX_SIDE: u32 = 1024;
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
    rotation: u32,
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
            status: i18n::t("status.detecting").to_string(),
            connected: false,
            corners: None,
            corners_frame: None,
            frame_size: None,
            selected: None,
            selected_name: None,
            preferred_format: None,
            auto_crop: true,
            rotation: 0,
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

    fn set_rotation(&self, degrees: u32) {
        let mut shared = self.lock();
        shared.rotation = degrees % 360;
        // 旧坐标系下的裁边结果已失效，等待下一帧重新检测。
        shared.corners = None;
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
        set_status(
            shared,
            false,
            i18n::t_args("status.mfInitFailed", &[&err.to_string()]),
        );
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
                set_status(
                    shared,
                    false,
                    i18n::t_args("status.enumerateFailed", &[&err.to_string()]),
                );
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
            let saved_name =
                selected_name.unwrap_or_else(|| i18n::t("status.savedDevice").to_string());
            set_status(
                shared,
                false,
                i18n::t_args("status.waitingCamera", &[&saved_name]),
            );
            sleep_interruptible(stop, Duration::from_millis(1000));
            continue;
        }
        let Some(info) = select_camera(&cameras, selected.as_deref(), selected_name.as_deref())
        else {
            set_status(shared, false, i18n::t("status.noCamera").to_string());
            sleep_interruptible(stop, Duration::from_millis(1000));
            continue;
        };

        let mut camera = match capture::Camera::open(info, preferred) {
            Ok(camera) => camera,
            Err(err) => {
                set_status(
                    shared,
                    false,
                    i18n::t_args("status.openCameraFailed", &[&err.to_string()]),
                );
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
            guard.status = i18n::t_args(
                "status.connected",
                &[&info.name, &width.to_string(), &height.to_string()],
            );
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
                    let rotation = shared
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .rotation;
                    let mut frame = trim_frame(frame);
                    if rotation != 0 {
                        if let Ok((width, height, data)) =
                            imaging::rotate_bgra(frame.width, frame.height, &frame.data, rotation)
                        {
                            frame = capture::Frame {
                                width,
                                height,
                                format: frame.format,
                                data,
                                timestamp: frame.timestamp,
                            };
                        }
                    }
                    let frame = Arc::new(frame);
                    let (record_sink, auto_crop) = {
                        let mut guard = shared
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        if guard.frame_size != Some((frame.width, frame.height)) {
                            guard.frame_size = Some((frame.width, frame.height));
                            if guard.connected {
                                if let Some(name) = &guard.selected_name {
                                    guard.status = i18n::t_args(
                                        "status.connected",
                                        &[
                                            name,
                                            &frame.width.to_string(),
                                            &frame.height.to_string(),
                                        ],
                                    );
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
                        guard.status = i18n::t_args("status.cameraError", &[&err.to_string()]);
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

type UpdateCheckSlot = Arc<Mutex<Option<Result<Option<update::UpdateInfo>, String>>>>;
type UpdateDownloadSlot = Arc<Mutex<Option<Result<PathBuf, String>>>>;

#[allow(clippy::struct_excessive_bools)]
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
    last_connected: bool,
    last_recording: bool,
    last_output_dir: String,
    settings_dirty: bool,
    last_settings_save: Instant,
    recording: Option<Recorder>,
    record_started: Option<Instant>,
    rotation: u32,
    rename_row: Option<usize>,
    audio: Option<Audio>,
    update_check: UpdateCheckSlot,
    update_download: UpdateDownloadSlot,
    update_info: Option<update::UpdateInfo>,
    update_checking: bool,
    update_check_manual: bool,
    update_downloading: bool,
    tessdata_dir: Option<PathBuf>,
    tesseract_install: Arc<Mutex<Option<Result<tesseract_install::Installed, String>>>>,
    tesseract_installing: bool,
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
            self.set_status(i18n::t("status.noFrameCapture"));
            return false;
        }
        self.play_sound(CAMERA_SOUND);
        self.flash();
        self.show_toast(i18n::t("toast.captured"));
        true
    }

    fn rotate_preview(&mut self) {
        self.rotation = (self.rotation + 90) % 360;
        self.worker.set_rotation(self.rotation);
        self.config.rotation = Some(self.rotation as u16);
        self.config.save();
        if let Some(ui) = self.ui() {
            ui.set_rotation_degrees(self.rotation as i32);
        }
        self.show_toast(&i18n::t_args(
            "toast.rotated",
            &[&self.rotation.to_string()],
        ));
    }

    fn capture_now(&mut self) {
        let Some(frame) = self.frame() else {
            self.set_status(i18n::t("status.noFrameCapture"));
            return;
        };
        let ui = self.ui();
        let options = self.enhance_options();
        let enhanced = self.enhance_frame(&frame, options);
        let _ = ui;
        let path = self.new_image_path(enhanced.width, enhanced.height);
        if let Some(parent) = path.parent() {
            if let Err(err) = fs::create_dir_all(parent) {
                self.set_status(&i18n::t_args("status.createDirFailed", &[&err.to_string()]));
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
            self.set_status(&i18n::t_args("status.saveImageFailed", &[&err]));
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
        self.set_status(&i18n::t_args(
            "status.savedPage",
            &[
                &path.file_name().unwrap_or_default().to_string_lossy(),
                &enhanced.width.to_string(),
                &enhanced.height.to_string(),
            ],
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
                    "{}_{}.jpg",
                    i18n::t("file.scan"),
                    naming::TimeStamp::now().format("yyyyMMdd_HHmmss")
                ))
            },
            |rendered| rendered.path(&self.output_root, "jpg"),
        );
        unique_path(&rendered)
    }

    fn new_import_path(&self) -> PathBuf {
        let now = naming::TimeStamp::now();
        unique_path(&self.output_root.join(format!(
            "{}_{}.jpg",
            i18n::t("file.import"),
            now.format("yyyyMMdd_HHmmss")
        )))
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
        ui.set_page_count_text(i18n::page_count(self.session.pages.len()).into());
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
            let mut names = vec![slint::SharedString::from(i18n::t("ui.auto"))];
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
        if connected != self.last_connected {
            self.last_connected = connected;
            ui.set_has_camera(connected);
        }
        let recording = self.recording.is_some();
        if recording != self.last_recording {
            self.last_recording = recording;
            ui.set_recording(recording);
        }
        self.sync_settings_from_ui();
        if sequence != self.last_sequence {
            self.last_sequence = sequence;
            if let Some(frame) = frame {
                let overlay = if ui.get_auto_crop() { corners } else { None };
                ui.set_preview_source(preview_image(&frame, overlay, corners_frame));
            }
        }
    }

    /// 把扫描设置实时同步到命名规则、配置文件和采集线程。
    ///
    /// 这里的设置没有“应用”按钮；轮询里发现界面值变化就立即生效，并延迟
    /// 落盘，避免用户每敲一个字符都写配置文件。
    fn sync_settings_from_ui(&mut self) {
        let Some(ui) = self.ui() else {
            return;
        };
        let mut changed = false;

        let template = ui.get_file_template().to_string();
        if template != self.rule.template {
            self.rule.template.clone_from(&template);
            self.config.file_template = Some(template);
            changed = true;
        }
        let folder = ui.get_folder_template().to_string();
        if folder != self.rule.folder {
            self.rule.folder.clone_from(&folder);
            self.config.folder_template = Some(folder);
            changed = true;
        }
        let output = ui.get_output_dir().to_string();
        if output != self.last_output_dir {
            let trimmed = output.trim();
            if !trimmed.is_empty() {
                let path = PathBuf::from(trimmed);
                if path != self.output_root {
                    self.set_output_root(path);
                }
                // 以界面里的原始文本为准，避免路径规范化（如末尾斜杠）导致每帧重复同步。
                self.last_output_dir.clone_from(&output);
                changed = true;
            }
        }

        let auto_crop = ui.get_auto_crop();
        if auto_crop != self.last_auto_crop {
            self.last_auto_crop = auto_crop;
            self.config.auto_crop = Some(auto_crop);
            self.worker.set_auto_crop(auto_crop);
            changed = true;
        }
        let deskew = ui.get_auto_deskew();
        if self.config.deskew != Some(deskew) {
            self.config.deskew = Some(deskew);
            changed = true;
        }
        let perspective = ui.get_perspective();
        if self.config.perspective != Some(perspective) {
            self.config.perspective = Some(perspective);
            changed = true;
        }
        let remove_shadow = ui.get_remove_shadow();
        if self.config.remove_shadow != Some(remove_shadow) {
            self.config.remove_shadow = Some(remove_shadow);
            changed = true;
        }
        let white_background = ui.get_white_background();
        if self.config.white_background != Some(white_background) {
            self.config.white_background = Some(white_background);
            changed = true;
        }
        let strength = ui.get_strength().round().clamp(0.0, 100.0) as u8;
        if self.config.enhance_strength != Some(strength) {
            self.config.enhance_strength = Some(strength);
            changed = true;
        }

        if changed {
            self.settings_dirty = true;
        }
        if self.settings_dirty && self.last_settings_save.elapsed() >= Duration::from_millis(600) {
            self.config.save();
            self.settings_dirty = false;
            self.last_settings_save = Instant::now();
        }
    }

    fn set_status(&self, message: &str) {
        if let Some(ui) = self.ui() {
            ui.set_status_text(message.into());
        }
    }

    fn set_language(&mut self, lang: i18n::Lang) {
        i18n::set_language(lang);
        self.config.language = Some(lang.code().to_string());
        self.config.save();
        let Some(ui) = self.ui() else {
            return;
        };
        apply_ui_language(&ui);
        ui.set_page_count_text(i18n::page_count(self.session.pages.len()).into());

        let mut names = vec![slint::SharedString::from(i18n::t("ui.auto"))];
        names.extend(
            self.formats
                .iter()
                .map(|(width, height)| format!("{width} × {height}").into()),
        );
        ui.set_format_options(ModelRc::from(Rc::new(VecModel::from(names))));

        let message = i18n::t("status.languageChanged").to_string();
        {
            let mut guard = self.worker.lock();
            guard.status.clone_from(&message);
        }
        self.last_status.clone_from(&message);
        ui.set_status_text(message.into());

        if let Some(info) = &self.update_info {
            let text = if self.update_downloading {
                i18n::t("status.downloadingUpdate").to_string()
            } else {
                i18n::t_args("dialog.updateBody", &[&info.version])
            };
            ui.set_update_text(text.into());
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

    fn start_update_check(&mut self, manual: bool) {
        if self.update_checking {
            if manual {
                self.set_status(i18n::t("ui.checkingUpdates"));
            }
            return;
        }
        self.update_checking = true;
        self.update_check_manual = manual;
        if let Some(ui) = self.ui() {
            ui.set_update_checking(true);
            ui.set_update_up_to_date(false);
            ui.set_update_failed(false);
        }
        if manual {
            self.set_status(i18n::t("ui.checkingUpdates"));
        }
        let slot = Arc::clone(&self.update_check);
        thread::spawn(move || {
            let result = update::check_latest();
            if let Ok(mut guard) = slot.lock() {
                *guard = Some(result);
            }
        });
    }

    fn poll_updates(&mut self) {
        let check = self
            .update_check
            .lock()
            .ok()
            .and_then(|mut guard| guard.take());
        if let Some(result) = check {
            self.update_checking = false;
            let manual = self.update_check_manual;
            self.update_check_manual = false;
            if let Some(ui) = self.ui() {
                ui.set_update_checking(false);
            }
            match result {
                Ok(Some(info)) => {
                    if let Some(ui) = self.ui() {
                        ui.set_update_up_to_date(false);
                        ui.set_update_failed(false);
                        ui.set_update_version(format!("v{}", info.version).into());
                        ui.set_update_text(
                            i18n::t_args("dialog.updateBody", &[&info.version]).into(),
                        );
                        ui.set_has_update(true);
                    }
                    if manual {
                        self.set_status(&i18n::t_args("status.newVersion", &[&info.version]));
                    }
                    self.update_info = Some(info);
                }
                Ok(None) => {
                    if let Some(ui) = self.ui() {
                        ui.set_update_up_to_date(true);
                        ui.set_update_failed(false);
                    }
                    if manual {
                        self.set_status(i18n::t("status.upToDate"));
                    }
                }
                Err(err) => {
                    if let Some(ui) = self.ui() {
                        ui.set_update_up_to_date(false);
                        ui.set_update_failed(true);
                    }
                    if manual {
                        self.set_status(&i18n::t_args("status.updateCheckFailed", &[&err]));
                    } else {
                        eprintln!("update check failed: {err}");
                    }
                }
            }
        }

        let download = self
            .update_download
            .lock()
            .ok()
            .and_then(|mut guard| guard.take());
        if let Some(result) = download {
            self.update_downloading = false;
            match result {
                Ok(path) => {
                    if let Some(ui) = self.ui() {
                        ui.set_update_text(i18n::t("status.installingUpdate").into());
                    }
                    self.session.closed_cleanly = true;
                    let _ = self.store.save(&self.session);
                    self.config.save();
                    let _ = std::process::Command::new(&path).spawn();
                    std::process::exit(0);
                }
                Err(err) => {
                    self.set_status(&i18n::t_args("status.updateDownloadFailed", &[&err]));
                    if let Some(ui) = self.ui() {
                        ui.set_update_text(i18n::t("status.updateFailedRetry").into());
                    }
                }
            }
        }
    }

    fn start_update_download(&mut self) {
        if self.update_downloading {
            return;
        }
        let Some(info) = self.update_info.clone() else {
            return;
        };
        if info.installer_url.is_none() {
            let _ = webbrowser::open(&info.release_url);
            return;
        }
        self.update_downloading = true;
        if let Some(ui) = self.ui() {
            ui.set_update_text(i18n::t("status.downloadingUpdate").into());
        }
        let slot = Arc::clone(&self.update_download);
        thread::spawn(move || {
            let result = update::download_installer(&info);
            if let Ok(mut guard) = slot.lock() {
                *guard = Some(result);
            }
        });
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
        self.set_status(&i18n::t_args("status.switchingCamera", &[&camera.name]));
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
            || i18n::t("status.resolutionAuto").to_string(),
            |(width, height)| {
                i18n::t_args(
                    "status.resolution",
                    &[&width.to_string(), &height.to_string()],
                )
            },
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
        self.set_status(&i18n::t_args("status.recordFps", &[&fps.to_string()]));
    }

    fn toggle_record(&mut self) {
        self.play_sound(RECORD_SOUND);
        if let Some(recorder) = self.recording.take() {
            self.worker.set_record_sink(None);
            self.record_started = None;
            match recorder.stop() {
                Ok(path) => {
                    self.set_status(&i18n::t_args(
                        "status.recordingSaved",
                        &[&path.display().to_string()],
                    ));
                    self.show_toast(i18n::t("toast.recordingSaved"));
                }
                Err(err) => self.set_status(&i18n::t_args("status.recordingSaveFailed", &[&err])),
            }
            if let Some(ui) = self.ui() {
                ui.set_recording(false);
            }
            return;
        }
        let Some(frame) = self.frame() else {
            self.set_status(i18n::t("status.noFrameRecord"));
            return;
        };
        let directory = self.output_root.join(i18n::t("dir.recordings"));
        if let Err(err) = fs::create_dir_all(&directory) {
            self.set_status(&i18n::t_args(
                "status.createRecordDirFailed",
                &[&err.to_string()],
            ));
            return;
        }
        let name = format!(
            "{}_{}.mp4",
            i18n::t("file.recording"),
            naming::TimeStamp::now().format("yyyyMMdd_HHmmss")
        );
        let path = unique_path(&directory.join(name));
        let (record_width, record_height) = recording_size(frame.width, frame.height);
        let fps = u32::from(self.config.record_fps.unwrap_or(25).clamp(25, 60));
        let (sender, receiver) = sync_channel(4);
        let recorder = Recorder::start(path, record_width, record_height, fps, receiver);
        self.worker.set_record_sink(Some(sender));
        self.recording = Some(recorder);
        self.record_started = Some(Instant::now());
        if let Some(ui) = self.ui() {
            ui.set_recording(true);
            ui.set_recording_elapsed("00:00".into());
        }
        self.set_status(&i18n::t_args(
            "status.recordingNow",
            &[
                &record_width.to_string(),
                &record_height.to_string(),
                &fps.to_string(),
            ],
        ));
        self.show_toast(i18n::t("toast.recordingStarted"));
    }

    /// 刷新预览窗口底部的录像计时。
    fn tick_recording(&mut self) {
        let Some(started) = self.record_started else {
            return;
        };
        let text = format_elapsed(started.elapsed().as_secs());
        if let Some(ui) = self.ui() {
            if ui.get_recording_elapsed().as_str() != text {
                ui.set_recording_elapsed(text.into());
            }
        }
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
                self.set_status(&i18n::t_args("status.loadImageFailed", &[&err.to_string()]));
                return;
            }
        };
        if imaging::detect_document_corners(width, height, &data).is_none() {
            self.set_status(i18n::t("status.noBoundary"));
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
                self.set_status(&i18n::t_args(
                    "status.autoProcessFailed",
                    &[&err.to_string()],
                ));
                return;
            }
        };
        if fixed.width == width && fixed.height == height && fixed.bgra == data {
            self.set_status(i18n::t("status.noCorrectionNeeded"));
            return;
        }
        if let Err(err) = write_jpeg_atomic(
            &path,
            fixed.width,
            fixed.height,
            &fixed.bgra,
            self.config.jpeg_quality.unwrap_or(92),
        ) {
            self.set_status(&i18n::t_args("status.writeBackFailed", &[&err]));
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
            i18n::t("status.autoCropped")
        } else {
            i18n::t("status.deskewed")
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
        let labels = usbscan_win::page_menu::PageMenuLabels {
            open: i18n::t("menu.open"),
            rename: i18n::t("menu.rename"),
            reveal: i18n::t("menu.reveal"),
            auto_crop: i18n::t("menu.autoCrop"),
            auto_deskew: i18n::t("menu.autoDeskew"),
            ocr: i18n::t("menu.ocr"),
            export_jpg: i18n::t("menu.exportJpg"),
            delete: i18n::t("menu.delete"),
        };
        match usbscan_win::page_menu::show_page_menu(&labels) {
            usbscan_win::page_menu::PageMenuCommand::Open => self.open_page(row),
            usbscan_win::page_menu::PageMenuCommand::Rename => self.rename_page(row),
            usbscan_win::page_menu::PageMenuCommand::RevealInExplorer => self.reveal_page(row),
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

    fn open_page(&mut self, row: usize) {
        let Some(path) = self.page_path(row) else {
            return;
        };
        if !path.exists() {
            self.set_status(i18n::t("status.fileMissing"));
            return;
        }
        usbscan_win::shell::open_path(&path);
    }

    fn reveal_page(&mut self, row: usize) {
        let Some(path) = self.page_path(row) else {
            return;
        };
        if !path.exists() {
            self.set_status(i18n::t("status.fileMissing"));
            return;
        }
        usbscan_win::shell::reveal_in_explorer(&path);
    }

    fn page_path(&self, row: usize) -> Option<PathBuf> {
        let index = self.session_index(row)?;
        let page = self.session.pages.get(index)?;
        Some(self.output_root.join(&page.path))
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
            self.set_status(i18n::t("status.invalidFileName"));
            return;
        }
        let extension = old_path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("jpg");
        let new_path = old_path.with_file_name(format!("{stem}.{extension}"));
        if new_path.exists() {
            self.set_status(i18n::t("status.fileExists"));
            return;
        }
        if let Err(err) = fs::rename(&old_path, &new_path) {
            self.set_status(&i18n::t_args("status.renameFailed", &[&err.to_string()]));
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
        self.set_status(i18n::t("status.renamed"));
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
        self.set_status(i18n::t("status.movedToTrash"));
    }

    fn import_images(&mut self) {
        let Some(files) = rfd::FileDialog::new()
            .add_filter(i18n::t("dialog.images"), &["jpg", "jpeg", "png", "bmp"])
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
        self.set_status(i18n::t("status.imagesImported"));
    }

    fn export_pdf(&mut self) {
        if self.session.pages.is_empty() {
            self.set_status(i18n::t("status.noPagesToExport"));
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
            Ok(()) => self.set_status(&i18n::t_args(
                "status.pdfExported",
                &[&path.display().to_string()],
            )),
            Err(err) => {
                self.set_status(&i18n::t_args("status.pdfExportFailed", &[&err.to_string()]));
            }
        }
    }

    fn export_jpg(&mut self) {
        let Some(row) = self.selected_row else {
            self.set_status(i18n::t("status.selectPageFirst"));
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
            Ok(_) => self.set_status(&i18n::t_args(
                "status.exported",
                &[&path.display().to_string()],
            )),
            Err(err) => {
                self.set_status(&i18n::t_args("status.exportFailed", &[&err.to_string()]));
            }
        }
    }

    fn ocr_page(&mut self) {
        let Some(row) = self.selected_row else {
            self.set_status(i18n::t("status.selectPageFirst"));
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
            self.prompt_tesseract_install();
            return;
        };
        let language = self
            .config
            .ocr_language
            .clone()
            .unwrap_or_else(|| ocr::DEFAULT_LANGUAGE.to_string());
        let path = self.output_root.join(&page.path);
        match ocr::recognize_with_tessdata(
            &executable,
            self.tessdata_dir.as_deref(),
            &path,
            &language,
        ) {
            Ok(text) => {
                let text_path = path.with_extension("txt");
                let _ = fs::write(&text_path, text.as_bytes());
                self.set_status(&i18n::t_args(
                    "status.ocrDone",
                    &[&text.chars().count().to_string()],
                ));
            }
            Err(err) => self.set_status(&i18n::t_args("status.ocrFailed", &[&err.to_string()])),
        }
    }

    fn prompt_tesseract_install(&mut self) {
        self.set_status(i18n::t("status.tesseractNotFound"));
        if let Some(ui) = self.ui() {
            ui.set_show_about(false);
            ui.set_tesseract_installing(false);
            ui.set_tesseract_status(i18n::t("tesseract.body").into());
            ui.set_show_tesseract_prompt(true);
        }
    }

    fn install_tesseract(&mut self) {
        if self.tesseract_installing {
            return;
        }
        self.tesseract_installing = true;
        if let Some(ui) = self.ui() {
            ui.set_tesseract_installing(true);
            ui.set_tesseract_status(i18n::t("tesseract.installing").into());
        }
        let slot = Arc::clone(&self.tesseract_install);
        let languages = self
            .config
            .ocr_language
            .clone()
            .unwrap_or_else(|| ocr::DEFAULT_LANGUAGE.to_string());
        thread::spawn(move || {
            let result = tesseract_install::install(&languages);
            if let Ok(mut guard) = slot.lock() {
                *guard = Some(result);
            }
        });
    }

    fn poll_tesseract(&mut self) {
        let result = self
            .tesseract_install
            .lock()
            .ok()
            .and_then(|mut guard| guard.take());
        let Some(result) = result else {
            return;
        };
        self.tesseract_installing = false;
        let Some(ui) = self.ui() else {
            return;
        };
        ui.set_tesseract_installing(false);
        match result {
            Ok(installed) => {
                self.config.tesseract_path =
                    Some(installed.executable.to_string_lossy().into_owned());
                self.config.tessdata_dir = installed
                    .tessdata
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned());
                self.config.save();
                self.tessdata_dir = installed.tessdata;
                ui.set_show_tesseract_prompt(false);
                ui.set_tesseract_status(i18n::t("tesseract.installed").into());
                self.set_status(i18n::t("status.tesseractInstalled"));
                self.ocr_page();
            }
            Err(err) => {
                ui.set_tesseract_status(i18n::t_args("tesseract.failed", &[&err]).into());
            }
        }
    }

    fn cancel_tesseract_install(&mut self) {
        if self.tesseract_installing {
            return;
        }
        if let Some(ui) = self.ui() {
            ui.set_show_tesseract_prompt(false);
        }
    }

    fn set_output_root(&mut self, folder: PathBuf) {
        let empty = self.session.pages.is_empty();
        if empty {
            self.output_root.clone_from(&folder);
            self.store = doc::SessionStore::new(folder.clone());
        }
        let text = folder.to_string_lossy().into_owned();
        self.last_output_dir.clone_from(&text);
        self.config.output_root = Some(folder);
        if empty {
            if let Some(ui) = self.ui() {
                ui.set_output_dir(text.into());
            }
        }
    }

    fn browse_output(&mut self) {
        let Some(folder) = rfd::FileDialog::new().pick_folder() else {
            return;
        };
        let empty = self.session.pages.is_empty();
        self.set_output_root(folder);
        self.config.save();
        self.set_status(if empty {
            i18n::t("status.outputDirUpdated")
        } else {
            i18n::t("status.outputDirSaved")
        });
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

fn apply_ui_language(ui: &MainWindow) {
    ui.set_t_device(i18n::t("ui.device").into());
    ui.set_t_resolution(i18n::t("ui.resolution").into());
    ui.set_t_fps(i18n::t("ui.fps").into());
    ui.set_t_refresh(i18n::t("ui.refresh").into());
    ui.set_t_record_start(i18n::t("ui.recordStart").into());
    ui.set_t_record_stop(i18n::t("ui.recordStop").into());
    ui.set_t_capture(i18n::t("ui.capture").into());
    ui.set_t_rotate(i18n::t("ui.rotate").into());
    ui.set_t_pages(i18n::t("ui.pages").into());
    ui.set_t_waiting(i18n::t("ui.waiting").into());
    ui.set_t_scan_settings(i18n::t("ui.scanSettings").into());
    ui.set_t_output_dir(i18n::t("ui.outputDir").into());
    ui.set_t_browse(i18n::t("ui.browse").into());
    ui.set_t_file_template(i18n::t("ui.fileTemplate").into());
    ui.set_t_folder_template(i18n::t("ui.folderTemplate").into());
    ui.set_t_import_images(i18n::t("ui.importImages").into());
    ui.set_t_export_pdf(i18n::t("ui.exportPdf").into());
    ui.set_t_enhance(i18n::t("ui.enhance").into());
    ui.set_t_auto_crop(i18n::t("ui.autoCrop").into());
    ui.set_t_auto_deskew(i18n::t("ui.autoDeskew").into());
    ui.set_t_perspective(i18n::t("ui.perspective").into());
    ui.set_t_remove_shadow(i18n::t("ui.removeShadow").into());
    ui.set_t_white_background(i18n::t("ui.whiteBackground").into());
    ui.set_t_strength(i18n::t("ui.strength").into());
    ui.set_t_rename_title(i18n::t("ui.renameTitle").into());
    ui.set_t_cancel(i18n::t("ui.cancel").into());
    ui.set_t_save(i18n::t("ui.save").into());
    ui.set_t_about(i18n::t("ui.about").into());
    ui.set_t_auto_check(i18n::t("ui.autoCheckUpdates").into());
    ui.set_t_update_hint(i18n::t("ui.updateHint").into());
    ui.set_t_check_updates(i18n::t("ui.checkUpdates").into());
    ui.set_t_checking_updates(i18n::t("ui.checkingUpdates").into());
    ui.set_t_up_to_date(i18n::t("status.upToDate").into());
    ui.set_t_retry_check(i18n::t("ui.retryCheck").into());
    ui.set_t_update_title(i18n::t("dialog.updateTitle").into());
    ui.set_t_update_now(i18n::t("dialog.updateNow").into());
    ui.set_t_update_later(i18n::t("dialog.later").into());
    ui.set_t_view_github(i18n::t("ui.viewOnGithub").into());
    ui.set_t_close(i18n::t("ui.close").into());
    ui.set_t_language(i18n::t("ui.language").into());
    ui.set_t_page_prefix(i18n::t("ui.pagePrefix").into());
    ui.set_t_page_suffix(i18n::t("ui.pageSuffix").into());
    ui.set_tesseract_title(i18n::t("tesseract.title").into());
    ui.set_tesseract_install_label(i18n::t("tesseract.install").into());

    let options: Vec<slint::SharedString> = i18n::Lang::ALL
        .iter()
        .map(|lang| lang.native_name().into())
        .collect();
    ui.set_language_options(ModelRc::from(Rc::new(VecModel::from(options))));
    ui.set_language_index(i18n::current().index() as i32);
}

#[allow(clippy::too_many_lines)]
fn run() -> Result<(), Box<dyn Error>> {
    let config = Config::load();
    let language = config
        .language
        .as_deref()
        .and_then(i18n::Lang::from_code)
        .unwrap_or_else(i18n::Lang::from_system);
    i18n::set_language(language);

    let ui = MainWindow::new()?;
    apply_ui_language(&ui);
    ui.set_status_text(i18n::t("status.detecting").into());
    ui.set_format_options(ModelRc::from(Rc::new(VecModel::from(vec![
        slint::SharedString::from(i18n::t("ui.auto")),
    ]))));
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
    if config.file_template.is_none() {
        rule.template = format!(
            "{}_{{date:yyyyMMdd}}_{{time:HHmmss}}_{{seq:3}}",
            i18n::t("file.scan")
        );
    }
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
    ui.set_version_text(format!("v{}", update::CURRENT_VERSION).into());
    ui.set_auto_check_updates(config.auto_check_updates.unwrap_or(true));
    let rotation = u32::from(config.rotation.unwrap_or(0) % 360);
    ui.set_rotation_degrees(rotation as i32);

    let worker = Worker::start();
    {
        let mut shared = worker.lock();
        shared.selected.clone_from(&config.last_camera_id);
        shared.selected_name.clone_from(&config.last_camera_name);
        shared.rotation = rotation;
        shared.preferred_format = match (config.capture_width, config.capture_height) {
            (Some(width), Some(height)) => Some((u32::from(width), u32::from(height))),
            _ => None,
        };
    }

    let tessdata_dir = config
        .tessdata_dir
        .as_ref()
        .map(PathBuf::from)
        .filter(|path| path.is_dir());
    let last_auto_crop = config.auto_crop.unwrap_or(true);
    let last_output_dir = output_root.to_string_lossy().into_owned();
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
        last_auto_crop,
        last_connected: false,
        last_recording: false,
        last_output_dir,
        settings_dirty: false,
        last_settings_save: Instant::now(),
        recording: None,
        record_started: None,
        rotation,
        rename_row: None,
        audio: None,
        update_check: Arc::new(Mutex::new(None)),
        update_download: Arc::new(Mutex::new(None)),
        update_info: None,
        update_checking: false,
        update_check_manual: false,
        update_downloading: false,
        tessdata_dir,
        tesseract_install: Arc::new(Mutex::new(None)),
        tesseract_installing: false,
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
    }

    if state.borrow().config.auto_check_updates.unwrap_or(true) {
        state.borrow_mut().start_update_check(false);
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
    {
        let weak = ui.as_weak();
        let timer_state = std::rc::Rc::clone(&state);
        let was_minimized = std::cell::Cell::new(usbscan_win::window::is_minimized(hwnd));
        let was_visible = std::cell::Cell::new(usbscan_win::window::is_visible(hwnd));
        let last_tick = std::cell::Cell::new(std::time::Instant::now());
        let timer = slint::Timer::default();
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(40),
            move || {
                let Some(ui) = weak.upgrade() else {
                    return;
                };
                let now = std::time::Instant::now();
                let long_gap = now.duration_since(last_tick.replace(now)) > Duration::from_secs(2);
                if !hwnd.is_null() {
                    let minimized = usbscan_win::window::is_minimized(hwnd);
                    let visible = usbscan_win::window::is_visible(hwnd);
                    let restored = (was_minimized.replace(minimized) && !minimized)
                        || (was_visible.replace(visible) && visible)
                        || (long_gap && visible && !minimized);
                    if restored {
                        // Windows 可能在最小化后丢弃窗口表面，而软件渲染器仍按旧画面
                        // 做局部重绘。改变全屏覆盖层的一个透明度色阶可强制整窗重绘。
                        let tick = ui.get_redraw_tick();
                        ui.set_redraw_tick(i32::from(tick == 0));
                        ui.window().request_redraw();
                    }
                }
                let mut app = timer_state.borrow_mut();
                app.poll();
                app.poll_updates();
                app.poll_tesseract();
                app.tick_recording();
            },
        );
        state.borrow_mut().timer = Some(timer);
    }

    slint::run_event_loop()?;
    state.borrow_mut().shutdown();
    Ok(())
}

#[allow(clippy::too_many_lines)]
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
        ui.on_rotate_preview(move || state.borrow_mut().rotate_preview());
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
        ui.on_open_page(move |row| state.borrow_mut().open_page(row as usize));
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
        ui.on_refresh_devices(move || {
            state.borrow_mut().worker.refresh();
            state.borrow_mut().set_status(i18n::t("status.redetecting"));
        });
    }
    {
        let weak = ui.as_weak();
        ui.on_open_about(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_show_about(true);
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.on_close_about(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_show_about(false);
            }
        });
    }
    ui.on_open_github(update::open_repo);
    {
        let state = std::rc::Rc::clone(state);
        ui.on_auto_check_changed(move |enabled| {
            let mut app = state.borrow_mut();
            app.config.auto_check_updates = Some(enabled);
            app.config.save();
            if enabled {
                app.start_update_check(false);
            }
        });
    }
    {
        let state = std::rc::Rc::clone(state);
        ui.on_install_update(move || state.borrow_mut().start_update_download());
    }
    {
        let state = std::rc::Rc::clone(state);
        ui.on_check_updates(move || state.borrow_mut().start_update_check(true));
    }
    {
        let state = std::rc::Rc::clone(state);
        ui.on_language_changed(move |index| {
            state
                .borrow_mut()
                .set_language(i18n::Lang::from_index(index as usize));
        });
    }
    {
        let state = std::rc::Rc::clone(state);
        ui.on_install_tesseract(move || state.borrow_mut().install_tesseract());
    }
    {
        let state = std::rc::Rc::clone(state);
        ui.on_cancel_tesseract(move || state.borrow_mut().cancel_tesseract_install());
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
    // 只在需要缩小时分配新缓冲区；否则直接借用采集帧，省掉一次整帧拷贝。
    let scaled = if scale < 1.0 {
        imaging::thumbnail_bgra(
            frame.width,
            frame.height,
            &frame.data,
            PREVIEW_MAX_SIDE,
            PREVIEW_MAX_SIDE,
        )
        .ok()
    } else {
        None
    };
    let (width, height, data) = match &scaled {
        Some((width, height, data)) => (*width, *height, data.as_slice()),
        None => (frame.width, frame.height, frame.data.as_slice()),
    };
    let mut image = bgra_to_buffer(width, height, data);
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
    let dst = buffer.make_mut_bytes();
    // 逐像素交换 R/B，并强制不透明：部分摄像头驱动把 alpha 写成 0，
    // 直接透传会让预览整块变黑。
    for (src, out) in data.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
        out[0] = src[2];
        out[1] = src[1];
        out[2] = src[0];
        out[3] = 255;
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

/// 把秒数格式化成录像计时：不足一小时为 `mm:ss`，否则为 `h:mm:ss`。
fn format_elapsed(total: u64) -> String {
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

fn default_output_root() -> PathBuf {
    usbscan_win::paths::default_output_root()
}
