// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 一键下载并安装 Tesseract。
//!
//! 安装包取自 Tesseract 官方 GitHub Releases（Windows 64 位安装器）。
//! 安装完成后会检查 OCR 所需语言的 `traineddata`；缺失时从
//! `tessdata_fast` 下载到用户目录，并通过 `TESSDATA_PREFIX` 使用。

use std::fs;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Deserialize;

use usbscan_win::shell::{self, ElevatedError};

use crate::http;

const RELEASES_API: &str =
    "https://api.github.com/repos/tesseract-ocr/tesseract/releases?per_page=10";
const TESSDATA_URL: &str = "https://github.com/tesseract-ocr/tessdata_fast/raw/main";
const FALLBACK_INSTALLER: &str = "https://github.com/tesseract-ocr/tesseract/releases/download/5.5.3/tesseract-ocr-w64-setup-5.5.3.20260724.exe";
const USER_AGENT: &str = "UsbScan";
const API_TIMEOUT: Duration = Duration::from_secs(60);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);
/// 进度回调的最小间隔，避免频繁刷新界面。
const PROGRESS_INTERVAL: Duration = Duration::from_millis(120);

/// 当前正在进行的步骤。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stage {
    /// 下载 Tesseract 安装包。
    #[default]
    Downloading,
    /// 等待 UAC 授权并运行安装程序。
    Installing,
    /// 下载 OCR 语言数据。
    Languages,
}

/// 下载/安装进度。
#[derive(Clone, Copy, Debug, Default)]
pub struct Progress {
    pub stage: Stage,
    pub received: u64,
    /// 服务器给出的总字节数；未知时为 `None`。
    pub total: Option<u64>,
}

impl Progress {
    /// 0.0–1.0 的完成比例；总量未知时返回 `None`。
    #[must_use]
    pub fn fraction(&self) -> Option<f32> {
        self.total
            .filter(|total| *total > 0)
            .map(|total| (self.received as f32 / total as f32).clamp(0.0, 1.0))
    }
}

#[derive(Deserialize)]
struct Release {
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// 安装结果。
pub struct Installed {
    pub executable: PathBuf,
    /// 需要覆盖 `TESSDATA_PREFIX` 时返回自定义 tessdata 目录。
    pub tessdata: Option<PathBuf>,
}

/// 用户取消安装时返回的错误文本。
pub const CANCELLED: &str = "tesseract installation cancelled";

fn ensure_running(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::SeqCst) {
        Err(CANCELLED.to_string())
    } else {
        Ok(())
    }
}

/// 下载并静默安装 Tesseract，随后补齐 `ocr_language` 需要的语言数据。
///
/// `progress` 会在下载和安装过程中被调用，用于更新界面；
/// `cancel` 被置位后函数会尽快返回 [`CANCELLED`]。
pub fn install(
    ocr_language: &str,
    progress: &dyn Fn(Progress),
    cancel: &AtomicBool,
) -> Result<Installed, String> {
    progress(Progress::default());
    ensure_running(cancel)?;
    let agent = http::agent()?;

    // 已经装好 Tesseract（例如用户手动安装）时直接复用，不再重复下载安装。
    let executable =
        if let Some(existing) = usbscan_ocr::find_tesseract(None) {
            existing
        } else {
            let url = latest_installer_url(&agent);
            let file_name = url
                .rsplit('/')
                .next()
                .filter(|name| {
                    Path::new(name)
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
                })
                .unwrap_or("tesseract-setup.exe");
            let directory = std::env::temp_dir().join("usbscan-tesseract");
            fs::create_dir_all(&directory).map_err(|err| format!("create temp folder: {err}"))?;
            let installer = directory.join(file_name);
            if !installer.is_file() {
                download(
                    &agent,
                    &url,
                    &installer,
                    Stage::Downloading,
                    progress,
                    cancel,
                )?;
            }
            ensure_running(cancel)?;

            progress(Progress {
                stage: Stage::Installing,
                received: 0,
                total: None,
            });
            let exit = shell::run_elevated_and_wait(&installer, "/SILENT /NORESTART /SP-")
                .map_err(|err| match err {
                    ElevatedError::Cancelled => crate::i18n::t("tesseract.cancelled").to_string(),
                    other => format!("run installer: {other}"),
                })?;
            if exit != 0 {
                return Err(format!("installer exited with code {exit}"));
            }
            ensure_running(cancel)?;

            usbscan_ocr::find_tesseract(None)
                .ok_or_else(|| "tesseract.exe was not found after installation".to_string())?
        };

    let tessdata = ensure_languages(&agent, &executable, ocr_language, progress, cancel)?;
    Ok(Installed {
        executable,
        tessdata,
    })
}

fn latest_installer_url(agent: &ureq::Agent) -> String {
    let releases = agent
        .get(RELEASES_API)
        .set("User-Agent", USER_AGENT)
        .set("Accept", "application/vnd.github+json")
        .timeout(API_TIMEOUT)
        .call()
        .ok()
        .and_then(|response| response.into_json::<Vec<Release>>().ok());
    if let Some(releases) = releases {
        for release in releases {
            if let Some(url) = release
                .assets
                .into_iter()
                .find(|asset| {
                    asset.name.starts_with("tesseract-ocr-w64-setup-")
                        && Path::new(&asset.name)
                            .extension()
                            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
                })
                .map(|asset| asset.browser_download_url)
            {
                return url;
            }
        }
    }
    FALLBACK_INSTALLER.to_string()
}

fn download(
    agent: &ureq::Agent,
    url: &str,
    path: &Path,
    stage: Stage,
    progress: &dyn Fn(Progress),
    cancel: &AtomicBool,
) -> Result<(), String> {
    ensure_running(cancel)?;
    let response = agent
        .get(url)
        .set("User-Agent", USER_AGENT)
        .timeout(DOWNLOAD_TIMEOUT)
        .call()
        .map_err(|err| format!("download {url}: {err}"))?;
    let total = response
        .header("Content-Length")
        .and_then(|value| value.parse::<u64>().ok());
    let temp = path.with_extension("download");
    let mut reader = response.into_reader();
    let mut file =
        fs::File::create(&temp).map_err(|err| format!("create {}: {err}", temp.display()))?;
    let mut buffer = vec![0u8; 64 * 1024];
    let mut received = 0u64;
    let mut last_report = Instant::now();
    progress(Progress {
        stage,
        received,
        total,
    });
    loop {
        if cancel.load(Ordering::SeqCst) {
            drop(file);
            let _ = fs::remove_file(&temp);
            return Err(CANCELLED.to_string());
        }
        let read = reader
            .read(&mut buffer)
            .map_err(|err| format!("download {url}: {err}"))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|err| format!("write {}: {err}", temp.display()))?;
        received += read as u64;
        if last_report.elapsed() >= PROGRESS_INTERVAL {
            progress(Progress {
                stage,
                received,
                total,
            });
            last_report = Instant::now();
        }
    }
    file.flush()
        .map_err(|err| format!("write {}: {err}", temp.display()))?;
    progress(Progress {
        stage,
        received,
        total,
    });
    fs::rename(&temp, path).map_err(|err| format!("finish {}: {err}", path.display()))?;
    Ok(())
}

/// 确保 `tesseract.exe` 旁边或用户目录中存在所需语言的 traineddata。
fn ensure_languages(
    agent: &ureq::Agent,
    executable: &Path,
    languages: &str,
    progress: &dyn Fn(Progress),
    cancel: &AtomicBool,
) -> Result<Option<PathBuf>, String> {
    let required: Vec<&str> = languages
        .split('+')
        .map(str::trim)
        .filter(|language| {
            !language.is_empty()
                && language
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        })
        .collect();
    if required.is_empty() {
        return Ok(None);
    }

    let bundled = executable.parent().map(|parent| parent.join("tessdata"));
    if let Some(directory) = &bundled {
        if required
            .iter()
            .all(|language| directory.join(format!("{language}.traineddata")).is_file())
        {
            return Ok(None);
        }
    }

    let base = std::env::var_os("LOCALAPPDATA").map_or_else(std::env::temp_dir, PathBuf::from);
    let directory = base.join("usbscan").join("tessdata");
    fs::create_dir_all(&directory).map_err(|err| format!("create tessdata: {err}"))?;
    for language in required {
        let target = directory.join(format!("{language}.traineddata"));
        if target.is_file() {
            continue;
        }
        // 安装器自带的语言数据直接复制，避免重复下载（例如 eng）。
        if let Some(source) = bundled
            .as_ref()
            .map(|directory| directory.join(format!("{language}.traineddata")))
            .filter(|source| source.is_file())
        {
            fs::copy(&source, &target)
                .map_err(|err| format!("copy {}: {err}", source.display()))?;
            continue;
        }
        ensure_running(cancel)?;
        let url = format!("{TESSDATA_URL}/{language}.traineddata");
        download(agent, &url, &target, Stage::Languages, progress, cancel)?;
    }
    Ok(Some(directory))
}

#[cfg(test)]
mod tests {
    use super::{Progress, Stage};

    #[test]
    fn fraction_is_clamped() {
        let half = Progress {
            stage: Stage::Downloading,
            received: 50,
            total: Some(100),
        };
        assert_eq!(half.fraction(), Some(0.5));

        let over = Progress {
            received: 200,
            total: Some(100),
            ..Progress::default()
        };
        assert_eq!(over.fraction(), Some(1.0));

        let unknown = Progress {
            received: 5,
            total: None,
            ..Progress::default()
        };
        assert_eq!(unknown.fraction(), None);

        let empty = Progress {
            received: 0,
            total: Some(0),
            ..Progress::default()
        };
        assert_eq!(empty.fraction(), None);
    }
}
