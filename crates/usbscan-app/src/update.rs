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

/// 更新包类型，决定下载后如何安装。
///
/// 不同平台只会构造其中一部分变体（Windows 用 `WindowsSetup`），
/// 但安装分支需要跨平台共用，因此允许未构造的平台变体。
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageKind {
    /// Windows 安装器（`.exe`），下载后直接启动。
    WindowsSetup,
    /// Debian / UOS 安装包（`.deb`），通过 `pkexec dpkg -i` 安装。
    Deb,
    /// 便携包（`.tar.gz`），解压后在文件管理器中显示。
    Archive,
    /// macOS 磁盘映像（`.dmg`），下载后打开。
    Dmg,
}

/// 可用的新版本信息。
#[derive(Clone, Debug)]
pub struct UpdateInfo {
    pub version: String,
    pub release_url: String,
    pub installer_url: Option<String>,
    /// 与 `installer_url` 对应的安装包类型。
    pub kind: Option<PackageKind>,
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
    let package = platform_package(&release.assets);
    Ok(Some(UpdateInfo {
        version,
        release_url: release.html_url,
        installer_url: package.as_ref().map(|(url, _)| url.clone()),
        kind: package.map(|(_, kind)| kind),
    }))
}

/// 选择当前平台与架构对应的更新包。
///
/// 找不到匹配资源时返回 `None`，界面会退回到打开 Releases 页面。
#[cfg(windows)]
fn platform_package(assets: &[Asset]) -> Option<(String, PackageKind)> {
    assets
        .iter()
        .find(|asset| {
            let name = asset.name.to_ascii_lowercase();
            name.contains("setup")
                && std::path::Path::new(&asset.name)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
        })
        .or_else(|| {
            assets.iter().find(|asset| {
                std::path::Path::new(&asset.name)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
            })
        })
        .map(|asset| {
            (
                asset.browser_download_url.clone(),
                PackageKind::WindowsSetup,
            )
        })
}

/// macOS 优先使用 `.dmg`，没有时使用通用 `.tar.gz` 便携包。
#[cfg(target_os = "macos")]
fn platform_package(assets: &[Asset]) -> Option<(String, PackageKind)> {
    let macos = |asset: &&Asset| asset.name.to_ascii_lowercase().contains("macos");
    assets
        .iter()
        .filter(macos)
        .find(|asset| {
            std::path::Path::new(&asset.name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("dmg"))
        })
        .map(|asset| (asset.browser_download_url.clone(), PackageKind::Dmg))
        .or_else(|| {
            assets
                .iter()
                .filter(macos)
                .find(|asset| asset.name.to_ascii_lowercase().ends_with(".tar.gz"))
                .map(|asset| (asset.browser_download_url.clone(), PackageKind::Archive))
        })
}

/// Linux / UOS：优先 `.deb`，没有 dpkg 时使用 `.tar.gz`。
#[cfg(all(not(windows), not(target_os = "macos")))]
fn platform_package(assets: &[Asset]) -> Option<(String, PackageKind)> {
    let environment = LinuxEnvironment {
        deb_arch: if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "amd64"
        },
        arch_token: if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "x64"
        },
        uos_like: is_uos_like(),
        has_dpkg: has_dpkg(),
    };
    select_linux_package(assets, &environment)
}

/// 选择 Linux 更新包所需的环境信息（单独抽出便于测试）。
#[cfg(all(not(windows), not(target_os = "macos")))]
#[derive(Clone, Copy)]
struct LinuxEnvironment {
    deb_arch: &'static str,
    arch_token: &'static str,
    uos_like: bool,
    has_dpkg: bool,
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn select_linux_package(
    assets: &[Asset],
    environment: &LinuxEnvironment,
) -> Option<(String, PackageKind)> {
    if environment.has_dpkg {
        let deb = |asset: &&Asset| {
            let name = asset.name.to_ascii_lowercase();
            std::path::Path::new(&name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("deb"))
                && name.contains(environment.deb_arch)
        };
        // 优先同发行版的包：UOS 选 uos 包，普通 Debian/Ubuntu 选通用包。
        if let Some(asset) = assets
            .iter()
            .filter(deb)
            .find(|asset| asset.name.to_ascii_lowercase().contains("_uos_") == environment.uos_like)
            .or_else(|| assets.iter().find(deb))
        {
            return Some((asset.browser_download_url.clone(), PackageKind::Deb));
        }
    }

    let archive = |asset: &&Asset| {
        let name = asset.name.to_ascii_lowercase();
        name.ends_with(".tar.gz") && name.contains(environment.arch_token)
    };
    let preferred = assets.iter().filter(archive).find(|asset| {
        let name = asset.name.to_ascii_lowercase();
        name.contains(if environment.uos_like { "uos" } else { "linux" })
    });
    preferred
        .or_else(|| assets.iter().find(archive))
        .or_else(|| {
            assets
                .iter()
                .find(|asset| asset.name.to_ascii_lowercase().ends_with(".tar.gz"))
        })
        .map(|asset| (asset.browser_download_url.clone(), PackageKind::Archive))
}

/// 判断当前系统是否 UOS / Deepin。
#[cfg(all(not(windows), not(target_os = "macos")))]
fn is_uos_like() -> bool {
    std::fs::read_to_string("/etc/os-release").is_ok_and(|text| {
        text.lines()
            .filter(|line| {
                line.starts_with("NAME=") || line.starts_with("ID=") || line.starts_with("ID_LIKE=")
            })
            .any(|line| {
                let value = line.to_ascii_lowercase();
                value.contains("uos") || value.contains("deepin")
            })
    })
}

/// 系统是否提供 dpkg（Debian / UOS / Ubuntu）。
#[cfg(all(not(windows), not(target_os = "macos")))]
fn has_dpkg() -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|directory| directory.join("dpkg").is_file())
    })
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
    let path = directory.join(local_file_name(info, url));
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

/// 用 URL 中的文件名作为本地文件名，保留 `.deb` / `.tar.gz` 等真实扩展名。
fn local_file_name(info: &UpdateInfo, url: &str) -> String {
    let from_url = url.rsplit('/').next().unwrap_or_default();
    let cleaned: String = from_url
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        format!("usbscan-{}-update.bin", info.version)
    } else {
        cleaned
    }
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
    use super::{is_newer, local_file_name, DownloadProgress, UpdateInfo};

    #[cfg(all(not(windows), not(target_os = "macos")))]
    #[test]
    fn selects_linux_update_package_for_platform() {
        use super::{select_linux_package, Asset, LinuxEnvironment, PackageKind};

        let asset = |name: &str| Asset {
            name: name.to_string(),
            browser_download_url: format!("https://example.com/{name}"),
        };
        let assets = vec![
            asset("usbscan-0.1.7-linux-x64.tar.gz"),
            asset("usbscan-0.1.7-uos-x64.tar.gz"),
            asset("usbscan_0.1.7_amd64.deb"),
            asset("usbscan_0.1.7_uos_amd64.deb"),
            asset("usbscan_0.1.7_arm64.deb"),
        ];
        let generic = LinuxEnvironment {
            deb_arch: "amd64",
            arch_token: "x64",
            uos_like: false,
            has_dpkg: true,
        };
        let (url, kind) = select_linux_package(&assets, &generic).unwrap();
        assert_eq!(kind, PackageKind::Deb);
        assert!(url.ends_with("usbscan_0.1.7_amd64.deb"));

        let uos = LinuxEnvironment {
            uos_like: true,
            ..generic
        };
        let (url, kind) = select_linux_package(&assets, &uos).unwrap();
        assert_eq!(kind, PackageKind::Deb);
        assert!(url.ends_with("usbscan_0.1.7_uos_amd64.deb"));

        let arm = LinuxEnvironment {
            deb_arch: "arm64",
            arch_token: "arm64",
            uos_like: false,
            has_dpkg: true,
        };
        let (url, _) = select_linux_package(&assets, &arm).unwrap();
        assert!(url.ends_with("usbscan_0.1.7_arm64.deb"));

        let no_dpkg = LinuxEnvironment {
            has_dpkg: false,
            ..generic
        };
        let (url, kind) = select_linux_package(&assets, &no_dpkg).unwrap();
        assert_eq!(kind, PackageKind::Archive);
        assert!(url.ends_with("usbscan-0.1.7-linux-x64.tar.gz"));
    }

    #[test]
    fn keeps_package_file_name_and_extension() {
        let info = UpdateInfo {
            version: "0.1.7".to_string(),
            release_url: String::new(),
            installer_url: None,
            kind: None,
        };
        assert_eq!(
            local_file_name(
                &info,
                "https://github.com/Kaiyuan/usbscan/releases/download/v0.1.7/usbscan_0.1.7_uos_amd64.deb"
            ),
            "usbscan_0.1.7_uos_amd64.deb"
        );
        assert_eq!(
            local_file_name(
                &info,
                "https://github.com/Kaiyuan/usbscan/releases/download/v0.1.7/usbscan-0.1.7-macos-universal.tar.gz"
            ),
            "usbscan-0.1.7-macos-universal.tar.gz"
        );
        assert_eq!(
            local_file_name(&info, "https://example.com/"),
            "usbscan-0.1.7-update.bin"
        );
    }

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
