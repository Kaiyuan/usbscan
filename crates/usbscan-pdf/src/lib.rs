// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! PDF 页面渲染为图片（跨平台实现）。
//!
//! **Windows**：使用自带的 `Windows.Data.Pdf` 渲染页面，不依赖第三方。
//!
//! **macOS / Linux / UOS**：使用系统标准 `pdftoppm`（Poppler）命令行工具。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use usbscan_imaging as imaging;

/// 渲染 DPI。
pub const RENDER_DPI: u32 = 200;

/// 渲染出的一页。
#[derive(Debug, Clone)]
pub struct PdfPageImage {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
}

/// PDF 渲染错误。
#[derive(Debug)]
pub enum PdfError {
    #[cfg(windows)]
    Windows(windows::core::Error),
    Io(io::Error),
    Render(String),
}

impl std::fmt::Display for PdfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(windows)]
            Self::Windows(err) => write!(f, "Windows PDF 组件错误：{err}"),
            Self::Io(err) => write!(f, "文件读写失败：{err}"),
            Self::Render(err) => write!(f, "PDF 渲染失败：{err}"),
        }
    }
}

impl std::error::Error for PdfError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            #[cfg(windows)]
            Self::Windows(err) => Some(err),
            Self::Io(err) => Some(err),
            Self::Render(_) => None,
        }
    }
}

#[cfg(windows)]
impl From<windows::core::Error> for PdfError {
    fn from(err: windows::core::Error) -> Self {
        Self::Windows(err)
    }
}

impl From<io::Error> for PdfError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

// ===========================================================================
// Windows 实现（使用 Windows.Data.Pdf）
// ===========================================================================

#[cfg(windows)]
mod windows_impl {
    use super::*;
    use windows::core::HSTRING;
    use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
    use windows::Storage::StorageFile;
    use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};

    pub fn render_pdf(
        pdf: &Path,
        output_dir: &Path,
        quality: u8,
    ) -> Result<Vec<PdfPageImage>, PdfError> {
        fs::create_dir_all(output_dir)?;
        let mut temporary = None;
        let direct = StorageFile::GetFileFromPathAsync(&HSTRING::from(pdf.to_string_lossy().as_ref()))
            .and_then(|operation| operation.get());
        let file = if let Ok(file) = direct {
            file
        } else {
            let temp = std::env::temp_dir().join(format!(
                "usbscan-pdf-{}-{}.pdf",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |duration| duration.as_millis())
            ));
            fs::copy(pdf, &temp)?;
            let file =
                StorageFile::GetFileFromPathAsync(&HSTRING::from(temp.to_string_lossy().as_ref()))?
                    .get()?;
            temporary = Some(temp);
            file
        };
        let result = render_document(&file, output_dir, quality);
        if let Some(temp) = temporary {
            let _ = fs::remove_file(temp);
        }
        result
    }

    fn render_document(
        file: &StorageFile,
        output_dir: &Path,
        quality: u8,
    ) -> Result<Vec<PdfPageImage>, PdfError> {
        let document = PdfDocument::LoadFromFileAsync(file)?.get()?;
        let count = document.PageCount()?;
        if count == 0 {
            return Err(PdfError::Render("PDF 没有页面".to_string()));
        }

        let mut pages = Vec::with_capacity(count as usize);
        for index in 0..count {
            let page = document.GetPage(index)?;
            let size = page.Size()?;
            let width = (f64::from(size.Width) * f64::from(RENDER_DPI) / 72.0)
                .round()
                .clamp(1.0, 20_000.0) as u32;
            let height = (f64::from(size.Height) * f64::from(RENDER_DPI) / 72.0)
                .round()
                .clamp(1.0, 20_000.0) as u32;
            let options = PdfPageRenderOptions::new()?;
            options.SetDestinationWidth(width)?;
            options.SetDestinationHeight(height)?;

            let stream = InMemoryRandomAccessStream::new()?;
            page.RenderWithOptionsToStreamAsync(&stream, &options)?
                .get()?;
            let size_bytes = stream.Size()?;
            stream.Seek(0)?;
            let reader = DataReader::CreateDataReader(&stream)?;
            reader
                .LoadAsync(u32::try_from(size_bytes).unwrap_or(u32::MAX))?
                .get()?;
            let mut bytes = vec![0u8; size_bytes as usize];
            reader.ReadBytes(&mut bytes)?;
            drop(reader);
            drop(stream);

            let temp = output_dir.join(format!("pdf-page-{:04}.png", index + 1));
            fs::write(&temp, &bytes)?;
            let target = unique_path(&output_dir.join(format!("PDF_{:03}.jpg", index + 1)));
            let mut file = fs::File::create(&target)?;
            let dimensions = imaging::transcode_to_jpeg(&temp, quality, &mut file)
                .map_err(|err| PdfError::Render(err.to_string()))?;
            file.sync_all()?;
            drop(file);
            let _ = fs::remove_file(&temp);
            pages.push(PdfPageImage {
                path: target,
                width: dimensions.0,
                height: dimensions.1,
            });
        }
        Ok(pages)
    }
}

// ===========================================================================
// Unix 实现（macOS / Linux / UOS：使用 pdftoppm）
// ===========================================================================

#[cfg(not(windows))]
mod unix_impl {
    use super::*;
    use std::process::Command;

    pub fn render_pdf(
        pdf: &Path,
        output_dir: &Path,
        quality: u8,
    ) -> Result<Vec<PdfPageImage>, PdfError> {
        fs::create_dir_all(output_dir)?;

        // 检查 pdftoppm 是否在 PATH
        let has_pdftoppm = which_command("pdftoppm").is_some();
        if !has_pdftoppm {
            #[cfg(target_os = "macos")]
            let tip = "未找到 pdftoppm 工具。在 macOS 上请通过 Homebrew 安装：\nbrew install poppler";
            #[cfg(target_os = "linux")]
            let tip = "未找到 pdftoppm 工具。在 UOS / Linux 上请安装：\nsudo apt install poppler-utils";
            #[cfg(not(any(target_os = "macos", target_os = "linux")))]
            let tip = "未找到 pdftoppm，请安装 poppler 工具包";
            return Err(PdfError::Render(tip.to_string()));
        }

        let prefix = output_dir.join(format!("pdftoppm-{}", std::process::id()));
        let prefix_str = prefix.to_string_lossy();

        // 运行：pdftoppm -jpeg -r <RENDER_DPI> -jpegopt quality=<quality> <pdf> <prefix>
        let status = Command::new("pdftoppm")
            .arg("-jpeg")
            .arg("-r")
            .arg(RENDER_DPI.to_string())
            .arg("-jpegopt")
            .arg(format!("quality={quality}"))
            .arg(pdf)
            .arg(prefix_str.as_ref())
            .status()
            .map_err(|err| PdfError::Render(format!("运行 pdftoppm 失败：{err}")))?;

        if !status.success() {
            return Err(PdfError::Render(format!(
                "pdftoppm 退出码非零：{}",
                status.code().unwrap_or(-1)
            )));
        }

        // 收集生成的文件（格式：prefix-1.jpg, prefix-01.jpg, 等）
        let mut generated = Vec::new();
        if let Ok(entries) = fs::read_dir(output_dir) {
            let prefix_file_stem = prefix.file_name().unwrap().to_string_lossy();
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if name.starts_with(prefix_file_stem.as_ref()) && name.ends_with(".jpg") {
                            generated.push(path);
                        }
                    }
                }
            }
        }

        generated.sort();
        if generated.is_empty() {
            return Err(PdfError::Render("PDF 渲染后未找到输出页面".to_string()));
        }

        let mut pages = Vec::with_capacity(generated.len());
        for (idx, gen_path) in generated.into_iter().enumerate() {
            let target = unique_path(&output_dir.join(format!("PDF_{:03}.jpg", idx + 1)));
            fs::rename(&gen_path, &target)?;

            // 读取图片尺寸
            let (width, height) = match image::image_dimensions(&target) {
                Ok((w, h)) => (w, h),
                Err(_) => (1000, 1400),
            };

            pages.push(PdfPageImage {
                path: target,
                width,
                height,
            });
        }

        Ok(pages)
    }

    fn which_command(name: &str) -> Option<PathBuf> {
        let path_var = std::env::var_os("PATH")?;
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    }
}

// ===========================================================================
// 统一导出
// ===========================================================================

/// 把 PDF 的每一页渲染到 `output_dir`，返回生成的 JPEG。
pub fn render_pdf(
    pdf: &Path,
    output_dir: &Path,
    quality: u8,
) -> Result<Vec<PdfPageImage>, PdfError> {
    #[cfg(windows)]
    return windows_impl::render_pdf(pdf, output_dir, quality);
    #[cfg(not(windows))]
    return unix_impl::render_pdf(pdf, output_dir, quality);
}

fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path.file_stem().map_or_else(
        || "page".to_string(),
        |stem| stem.to_string_lossy().into_owned(),
    );
    let extension = path.extension().map_or_else(
        || "jpg".to_string(),
        |ext| ext.to_string_lossy().into_owned(),
    );
    for counter in 1..10_000 {
        let candidate = parent.join(format!("{stem}-{counter}.{extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{stem}-{}.{extension}", std::process::id()))
}
