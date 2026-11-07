// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 渲染结果的字符消毒（`docs/04` §5）。

/// 消毒选项。
#[derive(Debug, Clone, Copy)]
pub struct SanitizeOptions {
    /// 是否执行消毒。关闭时仅做长度截断（非法字符会导致落盘失败，应由校验器拦下）。
    pub sanitize: bool,
    /// 连续下划线是否压缩为单个。
    pub collapse_underscores: bool,
    /// 单个文件名最大长度，按 UTF-16 码元计。
    pub max_len: usize,
}

impl Default for SanitizeOptions {
    fn default() -> Self {
        Self {
            sanitize: true,
            collapse_underscores: true,
            max_len: 200,
        }
    }
}

/// Windows 文件名中的非法字符。
const ILLEGAL: [char; 9] = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];

/// 保留名（不含扩展名的部分，比较时忽略大小写）。
const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// 对渲染结果做消毒。
#[must_use]
pub fn sanitize(input: &str, opts: &SanitizeOptions) -> String {
    if !opts.sanitize {
        return truncate_utf16(input, opts.max_len);
    }

    // 1. 非法字符替换为下划线。
    let mut s: String = input
        .chars()
        .map(|c| if ILLEGAL.contains(&c) { '_' } else { c })
        .collect();

    // 2. 删除控制字符。
    s.retain(|c| !c.is_control());

    // 3/4. 去掉首尾空格与句点。
    let trimmed = s.trim_matches(|c| c == ' ' || c == '.');
    let mut s = trimmed.to_string();

    // 5. 压缩连续下划线。
    if opts.collapse_underscores {
        while s.contains("__") {
            s = s.replace("__", "_");
        }
    }

    // 6. 空结果兜底。
    if s.is_empty() {
        s = "unnamed".to_string();
    }

    // 7. 保留名处理：比较第一个句点之前的部分。
    let base = s.split('.').next().unwrap_or(&s);
    if is_reserved(base) {
        // 在基础名之后插入下划线：CON → CON_，CON.txt → CON_.txt。
        let idx = s.find('.').unwrap_or(s.len());
        s.insert(idx, '_');
    }

    // 8. 按 UTF-16 码元截断。
    truncate_utf16(&s, opts.max_len)
}

/// 是否为 Windows 保留设备名。
#[must_use]
pub fn is_reserved(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    if RESERVED.contains(&upper.as_str()) {
        return true;
    }
    matches!(upper.as_str(), "CLOCK$" | "CONIN$" | "CONOUT$")
}

/// 按 UTF-16 码元数截断，保证不会切断代理对。
#[must_use]
pub fn truncate_utf16(input: &str, max_units: usize) -> String {
    if input.encode_utf16().count() <= max_units {
        return input.to_string();
    }
    let mut units = 0;
    let mut out = String::new();
    for ch in input.chars() {
        let len = ch.len_utf16();
        if units + len > max_units {
            break;
        }
        units += len;
        out.push(ch);
    }
    out
}

/// 统计 UTF-16 码元数。
#[must_use]
pub fn utf16_len(input: &str) -> usize {
    input.encode_utf16().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(input: &str) -> String {
        sanitize(input, &SanitizeOptions::default())
    }

    #[test]
    fn illegal_characters() {
        assert_eq!(clean(r#"a\b/c:d*e?f"g<h>i|j"#), "a_b_c_d_e_f_g_h_i_j");
    }

    #[test]
    fn control_characters_removed() {
        assert_eq!(clean("a\u{0}b\u{1f}c\u{7f}d"), "abcd");
    }

    #[test]
    fn trims_dots_and_spaces() {
        assert_eq!(clean("  ..name..  "), "name");
    }

    #[test]
    fn collapses_underscores() {
        assert_eq!(clean("a___b"), "a_b");
        let opts = SanitizeOptions {
            collapse_underscores: false,
            ..SanitizeOptions::default()
        };
        assert_eq!(sanitize("a___b", &opts), "a___b");
    }

    #[test]
    fn empty_becomes_unnamed() {
        assert_eq!(clean("   "), "unnamed");
        assert_eq!(clean("..."), "unnamed");
    }

    #[test]
    fn reserved_names_get_suffix() {
        assert_eq!(clean("CON"), "CON_");
        assert_eq!(clean("con.txt"), "con_.txt");
        assert_eq!(clean("COM1"), "COM1_");
        assert_eq!(clean("CLOCK$"), "CLOCK$_");
        assert_eq!(clean("CONSOLE"), "CONSOLE");
    }

    #[test]
    fn chinese_and_emoji_allowed() {
        assert_eq!(clean("扫描_合同📄"), "扫描_合同📄");
    }

    #[test]
    fn truncates_by_utf16_units() {
        let s = "汉".repeat(250);
        let out = clean(&s);
        assert_eq!(out.encode_utf16().count(), 200);
        // emoji 占两个码元，不能截半个。
        let e = "😀".repeat(150);
        let out = clean(&e);
        assert_eq!(out.encode_utf16().count(), 200);
    }
}
