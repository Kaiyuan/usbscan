// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 最小 JSON 读写器。
//!
//! `session.json` 完全由本程序生成，只需要对象、数组、字符串、整数、
//! 布尔与 `null`。这里不引入第三方依赖，避免为零依赖目标增加构建负担；
//! 解析失败时调用方会归档旧清单并从磁盘重建会话。

use std::fmt::Write as _;

/// 一个 JSON 值。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(i64),
    String(String),
    Array(Vec<Self>),
    Object(Vec<(String, Self)>),
}

impl Json {
    /// 按字段名取对象成员。
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Object(fields) => fields
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    /// 取字符串值。
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    /// 取整数，负数与越界值返回 `None`。
    #[must_use]
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Number(value) => u64::try_from(*value).ok(),
            _ => None,
        }
    }

    /// 取数组。
    #[must_use]
    pub fn as_array(&self) -> Option<&[Self]> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }
}

/// 解析 JSON 文本。
pub fn parse(text: &str) -> Result<Json, String> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        pos: 0,
    };
    let value = parser.value()?;
    parser.skip_whitespace();
    if parser.pos != parser.bytes.len() {
        return Err(format!("JSON 末尾存在多余内容（字节 {}）", parser.pos));
    }
    Ok(value)
}

/// 把字符串写成 JSON 字符串字面量（含两侧引号）。
#[must_use]
pub fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            ch if (ch as u32) < 0x20 => {
                // 控制字符必须转义；写入失败不会发生。
                let _ = write!(out, "\\u{:04x}", ch as u32);
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn value(&mut self) -> Result<Json, String> {
        self.skip_whitespace();
        let byte = *self
            .bytes
            .get(self.pos)
            .ok_or_else(|| "JSON 意外结束".to_string())?;
        match byte {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => self.string().map(Json::String),
            b't' => self.literal("true", Json::Bool(true)),
            b'f' => self.literal("false", Json::Bool(false)),
            b'n' => self.literal("null", Json::Null),
            b'-' | b'0'..=b'9' => self.number(),
            other => Err(format!("无法识别的 JSON 标记 0x{other:02x}")),
        }
    }

    fn object(&mut self) -> Result<Json, String> {
        self.pos += 1; // '{'
        let mut fields = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Object(fields));
        }
        loop {
            self.skip_whitespace();
            let key = self.string()?;
            self.skip_whitespace();
            self.expect(b':')?;
            let value = self.value()?;
            fields.push((key, value));
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Json::Object(fields));
                }
                _ => return Err("JSON 对象缺少逗号或右花括号".to_string()),
            }
        }
    }

    fn array(&mut self) -> Result<Json, String> {
        self.pos += 1; // '['
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Array(items));
        }
        loop {
            items.push(self.value()?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err("JSON 数组缺少逗号或右方括号".to_string()),
            }
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out = String::new();
        let mut chunk_start = self.pos;
        loop {
            let byte = *self
                .bytes
                .get(self.pos)
                .ok_or_else(|| "JSON 字符串未结束".to_string())?;
            match byte {
                b'"' => {
                    self.push_chunk(&mut out, chunk_start, self.pos)?;
                    self.pos += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.push_chunk(&mut out, chunk_start, self.pos)?;
                    self.pos += 1;
                    self.escape(&mut out)?;
                    chunk_start = self.pos;
                }
                0x00..=0x1f => return Err("JSON 字符串包含裸控制字符".to_string()),
                _ => self.pos += 1,
            }
        }
    }

    fn push_chunk(&self, out: &mut String, start: usize, end: usize) -> Result<(), String> {
        let chunk = std::str::from_utf8(&self.bytes[start..end])
            .map_err(|_| "JSON 字符串不是有效 UTF-8".to_string())?;
        out.push_str(chunk);
        Ok(())
    }

    fn escape(&mut self, out: &mut String) -> Result<(), String> {
        let byte = *self
            .bytes
            .get(self.pos)
            .ok_or_else(|| "JSON 转义序列未结束".to_string())?;
        self.pos += 1;
        match byte {
            b'"' => out.push('"'),
            b'\\' => out.push('\\'),
            b'/' => out.push('/'),
            b'b' => out.push('\u{08}'),
            b'f' => out.push('\u{0c}'),
            b'n' => out.push('\n'),
            b'r' => out.push('\r'),
            b't' => out.push('\t'),
            b'u' => {
                let ch = self.unicode_escape()?;
                out.push(ch);
            }
            other => return Err(format!("无效的 JSON 转义 0x{other:02x}")),
        }
        Ok(())
    }

    fn unicode_escape(&mut self) -> Result<char, String> {
        let first = self.hex4()?;
        if (0xD800..=0xDBFF).contains(&first) {
            // 代理对：需要紧跟 `\uXXXX` 的低代理。
            if self.bytes.get(self.pos) == Some(&b'\\')
                && self.bytes.get(self.pos + 1) == Some(&b'u')
            {
                self.pos += 2;
                let second = self.hex4()?;
                if (0xDC00..=0xDFFF).contains(&second) {
                    let combined = 0x1_0000
                        + ((u32::from(first) - 0xD800) << 10)
                        + (u32::from(second) - 0xDC00);
                    return char::from_u32(combined).ok_or_else(|| "JSON 代理对无效".to_string());
                }
            }
            return Ok('\u{FFFD}');
        }
        char::from_u32(u32::from(first)).ok_or_else(|| "JSON Unicode 转义无效".to_string())
    }

    fn hex4(&mut self) -> Result<u16, String> {
        let mut value = 0u16;
        for _ in 0..4 {
            let byte = *self
                .bytes
                .get(self.pos)
                .ok_or_else(|| "JSON Unicode 转义长度不足".to_string())?;
            let digit = match byte {
                b'0'..=b'9' => u16::from(byte - b'0'),
                b'a'..=b'f' => u16::from(byte - b'a') + 10,
                b'A'..=b'F' => u16::from(byte - b'A') + 10,
                _ => return Err("JSON Unicode 转义不是十六进制".to_string()),
            };
            value = value * 16 + digit;
            self.pos += 1;
        }
        Ok(value)
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| "JSON 数字不是有效 UTF-8".to_string())?;
        text.parse::<i64>()
            .map(Json::Number)
            .map_err(|_| format!("JSON 数字无效：{text}"))
    }

    fn literal(&mut self, name: &str, value: Json) -> Result<Json, String> {
        if self.bytes[self.pos..].starts_with(name.as_bytes()) {
            self.pos += name.len();
            Ok(value)
        } else {
            Err(format!("无法识别的 JSON 字面量（期望 {name}）"))
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), String> {
        if self.peek() == Some(byte) {
            self.pos += 1;
            Ok(())
        } else {
            Err(format!("JSON 期望 '{}'（字节 {}）", byte as char, self.pos))
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_values() {
        let text = r#"{"a": [1, -2, true, null], "b": {"c": "值"}}"#;
        let value = parse(text).expect("parse");
        assert_eq!(
            value.get("a").and_then(Json::as_array).map(<[Json]>::len),
            Some(4)
        );
        assert_eq!(
            value
                .get("b")
                .and_then(|b| b.get("c"))
                .and_then(Json::as_str),
            Some("值")
        );
    }

    #[test]
    fn quote_round_trips() {
        let text = "引号\" 反斜杠\\ 换行\n 制表\t 控制\u{1}";
        let json = quote(text);
        assert_eq!(parse(&json).expect("parse").as_str(), Some(text));
    }

    #[test]
    fn parses_unicode_escapes() {
        assert_eq!(
            parse(r#""\u4e2d\u6587""#).expect("parse").as_str(),
            Some("中文")
        );
        assert_eq!(
            parse(r#""\ud83d\ude00""#).expect("parse").as_str(),
            Some("😀")
        );
    }

    #[test]
    fn rejects_broken_input() {
        assert!(parse("{").is_err());
        assert!(parse("[1,]").is_err());
        assert!(parse(r#"{"a" 1}"#).is_err());
        assert!(parse("\"unterminated").is_err());
    }
}
