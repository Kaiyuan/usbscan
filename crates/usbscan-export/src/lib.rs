// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 轻量 PDF 导出。
//!
//! 会话中的成品已经是 JPEG，因此这里直接把 JPEG 以 `/DCTDecode` 图像
//! 嵌入 PDF，不重新编码、不依赖第三方 PDF 库。每页使用 A4 纸张并按比例
//! 居中，适合文档扫描件的归档与打印。

use std::fmt::Write as _;
use std::io::{self, Write};

/// 一页待导出的 JPEG。
pub struct PdfImage<'a> {
    pub jpeg: &'a [u8],
    pub width: u32,
    pub height: u32,
}

const PAGE_WIDTH: f64 = 595.276; // A4 210mm
const PAGE_HEIGHT: f64 = 841.890; // A4 297mm
const MARGIN: f64 = 36.0; // 12.7mm

/// 生成 PDF 并写入 `out`。
pub fn write_pdf<W: Write>(out: W, pages: &[PdfImage<'_>]) -> io::Result<()> {
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

        writer.object(
            page_object,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_WIDTH:.3} {PAGE_HEIGHT:.3}] \
                 /Resources << /XObject << /Im0 {image_object} 0 R >> >> \
                 /Contents {content_object} 0 R >>"
            )
            .as_bytes(),
        )?;

        let content = page_content(image.width, image.height);
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

fn page_content(width: u32, height: u32) -> String {
    let image_width = f64::from(width.max(1));
    let image_height = f64::from(height.max(1));
    let max_width = PAGE_WIDTH - MARGIN * 2.0;
    let max_height = PAGE_HEIGHT - MARGIN * 2.0;
    let scale = (max_width / image_width).min(max_height / image_height);
    let draw_width = image_width * scale;
    let draw_height = image_height * scale;
    let x = (PAGE_WIDTH - draw_width) / 2.0;
    let y = (PAGE_HEIGHT - draw_height) / 2.0;
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
}
