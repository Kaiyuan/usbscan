// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! GitHub Releases 更新检查与安装包下载。

use std::fs;
use std::io::{Read as _, Write as _};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::http;

pub const REPO_URL: &str = "https://github.com/Kaiyuan/usbscan";
const RELEASES_API: &str = "https://api.github.com/repos/Kaiyuan/usbscan/releases/latest";
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
/// 进度回调的最小间隔，避免频繁刷新界面。
const PROGRESS_INTERVAL: Duration = Duration::from_millis(120);

/// 安装包下载进度。
#[derive(Clone, Copy, Debug, Default)]
pub struct DownloadProgress {
    pub received: u64,
    /// 服务器给出的总字节数；未知时为 `None`。
    pub total: Option<u64>,
}

impl DownloadProgress {
    /// 0.0–1.0 的完成比例；总量未知时返回 `None`。
    #[must_use]
    pub fn fraction(&self) -> Option<f32> {
        self.total
            .filter(|total| *total > 0)
            .map(|total| (self.received as f32 / total as f32).clamp(0.0, 1.0))
    }
}

/// 可用的新版本信息。
#[derive(Clone, Debug)]
pub struct UpdateInfo {
    pub version: String,
    pub release_url: String,
    pub installer_url: Option<String>,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// 查询 GitHub 最新 Release；有更新时返回版本信息。
pub fn check_latest() -> Result<Option<UpdateInfo>, String> {
    let response = http::agent()?
        .get(RELEASES_API)
        .set("User-Agent", "UsbScan")
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|err| err.to_string())?;
    let release: Release = response.into_json().map_err(|err| err.to_string())?;
    let version = release.tag_name.trim_start_matches(['v', 'V']).to_string();
    if !is_newer(&version, CURRENT_VERSION) {
        return Ok(None);
    }
    let installer = release
        .assets
        .iter()
        .find(|asset| {
            let name = asset.name.to_ascii_lowercase();
            name.contains("setup")
                && std::path::Path::new(&asset.name)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
        })
        .or_else(|| {
            release.assets.iter().find(|asset| {
                std::path::Path::new(&asset.name)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
            })
        })
        .map(|asset| asset.browser_download_url.clone());
    Ok(Some(UpdateInfo {
        version,
        release_url: release.html_url,
        installer_url: installer,
    }))
}

/// 下载安装包到临时目录，返回本地路径。
///
/// `progress` 会在下载过程中被调用，用于在状态栏显示进度。
pub fn download_installer(
    info: &UpdateInfo,
    progress: &dyn Fn(DownloadProgress),
) -> Result<PathBuf, String> {
    let url = info
        .installer_url
        .as_ref()
        .ok_or_else(|| crate::i18n::t("status.noInstaller").to_string())?;
    let directory = std::env::temp_dir().join("usbscan-update");
    fs::create_dir_all(&directory).map_err(|err| err.to_string())?;
    let path = directory.join(format!("usbscan-{}-setup.exe", info.version));
    let response = http::agent()?
        .get(url)
        .set("User-Agent", "UsbScan")
        .call()
        .map_err(|err| err.to_string())?;
    let total = response
        .header("Content-Length")
        .and_then(|value| value.parse::<u64>().ok());
    let temp = path.with_extension("download");
    let mut reader = response.into_reader();
    let mut file = fs::File::create(&temp).map_err(|err| err.to_string())?;
    let mut buffer = vec![0u8; 64 * 1024];
    let mut received = 0u64;
    let mut last_report = Instant::now();
    progress(DownloadProgress { received, total });
    loop {
        let read = reader.read(&mut buffer).map_err(|err| err.to_string())?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|err| err.to_string())?;
        received += read as u64;
        if last_report.elapsed() >= PROGRESS_INTERVAL {
            progress(DownloadProgress { received, total });
            last_report = Instant::now();
        }
    }
    file.flush().map_err(|err| err.to_string())?;
    progress(DownloadProgress { received, total });
    let _ = fs::remove_file(&path);
    fs::rename(&temp, &path).map_err(|err| err.to_string())?;
    Ok(path)
}

/// 打开项目主页。
pub fn open_repo() {
    let _ = webbrowser::open(REPO_URL);
}

fn is_newer(latest: &str, current: &str) -> bool {
    let parse = |value: &str| {
        value
            .split(|ch: char| !ch.is_ascii_digit())
            .filter(|part| !part.is_empty())
            .filter_map(|part| part.parse::<u64>().ok())
            .collect::<Vec<_>>()
    };
    let mut latest = parse(latest);
    let mut current = parse(current);
    latest.resize(3, 0);
    current.resize(3, 0);
    latest > current
}

#[cfg(test)]
mod tests {
    use super::{is_newer, DownloadProgress};

    #[test]
    fn compares_versions() {
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
    }

    #[test]
    fn reports_download_fraction() {
        let half = DownloadProgress {
            received: 50,
            total: Some(100),
        };
        assert_eq!(half.fraction(), Some(0.5));
        let unknown = DownloadProgress {
            received: 50,
            total: None,
        };
        assert_eq!(unknown.fraction(), None);
        let empty = DownloadProgress {
            received: 0,
            total: Some(0),
        };
        assert_eq!(empty.fraction(), None);
    }
}
