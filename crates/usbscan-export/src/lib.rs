// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 轻量 PDF 导出。
//!
//! 会话中的成品已经是 JPEG，因此这里直接把 JPEG 以 `/DCTDecode` 图像
//! 嵌入 PDF，不重新编码、不依赖第三方 PDF 库。支持纸张尺寸、方向、页边距
//! 与“适应 / 填满”两种排版方式。

#![allow(clippy::cast_precision_loss)]

use std::fmt::Write as _;
use std::io::{self, Write};

mod avi;
mod docx;

pub use avi::AviWriter;
pub use docx::{write_docx, DocxPage};

/// 一页待导出的 JPEG。
pub struct PdfImage<'a> {
    pub jpeg: &'a [u8],
    pub width: u32,
    pub height: u32,
}

/// 纸张尺寸。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageSize {
    /// A4 210×297 mm。
    #[default]
    A4,
    /// US Letter 8.5×11 in。
    Letter,
    /// 页面尺寸等于图片尺寸（按 96 DPI 换算）。
    FitImage,
}

/// 页面方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Orientation {
    /// 图片横放时自动使用横向页面。
    #[default]
    Auto,
    Portrait,
    Landscape,
}

/// 图片在页面中的缩放方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FitMode {
    /// 完整显示，保持长宽比。
    #[default]
    Contain,
    /// 铺满页面，保持长宽比（可能裁掉边缘）。
    Fill,
}

/// PDF 排版选项。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PdfOptions {
    pub page_size: PageSize,
    pub orientation: Orientation,
    /// 页边距（毫米）。
    pub margin_mm: f64,
    pub fit: FitMode,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            page_size: PageSize::A4,
            orientation: Orientation::Auto,
            margin_mm: 12.7,
            fit: FitMode::Contain,
        }
    }
}

impl PdfOptions {
    /// 每页的 PDF 点尺寸（1 pt = 1/72 in）。
    #[must_use]
    pub fn page_points(&self, image_width: u32, image_height: u32) -> (f64, f64) {
        let margin = self.margin_mm.clamp(0.0, 50.0) * 72.0 / 25.4;
        let (mut width, mut height) = match self.page_size {
            PageSize::A4 => (595.276, 841.890),
            PageSize::Letter => (612.0, 792.0),
            PageSize::FitImage => (
                f64::from(image_width.max(1)) * 0.75 + margin * 2.0,
                f64::from(image_height.max(1)) * 0.75 + margin * 2.0,
            ),
        };
        let landscape = match self.orientation {
            Orientation::Auto => image_width > image_height,
            Orientation::Portrait => false,
            Orientation::Landscape => true,
        };
        if landscape && self.page_size != PageSize::FitImage && width < height {
            std::mem::swap(&mut width, &mut height);
        }
        (width, height)
    }
}

/// 使用默认排版（A4、自动方向、12.7 mm 边距、适应页面）生成 PDF。
pub fn write_pdf<W: Write>(out: W, pages: &[PdfImage<'_>]) -> io::Result<()> {
    write_pdf_with(out, pages, &PdfOptions::default())
}

/// 按指定排版生成 PDF 并写入 `out`。
pub fn write_pdf_with<W: Write>(
    out: W,
    pages: &[PdfImage<'_>],
    options: &PdfOptions,
) -> io::Result<()> {
    let mut writer = PdfWriter {
        out,
        position: 0,
        offsets: Vec::new(),
    };
    writer.bytes(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n")?;

    let mut kids = Vec::with_capacity(pages.len());
    let mut next_object = 3usize;
    for _ in pages {
        kids.push(next_object);
        next_object += 3;
    }

    writer.object(1, b"<< /Type /Catalog /Pages 2 0 R >>")?;
    let kids_text = kids
        .iter()
        .map(|number| format!("{number} 0 R"))
        .collect::<Vec<_>>()
        .join(" ");
    writer.object(
        2,
        format!(
            "<< /Type /Pages /Kids [{kids_text}] /Count {} >>",
            pages.len()
        )
        .as_bytes(),
    )?;

    for (index, image) in pages.iter().enumerate() {
        let page_object = 3 + index * 3;
        let content_object = page_object + 1;
        let image_object = page_object + 2;
        let (page_width, page_height) = options.page_points(image.width, image.height);

        writer.object(
            page_object,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {page_width:.3} {page_height:.3}] \
                 /Resources << /XObject << /Im0 {image_object} 0 R >> >> \
                 /Contents {content_object} 0 R >>"
            )
            .as_bytes(),
        )?;

        let content = page_content(image, options, page_width, page_height);
        writer.object(
            content_object,
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            )
            .as_bytes(),
        )?;

        writer.image_object(image_object, image)?;
    }

    let xref_offset = writer.position;
    let size = writer.offsets.len() + 1;
    let mut xref = format!("xref\n0 {size}\n0000000000 65535 f \n");
    for offset in &writer.offsets {
        let _ = writeln!(xref, "{offset:010} 00000 n ");
    }
    writer.text(&xref)?;
    writer.text(&format!(
        "trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n"
    ))?;
    writer.out.flush()
}

/// 读取 JPEG SOF 段中的分量数，选择正确的 PDF 色彩空间。
fn jpeg_color_space(jpeg: &[u8]) -> &'static str {
    let mut index = 2usize;
    while index + 1 < jpeg.len() {
        if jpeg[index] != 0xFF {
            index += 1;
            continue;
        }
        let marker = jpeg[index + 1];
        if matches!(
            marker,
            0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF
        ) && index + 9 < jpeg.len()
        {
            return match jpeg[index + 9] {
                1 => "/DeviceGray",
                4 => "/DeviceCMYK",
                _ => "/DeviceRGB",
            };
        }
        if marker == 0xD8 || marker == 0xD9 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            index += 2;
        } else if index + 3 < jpeg.len() {
            let length = usize::from(u16::from_be_bytes([jpeg[index + 2], jpeg[index + 3]]));
            index += 2 + length.max(2);
        } else {
            break;
        }
    }
    "/DeviceRGB"
}

fn page_content(
    image: &PdfImage<'_>,
    options: &PdfOptions,
    page_width: f64,
    page_height: f64,
) -> String {
    let image_width = f64::from(image.width.max(1));
    let image_height = f64::from(image.height.max(1));
    let margin = options.margin_mm.clamp(0.0, 50.0) * 72.0 / 25.4;
    let (draw_width, draw_height) = if options.page_size == PageSize::FitImage {
        (image_width * 0.75, image_height * 0.75)
    } else {
        let max_width = (page_width - margin * 2.0).max(1.0);
        let max_height = (page_height - margin * 2.0).max(1.0);
        let scale = match options.fit {
            FitMode::Contain => (max_width / image_width).min(max_height / image_height),
            FitMode::Fill => (max_width / image_width).max(max_height / image_height),
        };
        (image_width * scale, image_height * scale)
    };
    let x = (page_width - draw_width) / 2.0;
    let y = (page_height - draw_height) / 2.0;
    format!("q\n{draw_width:.3} 0 0 {draw_height:.3} {x:.3} {y:.3} cm\n/Im0 Do\nQ")
}

struct PdfWriter<W: Write> {
    out: W,
    position: u64,
    offsets: Vec<u64>,
}

impl<W: Write> PdfWriter<W> {
    fn bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.out.write_all(bytes)?;
        self.position += bytes.len() as u64;
        Ok(())
    }

    fn text(&mut self, text: &str) -> io::Result<()> {
        self.bytes(text.as_bytes())
    }

    fn object(&mut self, number: usize, body: &[u8]) -> io::Result<()> {
        debug_assert_eq!(number, self.offsets.len() + 1);
        self.offsets.push(self.position);
        self.text(&format!("{number} 0 obj\n"))?;
        self.bytes(body)?;
        self.text("\nendobj\n")
    }

    fn image_object(&mut self, number: usize, image: &PdfImage<'_>) -> io::Result<()> {
        debug_assert_eq!(number, self.offsets.len() + 1);
        self.offsets.push(self.position);
        self.text(&format!(
            "{number} 0 obj\n<< /Type /XObject /Subtype /Image /Width {} /Height {} \
             /ColorSpace {} /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
            image.width.max(1),
            image.height.max(1),
            jpeg_color_space(image.jpeg),
            image.jpeg.len()
        ))?;
        self.bytes(image.jpeg)?;
        self.text("\nendstream\nendobj\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use usbscan_imaging::encode_jpeg_bgra;

    fn sample_jpeg(width: u32, height: u32) -> Vec<u8> {
        let bgra = vec![255u8; width as usize * height as usize * 4];
        let mut jpeg = Vec::new();
        encode_jpeg_bgra(width, height, &bgra, 90, &mut jpeg).expect("jpeg");
        jpeg
    }

    #[test]
    fn writes_pdf_structure_and_offsets() {
        let first = sample_jpeg(8, 6);
        let second = sample_jpeg(4, 8);
        let pages = [
            PdfImage {
                jpeg: &first,
                width: 8,
                height: 6,
            },
            PdfImage {
                jpeg: &second,
                width: 4,
                height: 8,
            },
        ];
        let mut out = Vec::new();
        write_pdf(&mut out, &pages).expect("pdf");

        let text = String::from_utf8_lossy(&out);
        assert!(text.starts_with("%PDF-1.4"));
        assert!(text.contains("/Count 2"));
        assert!(text.contains("/DCTDecode"));
        assert!(text.contains("/Type /Page "));
        assert!(text.ends_with("%%EOF\n"));

        // 每个 xref 偏移都应指向对应的 `N 0 obj`。
        let start_marker = b"startxref\n";
        let start = out
            .windows(start_marker.len())
            .rposition(|window| window == start_marker)
            .expect("startxref");
        let rest = std::str::from_utf8(&out[start + start_marker.len()..]).expect("ascii");
        let xref_position = rest
            .lines()
            .next()
            .and_then(|line| line.parse::<usize>().ok())
            .expect("xref offset");
        assert!(out[xref_position..].starts_with(b"xref"));
        let mut offset = xref_position
            + out[xref_position..]
                .windows(20)
                .position(|window| window == b"0000000000 65535 f \n")
                .map_or(0, |index| index + 20);
        for number in 1..=8 {
            let line = std::str::from_utf8(&out[offset..offset + 20]).expect("xref entry");
            let position = line[..10].parse::<usize>().expect("offset");
            assert!(
                out[position..].starts_with(format!("{number} 0 obj").as_bytes()),
                "object {number} offset mismatch"
            );
            offset += 20;
        }

        // 保留一份样张，便于人工检查 PDF 阅读器中的显示效果。
        let directory =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-tmp");
        std::fs::create_dir_all(&directory).expect("dir");
        std::fs::write(directory.join("usbscan-sample.pdf"), &out).expect("sample pdf");
    }

    #[test]
    fn writes_empty_pdf() {
        let mut out = Vec::new();
        write_pdf(&mut out, &[]).expect("pdf");
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("/Count 0"));
        assert!(text.ends_with("%%EOF\n"));
    }

    #[test]
    fn auto_orientation_uses_landscape_for_wide_images() {
        let jpeg = sample_jpeg(8, 6);
        let pages = [PdfImage {
            jpeg: &jpeg,
            width: 8,
            height: 6,
        }];
        let mut out = Vec::new();
        write_pdf_with(&mut out, &pages, &PdfOptions::default()).expect("pdf");
        let text = String::from_utf8_lossy(&out);
        assert!(
            text.contains("/MediaBox [0 0 841.890 595.276]"),
            "landscape A4 expected"
        );

        let mut portrait = Vec::new();
        write_pdf_with(
            &mut portrait,
            &pages,
            &PdfOptions {
                orientation: Orientation::Portrait,
                ..PdfOptions::default()
            },
        )
        .expect("pdf");
        assert!(String::from_utf8_lossy(&portrait).contains("/MediaBox [0 0 595.276 841.890]"));
    }

    #[test]
    fn fit_image_uses_image_dimensions() {
        let jpeg = sample_jpeg(800, 600);
        let pages = [PdfImage {
            jpeg: &jpeg,
            width: 800,
            height: 600,
        }];
        let mut out = Vec::new();
        write_pdf_with(
            &mut out,
            &pages,
            &PdfOptions {
                page_size: PageSize::FitImage,
                margin_mm: 0.0,
                ..PdfOptions::default()
            },
        )
        .expect("pdf");
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("/MediaBox [0 0 600.000 450.000]"), "{text}");
    }

    #[test]
    fn letter_and_fill_mode_render() {
        let jpeg = sample_jpeg(40, 60);
        let pages = [PdfImage {
            jpeg: &jpeg,
            width: 40,
            height: 60,
        }];
        let mut out = Vec::new();
        write_pdf_with(
            &mut out,
            &pages,
            &PdfOptions {
                page_size: PageSize::Letter,
                orientation: Orientation::Portrait,
                margin_mm: 20.0,
                fit: FitMode::Fill,
            },
        )
        .expect("pdf");
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("/MediaBox [0 0 612.000 792.000]"));
        assert!(text.ends_with("%%EOF\n"));
    }
}
