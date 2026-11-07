// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 测试辅助：固定的时间与上下文。

use crate::{NamingContext, TimeStamp};

/// 固定的渲染上下文，时间 2026-11-07 14:30:12.045（周六）。
#[must_use]
pub fn ctx() -> NamingContext<'static> {
    NamingContext {
        now: TimeStamp::new(2026, 11, 7, 14, 30, 12, 45, 6),
        device: Some("Logitech C920"),
        doc: Some("报销单"),
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
        user: Some("kaiyuan"),
    }
}
