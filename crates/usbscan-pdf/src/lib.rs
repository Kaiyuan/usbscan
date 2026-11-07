// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 使用 Windows 自带的 `Windows.Data.Pdf` 渲染 PDF 页面。
//!
//! 不依赖第三方 PDF 引擎，也不需要联网。渲染结果先写成临时 PNG，
//! 再由 `usbscan-imaging` 转成 JPEG。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use usbscan_imaging as imaging;
use windows::core::{Error as WindowsError, HSTRING};
use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
use windows::Storage::StorageFile;
use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};

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
    Windows(WindowsError),
    Io(io::Error),
    Render(String),
}

impl std::fmt::Display for PdfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Windows(err) => write!(f, "Windows PDF 组件错误：{err}"),
            Self::Io(err) => write!(f, "文件读写失败：{err}"),
            Self::Render(err) => write!(f, "PDF 渲染失败：{err}"),
        }
    }
}

impl std::error::Error for PdfError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Windows(err) => Some(err),
            Self::Io(err) => Some(err),
            Self::Render(_) => None,
        }
    }
}

impl From<WindowsError> for PdfError {
    fn from(err: WindowsError) -> Self {
        Self::Windows(err)
    }
}

impl From<io::Error> for PdfError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

/// 把 PDF 的每一页渲染到 `output_dir`，返回生成的 JPEG。
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
        // WinRT 对非系统盘或受限目录可能拒绝直接打开，先复制到临时目录。
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
