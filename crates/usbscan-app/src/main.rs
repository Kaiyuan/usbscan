// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(err) = usbscan_win::run() {
        eprintln!("usbscan 启动失败：{err}");
        std::process::exit(1);
    }
}
