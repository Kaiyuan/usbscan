// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 轻量 DOCX 导出。
//!
//! 生成一个只包含图片页的最小 OOXML 文档。ZIP 使用 stored（不压缩）方式，
//! 避免引入第三方压缩库；图片仍以原始 JPEG 嵌入，不重新编码。

#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]

use std::fmt::Write as _;
use std::io::{self, Write};

/// 一页待导出的图片。
pub struct DocxPage<'a> {
    pub jpeg: &'a [u8],
    pub width: u32,
    pub height: u32,
}

const EMU_PER_PIXEL: u64 = 9_525; // 96 DPI
const CONTENT_WIDTH_EMU: u64 = 5_943_600; // 6.5 in
const CONTENT_HEIGHT_EMU: u64 = 8_229_600; // 9 in

/// 生成 DOCX 并写入 `out`。
pub fn write_docx<W: Write>(out: W, pages: &[DocxPage<'_>]) -> io::Result<()> {
    let mut files: Vec<(String, Vec<u8>)> = Vec::with_capacity(pages.len() + 4);
    files.push((
        "[Content_Types].xml".to_string(),
        content_types().into_bytes(),
    ));
    files.push(("_rels/.rels".to_string(), ROOT_RELS.as_bytes().to_vec()));

    let mut relationships = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    for index in 1..=pages.len() {
        let _ = write!(
            relationships,
            "<Relationship Id=\"rIdImg{index}\" \
             Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" \
             Target=\"media/image{index}.jpg\"/>"
        );
    }
    relationships.push_str("</Relationships>");
    files.push((
        "word/_rels/document.xml.rels".to_string(),
        relationships.into_bytes(),
    ));

    let mut document = String::from(DOCUMENT_HEADER);
    for (index, page) in pages.iter().enumerate() {
        let (cx, cy) = image_extent(page.width, page.height);
        let id = index + 1;
        let _ = write!(
            document,
            "<w:p><w:pPr><w:jc w:val=\"center\"/></w:pPr><w:r><w:drawing>\
             <wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\">\
             <wp:extent cx=\"{cx}\" cy=\"{cy}\"/>\
             <wp:docPr id=\"{id}\" name=\"Page {id}\"/>\
             <a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\">\
             <pic:pic><pic:nvPicPr><pic:cNvPr id=\"{id}\" name=\"image{id}.jpg\"/><pic:cNvPicPr/></pic:nvPicPr>\
             <pic:blipFill><a:blip r:embed=\"rIdImg{id}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>\
             <pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm>\
             <a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic>\
             </a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"
        );
        if index + 1 < pages.len() {
            document.push_str("<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>");
        }
        files.push((format!("word/media/image{id}.jpg"), page.jpeg.to_vec()));
    }
    document.push_str(DOCUMENT_FOOTER);
    files.push(("word/document.xml".to_string(), document.into_bytes()));

    write_zip(out, &files)
}

fn content_types() -> String {
    String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
         <Default Extension=\"jpg\" ContentType=\"image/jpeg\"/>\
         <Override PartName=\"/word/document.xml\" \
         ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
         </Types>",
    )
}

fn image_extent(width: u32, height: u32) -> (u64, u64) {
    let width = u64::from(width.max(1));
    let height = u64::from(height.max(1));
    let native_width = width * EMU_PER_PIXEL;
    let native_height = height * EMU_PER_PIXEL;
    let scale = (CONTENT_WIDTH_EMU as f64 / native_width as f64)
        .min(CONTENT_HEIGHT_EMU as f64 / native_height as f64)
        .min(1.0);
    (
        (native_width as f64 * scale).round() as u64,
        (native_height as f64 * scale).round() as u64,
    )
}

const ROOT_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" \
Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" \
Target=\"word/document.xml\"/></Relationships>";

const DOCUMENT_HEADER: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" \
xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><w:body>";

const DOCUMENT_FOOTER: &str = "<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/>\
<w:pgMar w:top=\"1134\" w:right=\"1134\" w:bottom=\"1134\" w:left=\"1134\" \
w:header=\"709\" w:footer=\"709\" w:gutter=\"0\"/></w:sectPr></w:body></w:document>";

fn write_zip<W: Write>(mut out: W, files: &[(String, Vec<u8>)]) -> io::Result<()> {
    let mut central = Vec::new();
    let mut offset = 0u64;
    for (name, data) in files {
        let name_bytes = name.as_bytes();
        let crc = crc32(data);
        let local_offset = offset;
        let mut local = Vec::with_capacity(30 + name_bytes.len() + data.len());
        local.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        local.extend_from_slice(&20u16.to_le_bytes());
        local.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8
        local.extend_from_slice(&0u16.to_le_bytes()); // stored
        local.extend_from_slice(&0u16.to_le_bytes()); // time
        local.extend_from_slice(&0x21u16.to_le_bytes()); // 1980-01-01
        local.extend_from_slice(&crc.to_le_bytes());
        local.extend_from_slice(&(data.len() as u32).to_le_bytes());
        local.extend_from_slice(&(data.len() as u32).to_le_bytes());
        local.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(name_bytes);
        local.extend_from_slice(data);
        out.write_all(&local)?;
        offset += local.len() as u64;

        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&0x0800u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0x21u16.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u32.to_le_bytes());
        central.extend_from_slice(&(local_offset as u32).to_le_bytes());
        central.extend_from_slice(name_bytes);
    }
    let central_offset = offset;
    out.write_all(&central)?;
    out.write_all(&0x0605_4b50u32.to_le_bytes())?;
    out.write_all(&0u16.to_le_bytes())?;
    out.write_all(&0u16.to_le_bytes())?;
    out.write_all(&(files.len() as u16).to_le_bytes())?;
    out.write_all(&(files.len() as u16).to_le_bytes())?;
    out.write_all(&(central.len() as u32).to_le_bytes())?;
    out.write_all(&(central_offset as u32).to_le_bytes())?;
    out.write_all(&0u16.to_le_bytes())?;
    out.flush()
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use usbscan_imaging::encode_jpeg_bgra;

    fn sample_jpeg(width: u32, height: u32) -> Vec<u8> {
        let bgra = vec![240u8; width as usize * height as usize * 4];
        let mut jpeg = Vec::new();
        encode_jpeg_bgra(width, height, &bgra, 90, &mut jpeg).expect("jpeg");
        jpeg
    }

    fn read_zip(data: &[u8]) -> Vec<(String, Vec<u8>)> {
        let mut entries = Vec::new();
        let mut index = 0usize;
        while index + 30 <= data.len() && data[index..index + 4] == [0x50, 0x4b, 0x03, 0x04] {
            let method = u16::from_le_bytes([data[index + 8], data[index + 9]]);
            assert_eq!(method, 0);
            let crc = u32::from_le_bytes(data[index + 14..index + 18].try_into().unwrap());
            let size =
                u32::from_le_bytes(data[index + 18..index + 22].try_into().unwrap()) as usize;
            let name_len = u16::from_le_bytes([data[index + 26], data[index + 27]]) as usize;
            let extra_len = u16::from_le_bytes([data[index + 28], data[index + 29]]) as usize;
            let name =
                String::from_utf8(data[index + 30..index + 30 + name_len].to_vec()).expect("name");
            let start = index + 30 + name_len + extra_len;
            let content = data[start..start + size].to_vec();
            assert_eq!(crc32(&content), crc, "crc mismatch for {name}");
            entries.push((name, content));
            index = start + size;
        }
        entries
    }

    #[test]
    fn writes_valid_zip_and_document() {
        let first = sample_jpeg(8, 6);
        let second = sample_jpeg(4, 8);
        let pages = [
            DocxPage {
                jpeg: &first,
                width: 8,
                height: 6,
            },
            DocxPage {
                jpeg: &second,
                width: 4,
                height: 8,
            },
        ];
        let mut out = Vec::new();
        write_docx(&mut out, &pages).expect("docx");
        assert!(out.starts_with(b"PK\x03\x04"));
        let entries = read_zip(&out);
        let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
        for expected in [
            "[Content_Types].xml",
            "_rels/.rels",
            "word/document.xml",
            "word/_rels/document.xml.rels",
            "word/media/image1.jpg",
            "word/media/image2.jpg",
        ] {
            assert!(names.contains(&expected), "missing {expected}");
        }
        let document = &entries
            .iter()
            .find(|(name, _)| name == "word/document.xml")
            .expect("document")
            .1;
        let text = String::from_utf8_lossy(document);
        assert!(text.contains("rIdImg1"));
        assert!(text.contains("rIdImg2"));
        assert!(text.contains("w:br w:type=\"page\""));
        assert!(text.contains("<w:sectPr>"));
        let image1 = &entries
            .iter()
            .find(|(name, _)| name == "word/media/image1.jpg")
            .expect("image1")
            .1;
        assert_eq!(image1, &first);
    }

    #[test]
    fn empty_document_is_valid() {
        let mut out = Vec::new();
        write_docx(&mut out, &[]).expect("docx");
        let entries = read_zip(&out);
        assert!(entries.iter().any(|(name, _)| name == "word/document.xml"));
        assert_eq!(&out[out.len() - 2..], &[0, 0]);
        assert!(out.windows(4).any(|window| window == b"PK\x05\x06"));
    }
}
