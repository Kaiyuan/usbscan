# 第三方组件与许可证

本文件记录 usbscan 使用的全部第三方组件及其许可证，是 GPL-3.0-or-later 合规的一部分。

> 当前项目仍在持续演进。下表以 `Cargo.lock` 与实际构建结果为准，并在合并 PR 时同步更新。
>
> 最后更新：2026-11-08

---

## 1. 兼容性总则

usbscan 以 **GPL-3.0-or-later** 发布。所有依赖必须满足：

| 允许 | 说明 |
|---|---|
| ✅ MIT / BSD-2-Clause / BSD-3-Clause / ISC / Zlib | 宽松许可，可与 GPLv3 组合 |
| ✅ Apache-2.0 | GPLv3 明确兼容（GPLv2 不兼容） |
| ✅ MPL-2.0 | 文件级著佐权，可组合 |
| ✅ Unlicense / CC0 | 公共领域 |
| ⚠️ SIL OFL-1.1 | 仅限字体，作为**独立文件**分发，不并入二进制 |
| ❌ GPL-2.0-only | 与 GPLv3 不兼容 |
| ❌ SSPL / BUSL / Commons Clause | 非自由许可 |
| ❌ 无许可证声明 | 默认保留所有权利，不可使用 |

CI 通过 `cargo deny check licenses` 强制校验，白名单配置见 `deny.toml`（随工作区一起创建）。

---

## 2. 运行时依赖

| 组件 | 用途 | 许可证 | 分发形式 |
|---|---|---|---|
| `windows` / `windows-sys` | Win32、COM、Media Foundation 绑定 | MIT OR Apache-2.0 | 编译期内联 |
| **Slint**（`slint`、`slint-build`） | 界面渲染与声明式 UI | GPL-3.0-only；本项目按 GPL-3.0 使用（Slint 另有商业 / Royalty-free 授权） | 编译期内联 |
| `resvg` / `usvg` | Slint SVG 图标栅格化 | Apache-2.0 OR MIT | 编译期内联 |
| `rodio` | 拍照/录像 MP3 提示音播放 | MIT OR Apache-2.0 | 编译期内联，音频来自 `assets/*.mp3` |
| `ureq` / `native-tls` | GitHub Releases 更新检查与安装包下载（Windows SChannel） | MIT OR Apache-2.0 / Apache-2.0 OR MIT | 编译期内联 |
| `webbrowser` | 打开 GitHub 项目页 | MIT OR Apache-2.0 | 编译期内联 |
| `rfd` | 系统文件选择 / 保存对话框 | MIT OR Apache-2.0 | 编译期内联 |
| `serde` / `serde_derive` | 配置与清单序列化 | MIT OR Apache-2.0 | 编译期内联 |
| `toml` | 配置文件解析 | MIT OR Apache-2.0 | 编译期内联 |
| `thiserror` | 错误类型定义 | MIT OR Apache-2.0 | 编译期内联 |
| `tracing` / `tracing-subscriber` | 日志 | MIT | 编译期内联 |
| `image` | 图片编解码（JPG/PNG/TIFF/WebP） | MIT OR Apache-2.0 | 编译期内联 |
| **Tabler Icons** | 工具栏与界面图标字体 `assets/icons/tabler-icons.ttf` | MIT | 以字节嵌入 `usbscan.exe`；随包附 `TABLER-ICONS-LICENSE.txt` |
| `rayon` | 数据并行 | MIT OR Apache-2.0 | 编译期内联 |
| `zune-jpeg`（或等价） | MJPEG 软件解码兜底 | MIT OR Apache-2.0 | 编译期内联 |
| `parking_lot` | 同步原语 | MIT OR Apache-2.0 | 编译期内联 |
| PDF 生成库（待定） | PDF 输出 | 需在选型时确认（MIT / Apache-2.0） | 编译期内联 |
| 条码解码库（待定） | 条码识别（P2） | 需在选型时确认（MIT / Apache-2.0） | 编译期内联 |

---

## 3. 原生库

| 组件 | 用途 | 许可证 | 分发形式 | 合规动作 |
|---|---|---|---|---|
| **Tesseract 5** | OCR 引擎 | Apache-2.0 | 运行时调用用户本机安装的 `tesseract.exe`，不随安装包捆绑 | 在界面/文档中说明安装来源；用户确认后可一键从官方 GitHub Releases 下载安装 |
| **Leptonica** | Tesseract 依赖 | BSD-2-Clause | 由用户安装的 Tesseract 提供 | 不随包分发 |
| **tessdata**（`chi_sim`、`eng`） | OCR 语言模型 | Apache-2.0 | 用户自行下载或随 Tesseract 安装 | 不随包分发；用户确认后由程序从 `tessdata_fast` 下载到本机 |
| **思源黑体**（Source Han Sans 子集） | 文档渲染字体 | SIL OFL-1.1 | 独立文件 `assets/fonts/*.otf` | 随包附 OFL 许可证；不修改字体名 |

> Tesseract 与 Leptonica 由用户单独安装，或在用户确认后由 usbscan 从官方 GitHub Releases 下载安装；usbscan 只通过命令行调用，不静态链接、不随包分发。整体二进制仍按 GPLv3 分发。

---

## 4. 构建与开发工具（不随产品分发）

| 工具 | 用途 | 许可证 |
|---|---|---|
| Rust toolchain | 编译器 | MIT OR Apache-2.0 |
| Visual Studio Build Tools | MSVC 链接器 | 微软许可（允许分发编译产物） |
| Inno Setup | 安装包制作 | 自定义许可（允许自由分发安装包） |
| `cargo-deny` | 许可证审计 | MIT OR Apache-2.0 |
| `cargo-cyclonedx` | SBOM 生成 | Apache-2.0 |

---

## 5. 维护规则

1. **新增依赖必须在 PR 中说明**用途与许可证，并更新本表。
2. 依赖版本变更（`Cargo.lock` 变化）时，CI 自动运行 `cargo deny check`；失败则阻止合并。
3. 每次发布前生成 SBOM：
   ```powershell
   cargo cyclonedx --format json --output sbom.json
   ```
   随 Release 附件发布。
4. 若某依赖更换许可证为不兼容类型，应在其上游变更后的**第一个版本发布前**替换或移除。
5. 本文件的版权声明部分应与实际 `LICENSE` 文件内容保持一致。

---

## 6. 版权声明汇总

除上表所列外，usbscan 自身代码：

```
Copyright (C) 2026 usbscan contributors

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

You should have received a copy of the GNU General Public License
along with this program.  If not, see <https://www.gnu.org/licenses/>.
```
