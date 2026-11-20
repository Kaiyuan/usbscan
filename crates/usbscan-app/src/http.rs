// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 统一构造 HTTP Agent。
//!
//! **Windows**：使用系统 `SChannel`（`native-tls`）。
//!
//! **Linux / macOS**：使用 `ureq` 内置的 rustls（根证书随二进制打包），
//! 不依赖系统的 OpenSSL，避免 UOS V20 的 `libssl1.1` 与 V25 的
//! `libssl3` 差异导致安装失败。

#[cfg(windows)]
use std::sync::Arc;

/// 构造带平台 TLS 后端的 Agent。
// Linux 分支不会失败，但 Windows 的 `TlsConnector::new()` 可能出错，
// 因此保留统一的 `Result` 接口。
#[allow(clippy::unnecessary_wraps)]
pub fn agent() -> Result<ureq::Agent, String> {
    #[cfg(windows)]
    {
        let connector = native_tls::TlsConnector::new().map_err(|err| err.to_string())?;
        Ok(ureq::AgentBuilder::new()
            .tls_connector(Arc::new(connector))
            .build())
    }
    #[cfg(not(windows))]
    {
        Ok(ureq::AgentBuilder::new().build())
    }
}

#[cfg(test)]
mod tests {
    use super::agent;

    #[test]
    fn builds_an_agent_with_the_platform_tls_backend() {
        assert!(agent().is_ok());
    }
}
