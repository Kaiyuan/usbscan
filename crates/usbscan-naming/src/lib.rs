// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! `usbscan` 的文件命名规则引擎。
//!
//! 本 crate 是"边拍边存"的契约实现，对应 [`docs/04-命名规则.md`]：
//!
//! - 模板渲染：`{date:yyyyMMdd}_{seq:3}` 等；
//! - 字符消毒：非法字符、保留名、长度；
//! - 校验：E01–E08 / W01–W06；
//! - 重名处理与原子占位。
//!
//! 引擎本身没有全局状态，可安全地在任意线程调用。
//!
//! [`docs/04-命名规则.md`]: https://example.invalid/usbscan/docs/04

mod collision;
mod error;
mod sanitize;
mod template;
mod timestamp;
mod validate;

#[cfg(test)]
mod testsupport;

use std::path::{Path, PathBuf};

pub use collision::{reserve, Collision, ReservedFile};
pub use error::NamingError;
pub use sanitize::{is_reserved, sanitize, truncate_utf16, utf16_len, SanitizeOptions};
pub use timestamp::TimeStamp;
pub use validate::{has_errors, validate, Diagnostic, Severity};

pub use template::{is_known_variable, parse, Token};

/// 单个文件名的默认长度上限（UTF-16 码元）。
pub const DEFAULT_MAX_LEN: usize = 200;

/// 完整路径的目标长度上限（字符）。
pub const MAX_PATH_LEN: usize = 250;

/// 一条命名规则。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamingRule {
    /// 主文件名模板，不含扩展名。
    pub template: String,
    /// 目录模板，相对输出根目录；空串表示不分层。
    pub folder: String,
}

impl Default for NamingRule {
    fn default() -> Self {
        Self {
            template: "扫描_{date:yyyyMMdd}_{time:HHmmss}_{seq:3}".to_string(),
            folder: "{date:yyyy}/{date:MM}/{date:dd}".to_string(),
        }
    }
}

/// 渲染所需的上下文。
///
/// 所有可选字段缺省时渲染为空串（`doc` 例外，缺省为 `未命名`）。
#[derive(Debug, Clone, Copy)]
pub struct NamingContext<'a> {
    pub now: TimeStamp,
    pub device: Option<&'a str>,
    pub doc: Option<&'a str>,
    pub barcode: Option<&'a str>,
    pub preset: Option<&'a str>,
    pub text: Option<&'a str>,
    pub seq: u64,
    pub page: Option<u32>,
    pub total: Option<u32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub dpi: Option<u32>,
    /// 持久计数器（跨会话）。
    pub counter: u64,
    /// 覆盖 `{user}`；缺省读取 `USERNAME` 环境变量。
    pub user: Option<&'a str>,
}

impl Default for NamingContext<'_> {
    fn default() -> Self {
        Self {
            now: TimeStamp::now(),
            device: None,
            doc: None,
            barcode: None,
            preset: None,
            text: None,
            seq: 1,
            page: None,
            total: None,
            width: None,
            height: None,
            dpi: None,
            counter: 0,
            user: None,
        }
    }
}

/// 一次渲染的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    /// 主文件名（不含扩展名，已消毒）。
    pub file_name: String,
    /// 相对输出根目录的文件夹路径。
    pub folder: PathBuf,
}

impl Rendered {
    /// 拼接完整路径。
    #[must_use]
    pub fn path(&self, root: &Path, extension: &str) -> PathBuf {
        let extension = extension.trim_start_matches('.');
        let file = if extension.is_empty() {
            self.file_name.clone()
        } else {
            format!("{}.{}", self.file_name, extension)
        };
        root.join(&self.folder).join(file)
    }
}

/// 命名引擎。
#[derive(Debug, Clone, Copy)]
pub struct NamingEngine {
    opts: SanitizeOptions,
}

impl Default for NamingEngine {
    fn default() -> Self {
        Self::new(SanitizeOptions::default())
    }
}

impl NamingEngine {
    #[must_use]
    pub const fn new(opts: SanitizeOptions) -> Self {
        Self { opts }
    }

    #[must_use]
    pub const fn options(&self) -> SanitizeOptions {
        self.opts
    }

    /// 校验规则。`root` 用于检查完整路径长度。
    #[must_use]
    pub fn validate(
        &self,
        rule: &NamingRule,
        ctx: &NamingContext<'_>,
        root: Option<&Path>,
    ) -> Vec<Diagnostic> {
        validate(rule, ctx, root, &self.opts)
    }

    /// 渲染一次文件名与目录。
    pub fn render(
        &self,
        rule: &NamingRule,
        ctx: &NamingContext<'_>,
    ) -> Result<Rendered, NamingError> {
        if rule.template.trim().is_empty() {
            return Err(NamingError::EmptyTemplate);
        }
        let parsed = template::parse_strict(&rule.template)?;

        let raw = render_tokens(&parsed.tokens, ctx)?;
        if utf16_len(&raw) > self.opts.max_len {
            return Err(NamingError::NameTooLong);
        }
        let file_name = sanitize::sanitize(&raw, &self.opts);
        let folder = render_folder_inner(rule, ctx, &self.opts)?;
        Ok(Rendered { file_name, folder })
    }

    /// 预览 `n` 个连续序号的文件名，用于设置页。
    #[must_use]
    pub fn preview(&self, rule: &NamingRule, ctx: &NamingContext<'_>, n: usize) -> Vec<String> {
        (0..n)
            .filter_map(|offset| {
                let mut sample = *ctx;
                sample.seq = ctx.seq.saturating_add(offset as u64);
                self.render(rule, &sample).ok().map(|r| {
                    let ext = r.path(Path::new(""), "jpg");
                    ext.file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default()
                })
            })
            .collect()
    }

    /// 原子占位一个文件名，处理重名。返回 `None` 表示按"跳过"策略未保存。
    pub fn reserve(
        &self,
        dir: &Path,
        stem: &str,
        extension: &str,
        policy: Collision,
    ) -> Result<Option<ReservedFile>, NamingError> {
        collision::reserve(dir, stem, extension, policy)
    }
}

/// 渲染 token 序列（内部使用，也供校验器复用）。
pub(crate) fn render_tokens(
    tokens: &[Token],
    ctx: &NamingContext<'_>,
) -> Result<String, NamingError> {
    let mut out = String::with_capacity(48);
    for token in tokens {
        match token {
            Token::Literal(text) => out.push_str(text),
            Token::Variable { name, arg } => {
                out.push_str(&resolve_variable(name, arg.as_deref(), ctx)?);
            }
        }
    }
    Ok(out)
}

/// 渲染目录模板为相对路径。
pub(crate) fn render_folder_inner(
    rule: &NamingRule,
    ctx: &NamingContext<'_>,
    opts: &SanitizeOptions,
) -> Result<PathBuf, NamingError> {
    let trimmed = rule.folder.trim();
    if trimmed.is_empty() {
        return Ok(PathBuf::new());
    }
    let bytes = trimmed.as_bytes();
    if trimmed.starts_with('\\')
        || trimmed.starts_with('/')
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
    {
        return Err(NamingError::AbsoluteFolder);
    }

    let mut path = PathBuf::new();
    for part in trimmed.split(['\\', '/']) {
        if part == ".." {
            return Err(NamingError::FolderEscape);
        }
        let parsed = template::parse_strict(part)?;
        let raw = render_tokens(&parsed.tokens, ctx)?;
        let component = sanitize::sanitize(&raw, opts);
        if component != "unnamed" || !raw.is_empty() {
            path.push(component);
        }
    }
    Ok(path)
}

fn resolve_variable(
    name: &str,
    arg: Option<&str>,
    ctx: &NamingContext<'_>,
) -> Result<String, NamingError> {
    let value = match name {
        "date" => ctx.now.format(arg.unwrap_or("yyyyMMdd")),
        "time" => ctx.now.format(arg.unwrap_or("HHmmss")),
        "datetime" => format!(
            "{}_{}",
            ctx.now.format(arg.unwrap_or("yyyyMMdd")),
            ctx.now.format("HHmmss")
        ),
        "device" => ctx.device.unwrap_or("").to_string(),
        "doc" => ctx.doc.unwrap_or("未命名").to_string(),
        "seq" => pad(ctx.seq, digits(arg, 3)),
        "page" => pad(u64::from(ctx.page.unwrap_or(0)), digits(arg, 1)),
        "pages" => ctx.total.map(|v| v.to_string()).unwrap_or_default(),
        "barcode" => ctx.barcode.unwrap_or("").to_string(),
        "preset" => ctx.preset.unwrap_or("").to_string(),
        "width" => ctx.width.map(|v| v.to_string()).unwrap_or_default(),
        "height" => ctx.height.map(|v| v.to_string()).unwrap_or_default(),
        "dpi" => ctx.dpi.map(|v| v.to_string()).unwrap_or_default(),
        "user" => ctx
            .user
            .map(str::to_string)
            .or_else(|| std::env::var("USERNAME").ok())
            .unwrap_or_default(),
        "text" => ctx.text.unwrap_or("").to_string(),
        "counter" => pad(ctx.counter, digits(arg, 6)),
        "rand" => random_alnum(digits(arg, 6)),
        "uuid" => format!("{:08x}", random_u32()),
        _ => return Err(NamingError::UnknownVariable(name.to_string())),
    };
    Ok(value)
}

/// 解析位数参数。
fn digits(arg: Option<&str>, default: usize) -> usize {
    arg.and_then(|a| a.parse::<usize>().ok()).unwrap_or(default)
}

/// 左侧补零；数值超过位数时不截断。位数上限 9（`docs/04` §4.3）。
fn pad(value: u64, width: usize) -> String {
    let width = width.min(9);
    if width == 0 {
        value.to_string()
    } else {
        format!("{value:0width$}")
    }
}

const ALPHABET: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

fn random_alnum(len: usize) -> String {
    let len = len.clamp(1, 64);
    (0..len)
        .map(|_| ALPHABET[(random_u32() % 36) as usize] as char)
        .collect()
}

/// 无外部依赖的伪随机数，已足够用于文件名区分。
fn random_u32() -> u32 {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::OnceLock;
    use std::time::{SystemTime, UNIX_EPOCH};

    static STATE: OnceLock<AtomicU64> = OnceLock::new();

    let state = STATE.get_or_init(|| {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        let seed = nanos ^ (u64::from(std::process::id()) << 32) ^ 0x9E37_79B9_7F4A_7C15;
        AtomicU64::new(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    });

    loop {
        let current = state.load(Ordering::Relaxed);
        let next = xorshift64(current);
        if state
            .compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return (next >> 32) as u32;
        }
    }
}

fn xorshift64(mut x: u64) -> u64 {
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> NamingContext<'static> {
        NamingContext {
            now: TimeStamp::new(2026, 11, 7, 14, 30, 12, 45, 6),
            device: Some("Logitech C920"),
            doc: Some("报销单"),
            barcode: Some("SN20261107001"),
            preset: Some("票据"),
            text: Some("合同"),
            seq: 1,
            page: Some(3),
            total: Some(12),
            width: Some(3264),
            height: Some(2448),
            dpi: Some(300),
            counter: 123,
            user: Some("kaiyuan"),
        }
    }

    fn render(template: &str) -> String {
        let engine = NamingEngine::default();
        let rule = NamingRule {
            template: template.to_string(),
            folder: String::new(),
        };
        engine.render(&rule, &ctx()).expect("render").file_name
    }

    #[test]
    fn default_rule_renders_documented_example() {
        let engine = NamingEngine::default();
        let rule = NamingRule::default();
        let rendered = engine.render(&rule, &ctx()).expect("render");
        assert_eq!(rendered.file_name, "扫描_20261107_143012_001");
        assert_eq!(rendered.folder, PathBuf::from("2026").join("11").join("07"));
        assert_eq!(
            rendered.path(Path::new("D:\\扫描"), "jpg"),
            PathBuf::from("D:\\扫描\\2026\\11\\07\\扫描_20261107_143012_001.jpg")
        );
    }

    #[test]
    fn every_variable_renders() {
        let cases = [
            ("{date}", "20261107"),
            ("{date:yyyy-MM-dd}", "2026-11-07"),
            ("{time}", "143012"),
            ("{datetime}", "20261107_143012"),
            ("{device}", "Logitech C920"),
            ("{doc}", "报销单"),
            ("{seq}", "001"),
            ("{seq:4}", "0001"),
            ("{seq:0}", "1"),
            ("{page}", "3"),
            ("{page:2}", "03"),
            ("{pages}", "12"),
            ("{barcode}", "SN20261107001"),
            ("{preset}", "票据"),
            ("{width}", "3264"),
            ("{height}", "2448"),
            ("{dpi}", "300"),
            ("{user}", "kaiyuan"),
            ("{text}", "合同"),
            ("{counter}", "000123"),
            ("{counter:3}", "123"),
        ];
        for (template, expected) in cases {
            assert_eq!(render(template), expected, "template = {template}");
        }
    }

    #[test]
    fn defaults_for_missing_values() {
        let engine = NamingEngine::default();
        let rule = NamingRule {
            template: "{doc}".to_string(),
            folder: String::new(),
        };
        let empty = NamingContext {
            doc: None,
            user: Some(""),
            ..NamingContext::default()
        };
        assert_eq!(engine.render(&rule, &empty).unwrap().file_name, "未命名");
    }

    #[test]
    fn illegal_characters_in_variable_are_sanitized() {
        assert_eq!(render("{device}"), "Logitech C920");
        let mut c = ctx();
        c.device = Some("USB:Camera/Front");
        let engine = NamingEngine::default();
        let rule = NamingRule {
            template: "{device}".to_string(),
            folder: String::new(),
        };
        assert_eq!(
            engine.render(&rule, &c).unwrap().file_name,
            "USB_Camera_Front"
        );
    }

    #[test]
    fn escaped_braces_are_literal() {
        assert_eq!(render("{{seq}}"), "{seq}");
    }

    #[test]
    fn seq_overflow_widens_instead_of_truncating() {
        let mut c = ctx();
        c.seq = 1000;
        let engine = NamingEngine::default();
        let rule = NamingRule {
            template: "{seq:3}".to_string(),
            folder: String::new(),
        };
        assert_eq!(engine.render(&rule, &c).unwrap().file_name, "1000");
    }

    #[test]
    fn folder_with_device() {
        let engine = NamingEngine::default();
        let rule = NamingRule {
            template: "{seq}".to_string(),
            folder: "{device}/{date:yyyyMM}".to_string(),
        };
        let mut c = ctx();
        c.device = Some("Logitech C920");
        assert_eq!(
            engine.render(&rule, &c).unwrap().folder,
            PathBuf::from("Logitech C920").join("202611")
        );
    }

    #[test]
    fn preview_lists_consecutive_names() {
        let engine = NamingEngine::default();
        let rule = NamingRule {
            template: "扫描_{seq:3}".to_string(),
            folder: String::new(),
        };
        let names = engine.preview(&rule, &ctx(), 3);
        assert_eq!(names, vec!["扫描_001.jpg", "扫描_002.jpg", "扫描_003.jpg"]);
    }

    #[test]
    fn empty_template_rejected() {
        let engine = NamingEngine::default();
        let rule = NamingRule {
            template: "   ".to_string(),
            folder: String::new(),
        };
        assert!(matches!(
            engine.render(&rule, &ctx()),
            Err(NamingError::EmptyTemplate)
        ));
    }

    #[test]
    fn absolute_folder_rejected() {
        let engine = NamingEngine::default();
        let rule = NamingRule {
            template: "{seq}".to_string(),
            folder: "C:\\扫描".to_string(),
        };
        assert!(matches!(
            engine.render(&rule, &ctx()),
            Err(NamingError::AbsoluteFolder)
        ));
    }

    #[test]
    fn folder_escape_rejected() {
        let engine = NamingEngine::default();
        let rule = NamingRule {
            template: "{seq}".to_string(),
            folder: "a\\..\\b".to_string(),
        };
        assert!(matches!(
            engine.render(&rule, &ctx()),
            Err(NamingError::FolderEscape)
        ));
    }

    #[test]
    fn random_values_are_well_formed() {
        assert_eq!(render("{rand:10}").len(), 10);
        assert!(render("{rand:10}")
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_'));
        let uuid = render("{uuid}");
        assert_eq!(uuid.len(), 8);
        assert!(uuid.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn timestamp_iso() {
        assert_eq!(ctx().now.iso(), "2026-11-07 14:30:12");
    }
}
