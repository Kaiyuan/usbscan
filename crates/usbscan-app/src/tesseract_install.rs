// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 一键下载并安装 Tesseract。
//!
//! 安装包取自 Tesseract 官方 GitHub Releases（Windows 64 位安装器）。
//! 安装完成后会检查 OCR 所需语言的 `traineddata`；缺失时从
//! `tessdata_fast` 下载到用户目录，并通过 `TESSDATA_PREFIX` 使用。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;

use crate::http;

const RELEASES_API: &str =
    "https://api.github.com/repos/tesseract-ocr/tesseract/releases?per_page=10";
const TESSDATA_URL: &str = "https://github.com/tesseract-ocr/tessdata_fast/raw/main";
const FALLBACK_INSTALLER: &str = "https://github.com/tesseract-ocr/tesseract/releases/download/5.5.3/tesseract-ocr-w64-setup-5.5.3.20260724.exe";
const USER_AGENT: &str = "UsbScan";
const API_TIMEOUT: Duration = Duration::from_secs(60);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);

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

/// 下载并静默安装 Tesseract，随后补齐 `ocr_language` 需要的语言数据。
pub fn install(ocr_language: &str) -> Result<Installed, String> {
    let agent = http::agent()?;
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
        download(&agent, &url, &installer)?;
    }

    let status = Command::new(&installer)
        .args(["/SILENT", "/NORESTART", "/SP-"])
        .status()
        .map_err(|err| format!("run installer: {err}"))?;
    if !status.success() {
        return Err(format!(
            "installer exited with code {}",
            status.code().unwrap_or(-1)
        ));
    }

    let executable = usbscan_ocr::find_tesseract(None)
        .ok_or_else(|| "tesseract.exe was not found after installation".to_string())?;
    let tessdata = ensure_languages(&agent, &executable, ocr_language)?;
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

fn download(agent: &ureq::Agent, url: &str, path: &Path) -> Result<(), String> {
    let response = agent
        .get(url)
        .set("User-Agent", USER_AGENT)
        .timeout(DOWNLOAD_TIMEOUT)
        .call()
        .map_err(|err| format!("download {url}: {err}"))?;
    let temp = path.with_extension("download");
    let mut reader = response.into_reader();
    let mut file =
        fs::File::create(&temp).map_err(|err| format!("create {}: {err}", temp.display()))?;
    io::copy(&mut reader, &mut file).map_err(|err| format!("write {}: {err}", temp.display()))?;
    fs::rename(&temp, path).map_err(|err| format!("finish {}: {err}", path.display()))?;
    Ok(())
}

/// 确保 `tesseract.exe` 旁边或用户目录中存在所需语言的 traineddata。
fn ensure_languages(
    agent: &ureq::Agent,
    executable: &Path,
    languages: &str,
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

    if let Some(directory) = executable.parent().map(|parent| parent.join("tessdata")) {
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
        if !target.is_file() {
            let url = format!("{TESSDATA_URL}/{language}.traineddata");
            download(agent, &url, &target)?;
        }
    }
    Ok(Some(directory))
}
