// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 统一构造带系统 TLS 后端的 HTTP Agent。
//!
//! `ureq` 的快捷函数（`ureq::get` 等）只认 rustls；本项目关闭了默认特性，
//! 改用 native-tls（Windows 上是 SChannel），因此所有请求都必须经由
//! 显式传入连接器的 [`agent`]，否则 HTTPS 会报
//! “no TLS backend is configured”。

use std::sync::Arc;

/// 使用系统 TLS 构建 Agent。
pub fn agent() -> Result<ureq::Agent, String> {
    let connector = native_tls::TlsConnector::new().map_err(|err| err.to_string())?;
    Ok(ureq::AgentBuilder::new()
        .tls_connector(Arc::new(connector))
        .build())
}

#[cfg(test)]
mod tests {
    use super::agent;

    #[test]
    fn builds_an_agent_with_the_system_tls_backend() {
        assert!(agent().is_ok());
    }
}
