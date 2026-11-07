// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 模板解析：把规则文本切成字面量与变量。

use crate::error::NamingError;

/// 模板中的一个片段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// 普通文本（已处理 `{{` / `}}` 转义）。
    Literal(String),
    /// 变量及其参数，如 `{seq:3}` 中的 `seq` 与 `3`。
    Variable { name: String, arg: Option<String> },
}

/// 解析结果，附带校验所需的元信息。
#[derive(Debug, Clone, Default)]
pub struct Parsed {
    pub tokens: Vec<Token>,
    /// 出现顺序与位置：变量名、参数、在原文中的字节结束位置。
    pub variables: Vec<VariableRef>,
    /// 是否存在未闭合的 `{`。
    pub unclosed: bool,
}

#[derive(Debug, Clone)]
pub struct VariableRef {
    pub name: String,
    pub arg: Option<String>,
    /// 该变量在模板原文中结束的字节位置（用于检测紧邻字符）。
    pub end: usize,
}

/// 解析模板。
///
/// 非法语法不在这里返回错误，而是记录到 [`Parsed`] 中，
/// 这样校验器可以一次报告全部问题。
#[must_use]
pub fn parse(input: &str) -> Parsed {
    let mut parsed = Parsed::default();
    let chars: Vec<(usize, char)> = input.char_indices().collect();
    let mut literal = String::new();
    let mut i = 0;

    while i < chars.len() {
        let (byte_pos, ch) = chars[i];
        match ch {
            '{' => {
                if i + 1 < chars.len() && chars[i + 1].1 == '{' {
                    literal.push('{');
                    i += 2;
                    continue;
                }
                // 找配对的 '}'；遇到嵌套 '{' 视为语法错误，直接吞到末尾。
                let mut j = i + 1;
                while j < chars.len() && chars[j].1 != '}' {
                    if chars[j].1 == '{' {
                        break;
                    }
                    j += 1;
                }
                if j >= chars.len() || chars[j].1 != '}' {
                    parsed.unclosed = true;
                    literal.push_str(&input[byte_pos..]);
                    break;
                }

                if !literal.is_empty() {
                    parsed
                        .tokens
                        .push(Token::Literal(std::mem::take(&mut literal)));
                }
                let end_byte = chars[j].0 + '}'.len_utf8();
                let inner: String = chars[i + 1..j].iter().map(|(_, c)| *c).collect();
                let (name, arg) = match inner.split_once(':') {
                    Some((name, arg)) => (name.to_string(), Some(arg.to_string())),
                    None => (inner, None),
                };
                parsed.variables.push(VariableRef {
                    name: name.clone(),
                    arg: arg.clone(),
                    end: end_byte,
                });
                parsed.tokens.push(Token::Variable { name, arg });
                i = j + 1;
            }
            '}' => {
                if i + 1 < chars.len() && chars[i + 1].1 == '}' {
                    literal.push('}');
                    i += 2;
                } else {
                    // 单个 '}' 按字面量处理。
                    literal.push('}');
                    i += 1;
                }
            }
            _ => {
                literal.push(ch);
                i += 1;
            }
        }
    }

    if !literal.is_empty() {
        parsed.tokens.push(Token::Literal(literal));
    }
    parsed
}

/// 变量名是否在总表中已知。
#[must_use]
pub fn is_known_variable(name: &str) -> bool {
    matches!(
        name,
        "date"
            | "time"
            | "datetime"
            | "device"
            | "doc"
            | "seq"
            | "page"
            | "pages"
            | "barcode"
            | "preset"
            | "width"
            | "height"
            | "dpi"
            | "user"
            | "text"
            | "counter"
            | "rand"
            | "uuid"
    )
}

/// 解析模板，遇到未知变量或未闭合花括号时返回错误。
pub fn parse_strict(input: &str) -> Result<Parsed, NamingError> {
    let parsed = parse(input);
    if parsed.unclosed {
        return Err(NamingError::UnclosedBrace);
    }
    if let Some(var) = parsed
        .variables
        .iter()
        .find(|v| !is_known_variable(&v.name))
    {
        return Err(NamingError::UnknownVariable(var.name.clone()));
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_and_variables() {
        let p = parse("扫描_{date:yyyyMMdd}_{seq:3}");
        assert!(!p.unclosed);
        assert_eq!(p.variables.len(), 2);
        assert_eq!(p.variables[0].name, "date");
        assert_eq!(p.variables[0].arg.as_deref(), Some("yyyyMMdd"));
        assert_eq!(p.variables[1].name, "seq");
        assert_eq!(p.tokens.len(), 4);
    }

    #[test]
    fn escaped_braces() {
        let p = parse("{{date}}_{{x}}");
        assert!(p.variables.is_empty());
        assert_eq!(p.tokens, vec![Token::Literal("{date}_{x}".to_string())]);
    }

    #[test]
    fn unclosed_brace_detected() {
        let p = parse("扫描_{date");
        assert!(p.unclosed);
    }

    #[test]
    fn lone_closing_brace_is_literal() {
        let p = parse("a}b");
        assert_eq!(p.tokens, vec![Token::Literal("a}b".to_string())]);
    }

    #[test]
    fn strict_rejects_unknown() {
        assert!(matches!(
            parse_strict("{seq3}"),
            Err(NamingError::UnknownVariable(_))
        ));
        assert!(matches!(
            parse_strict("{seq"),
            Err(NamingError::UnclosedBrace)
        ));
    }
}
