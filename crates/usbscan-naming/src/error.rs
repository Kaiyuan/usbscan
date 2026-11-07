// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 命名引擎的错误类型。

use std::fmt;
use std::io;

/// 渲染或落盘失败的原因。
#[derive(Debug)]
pub enum NamingError {
    /// 模板为空。
    EmptyTemplate,
    /// 变量缺少右花括号。
    UnclosedBrace,
    /// 未定义的变量名。
    UnknownVariable(String),
    /// 目录规则中包含绝对路径或盘符。
    AbsoluteFolder,
    /// 目录规则试图跳出输出根目录。
    FolderEscape,
    /// 文件名超过长度上限。
    NameTooLong,
    /// 完整路径超过长度上限。
    PathTooLong,
    /// 未能在限定次数内找到可用文件名。
    CollisionExhausted,
    /// 文件名包含路径分隔符（内部调用错误）。
    IllegalName,
    /// 读写文件系统失败。
    Io(io::Error),
}

impl fmt::Display for NamingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyTemplate => write!(f, "命名规则不能为空"),
            Self::UnclosedBrace => write!(f, "变量缺少右花括号 `}}`"),
            Self::UnknownVariable(name) => write!(f, "未知变量 `{{{name}}}`"),
            Self::AbsoluteFolder => write!(f, "目录规则必须是相对路径"),
            Self::FolderEscape => write!(f, "目录规则不能跳出输出根目录"),
            Self::NameTooLong => write!(f, "文件名过长"),
            Self::PathTooLong => write!(f, "完整路径过长"),
            Self::CollisionExhausted => write!(f, "无法找到可用的文件名"),
            Self::IllegalName => write!(f, "文件名包含非法字符"),
            Self::Io(err) => write!(f, "文件操作失败：{err}"),
        }
    }
}

impl std::error::Error for NamingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for NamingError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}
