// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! GitHub Releases 更新检查与安装包下载。

use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;

pub const REPO_URL: &str = "https://github.com/Kaiyuan/usbscan";
const RELEASES_API: &str = "https://api.github.com/repos/Kaiyuan/usbscan/releases/latest";
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

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
    let response = agent()?
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
pub fn download_installer(info: &UpdateInfo) -> Result<PathBuf, String> {
    let url = info
        .installer_url
        .as_ref()
        .ok_or_else(|| crate::i18n::t("status.noInstaller").to_string())?;
    let directory = std::env::temp_dir().join("usbscan-update");
    fs::create_dir_all(&directory).map_err(|err| err.to_string())?;
    let path = directory.join(format!("usbscan-{}-setup.exe", info.version));
    let response = agent()?
        .get(url)
        .set("User-Agent", "UsbScan")
        .call()
        .map_err(|err| err.to_string())?;
    let mut reader = response.into_reader();
    let mut file = fs::File::create(&path).map_err(|err| err.to_string())?;
    io::copy(&mut reader, &mut file).map_err(|err| err.to_string())?;
    Ok(path)
}

/// 打开项目主页。
pub fn open_repo() {
    let _ = webbrowser::open(REPO_URL);
}

/// 使用系统 TLS（Windows 上是 SChannel）构建 Agent。
///
/// `ureq` 的快捷函数只支持 rustls；关闭默认特性后必须显式传入 native-tls
/// 连接器，否则 HTTPS 请求会报 “no TLS backend is configured”。
fn agent() -> Result<ureq::Agent, String> {
    let connector = native_tls::TlsConnector::new().map_err(|err| err.to_string())?;
    Ok(ureq::AgentBuilder::new()
        .tls_connector(Arc::new(connector))
        .build())
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
    use super::is_newer;

    #[test]
    fn compares_versions() {
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
    }
}
