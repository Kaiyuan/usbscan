// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 命名规则校验（`docs/04` §6）。
//!
//! 校验是纯函数：同一规则与上下文必得同一结果，便于在设置页实时调用。

use std::collections::BTreeSet;
use std::path::Path;

use crate::sanitize::{utf16_len, SanitizeOptions};
use crate::template::parse;
use crate::timestamp::TimeStamp;
use crate::{render_tokens, NamingContext, NamingRule, MAX_PATH_LEN};

/// 校验结果的严重程度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// 阻止保存 / 阻止开始拍摄。
    Error,
    /// 允许保存，但提示风险。
    Warning,
}

/// 一条校验结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    /// 编号，如 `E01` / `W03`。
    pub code: &'static str,
    pub message: String,
}

impl Diagnostic {
    fn error(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            code,
            message: message.into(),
        }
    }

    fn warning(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            code,
            message: message.into(),
        }
    }
}

/// 规则中出现的日期格式串。
fn date_formats(name: &str, arg: Option<&str>) -> Vec<(String, &'static str)> {
    match name {
        "date" => vec![(arg.unwrap_or("yyyyMMdd").to_string(), "date")],
        "time" => vec![(arg.unwrap_or("HHmmss").to_string(), "time")],
        "datetime" => vec![
            (arg.unwrap_or("yyyyMMdd").to_string(), "date"),
            ("HHmmss".to_string(), "time"),
        ],
        _ => Vec::new(),
    }
}

/// 检查目录模板是否合法。
fn check_folder(folder: &str, diags: &mut Vec<Diagnostic>) {
    let trimmed = folder.trim();
    if trimmed.is_empty() {
        return;
    }
    let bytes = trimmed.as_bytes();
    let absolute = trimmed.starts_with('\\')
        || trimmed.starts_with('/')
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':');
    if absolute {
        diags.push(Diagnostic::error(
            "E07",
            "目录规则必须是相对路径，不能包含盘符或绝对路径",
        ));
    }
    if trimmed.split(['\\', '/']).any(|part| part == "..") {
        diags.push(Diagnostic::error(
            "E08",
            "目录规则不能跳出输出根目录（禁止 `..`）",
        ));
    }
}

/// 执行完整校验。
///
/// `root` 用于检查完整路径长度（E06）；传 `None` 表示只做规则本身的校验。
#[must_use]
pub fn validate(
    rule: &NamingRule,
    ctx: &NamingContext<'_>,
    root: Option<&Path>,
    opts: &SanitizeOptions,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();

    if rule.template.trim().is_empty() {
        diags.push(Diagnostic::error("E01", "命名规则不能为空"));
        return diags;
    }

    let parsed = parse(&rule.template);
    if parsed.unclosed {
        diags.push(Diagnostic::error("E02", "变量缺少右花括号 `}`"));
        return diags;
    }

    let mut unknown = BTreeSet::new();
    for var in &parsed.variables {
        if !crate::template::is_known_variable(&var.name) {
            unknown.insert(var.name.clone());
        }
    }
    for name in unknown {
        diags.push(Diagnostic::error(
            "E03",
            format!("未知变量 `{{{name}}}`，可用变量见帮助"),
        ));
    }

    check_folder(&rule.folder, &mut diags);

    // W01：{seq} 后紧跟数字。
    let bytes = rule.template.as_bytes();
    for var in &parsed.variables {
        if var.name == "seq" && bytes.get(var.end).is_some_and(u8::is_ascii_digit) {
            diags.push(Diagnostic::warning(
                "W01",
                "建议在 `{seq}` 后加分隔符，否则序号会与后续数字粘连",
            ));
        }
    }

    // W02：同时使用 seq 与 page。
    let has = |name: &str| parsed.variables.iter().any(|v| v.name == name);
    if has("seq") && has("page") {
        diags.push(Diagnostic::warning(
            "W02",
            "同时使用 `{seq}` 与 `{page}` 可能产生相同结果，建议只保留一个",
        ));
    }

    // W03：日期格式串中的未知占位符。
    let mut seen_formats = BTreeSet::new();
    for var in &parsed.variables {
        for (fmt, _) in date_formats(&var.name, var.arg.as_deref()) {
            if !seen_formats.insert(fmt.clone()) {
                continue;
            }
            let unknown = TimeStamp::unknown_placeholders(&fmt);
            if !unknown.is_empty() {
                diags.push(Diagnostic::warning(
                    "W03",
                    format!("未知日期格式符 `{}`，将按字面输出", unknown.join("`, `")),
                ));
            }
        }
    }

    // W04：没有任何变量。
    if parsed.variables.is_empty() {
        diags.push(Diagnostic::warning(
            "W04",
            "固定文件名会导致重名，已自动启用“追加序号”",
        ));
    }

    // W06：不可预测的文件名。
    if has("rand") || has("uuid") {
        diags.push(Diagnostic::warning("W06", "文件名不可预测，不利于人工检索"));
    }

    // E04：消毒关闭时出现非法字符。
    if has_illegal(&rule.template) && !opts.sanitize {
        diags.push(Diagnostic::error("E04", "规则包含非法字符"));
    }

    // 渲染一次以检查长度与预览稳定性。
    let has_errors = diags.iter().any(|d| d.severity == Severity::Error);
    if has_errors {
        return diags;
    }

    match render_tokens(&parsed.tokens, ctx) {
        Ok(raw) => {
            if utf16_len(&raw) > opts.max_len {
                diags.push(Diagnostic::error(
                    "E05",
                    format!("文件名过长（最大 {} 字符）", opts.max_len),
                ));
            }
            if let Some(root) = root {
                let folder = crate::render_folder_inner(rule, ctx, opts).unwrap_or_default();
                let candidate = root
                    .join(&folder)
                    .join(format!("{}.jpg", crate::sanitize::sanitize(&raw, opts)));
                if utf16_len(&candidate.to_string_lossy()) > MAX_PATH_LEN {
                    diags.push(Diagnostic::error("E06", "完整路径过长，请缩短目录规则"));
                }
            }
            check_preview_stability(rule, ctx, opts, &mut diags);
        }
        Err(err) => diags.push(Diagnostic::error("E05", err.to_string())),
    }

    diags
}

/// W05：三次预览完全相同。
fn check_preview_stability(
    rule: &NamingRule,
    ctx: &NamingContext<'_>,
    opts: &SanitizeOptions,
    diags: &mut Vec<Diagnostic>,
) {
    let mut names = Vec::with_capacity(3);
    for offset in 0..3 {
        let mut sample = *ctx;
        sample.seq = ctx.seq.saturating_add(offset);
        let Ok(parsed) = crate::template::parse_strict(&rule.template) else {
            return;
        };
        let Ok(raw) = render_tokens(&parsed.tokens, &sample) else {
            return;
        };
        names.push(crate::sanitize::sanitize(&raw, opts));
    }
    if names[0] == names[1] && names[1] == names[2] {
        diags.push(Diagnostic::warning(
            "W05",
            "三个预览样例完全相同，该规则无法区分不同拍摄",
        ));
    }
}

fn has_illegal(template: &str) -> bool {
    template
        .chars()
        .any(|c| matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
}

/// 供 UI 判断是否允许保存。
#[must_use]
pub fn has_errors(diags: &[Diagnostic]) -> bool {
    diags.iter().any(|d| d.severity == Severity::Error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testsupport::ctx;
    use crate::DEFAULT_MAX_LEN;

    fn rule(template: &str, folder: &str) -> NamingRule {
        NamingRule {
            template: template.to_string(),
            folder: folder.to_string(),
        }
    }

    fn codes(diags: &[Diagnostic]) -> Vec<&'static str> {
        diags.iter().map(|d| d.code).collect()
    }

    #[test]
    fn empty_template_is_error() {
        let d = validate(&rule("  ", ""), &ctx(), None, &SanitizeOptions::default());
        assert_eq!(codes(&d), vec!["E01"]);
    }

    #[test]
    fn unclosed_brace_is_error() {
        let d = validate(
            &rule("扫描_{date", ""),
            &ctx(),
            None,
            &SanitizeOptions::default(),
        );
        assert_eq!(codes(&d), vec!["E02"]);
    }

    #[test]
    fn unknown_variable_is_error() {
        let d = validate(
            &rule("{seq3}", ""),
            &ctx(),
            None,
            &SanitizeOptions::default(),
        );
        assert!(codes(&d).contains(&"E03"));
    }

    #[test]
    fn absolute_folder_is_error() {
        for folder in ["C:\\扫描", "\\\\server\\share", "/tmp", "\\root"] {
            let d = validate(
                &rule("{seq}", folder),
                &ctx(),
                None,
                &SanitizeOptions::default(),
            );
            assert!(codes(&d).contains(&"E07"), "folder = {folder}");
        }
    }

    #[test]
    fn parent_folder_is_error() {
        for folder in ["..", "../扫描", "a\\..\\b"] {
            let d = validate(
                &rule("{seq}", folder),
                &ctx(),
                None,
                &SanitizeOptions::default(),
            );
            assert!(codes(&d).contains(&"E08"), "folder = {folder}");
        }
    }

    #[test]
    fn seq_followed_by_digit_warns() {
        let d = validate(
            &rule("扫描_{seq}1", ""),
            &ctx(),
            None,
            &SanitizeOptions::default(),
        );
        assert!(codes(&d).contains(&"W01"));
    }

    #[test]
    fn seq_and_page_warn() {
        let d = validate(
            &rule("{doc}_{seq}_{page}", ""),
            &ctx(),
            None,
            &SanitizeOptions::default(),
        );
        assert!(codes(&d).contains(&"W02"));
    }

    #[test]
    fn unknown_date_placeholder_warns() {
        let d = validate(
            &rule("{date:yyyyzz}", ""),
            &ctx(),
            None,
            &SanitizeOptions::default(),
        );
        assert!(codes(&d).contains(&"W03"));
    }

    #[test]
    fn fixed_name_warns_and_stable_preview_warns() {
        let d = validate(
            &rule("固定名", ""),
            &ctx(),
            None,
            &SanitizeOptions::default(),
        );
        assert!(codes(&d).contains(&"W04"));
        assert!(codes(&d).contains(&"W05"));
    }

    #[test]
    fn random_variables_warn() {
        let d = validate(
            &rule("{rand:4}", ""),
            &ctx(),
            None,
            &SanitizeOptions::default(),
        );
        assert!(codes(&d).contains(&"W06"));
    }

    #[test]
    fn good_rule_has_no_errors() {
        let d = validate(
            &rule(
                "扫描_{date:yyyyMMdd}_{time:HHmmss}_{seq:3}",
                "{date:yyyy}/{date:MM}/{date:dd}",
            ),
            &ctx(),
            None,
            &SanitizeOptions::default(),
        );
        assert!(!has_errors(&d), "{d:?}");
    }

    #[test]
    fn path_length_error_detected() {
        let long_root = "C:\\".to_string() + &"很长的目录".repeat(50);
        let d = validate(
            &rule("{date:yyyyMMdd}_{time:HHmmss}_{seq}", ""),
            &ctx(),
            Some(Path::new(&long_root)),
            &SanitizeOptions::default(),
        );
        assert!(codes(&d).contains(&"E06"));
    }

    #[test]
    fn default_rule_is_within_limits() {
        assert_eq!(DEFAULT_MAX_LEN, 200);
    }
}
