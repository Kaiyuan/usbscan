# usbscan

> 把任意 USB 摄像头变成文档扫描仪。

Windows 10 / 11 · Rust · 原生 Win32 界面 · 完全离线 · 开源

---

## 这是什么

usbscan 是一款面向 Windows 的文档采集软件。插上任一 UVC 摄像头（网络摄像头、高拍仪、工业模组、笔记本内置摄像头），即可完成 **拍摄 → 自动校正 → 增强 → OCR → 归档** 的全流程，输出可直接归档的 PDF 或图片。

产品不做云端、不做注册、不做遥测。所有识别与处理都在本机完成。

## 核心特性

| 特性 | 说明 |
|---|---|
| 任意摄像头 | Media Foundation 为主、DirectShow 兜底，覆盖杂牌驱动与非标格式 |
| 边拍边存 | 每次快门立即生成成品文件，断电崩溃不丢已拍内容 |
| 自动校正 | 文档边缘检测、透视矫正、纠偏、去阴影、白底净化 |
| 离线 OCR | 内置 Tesseract 中文/英文模型，不联网、不上传 |
| 命名规则 | 模板化文件名与目录规则，拍摄瞬间即确定落盘路径 |
| 原生界面 | 系统标准控件、系统字体、系统对话框，无自绘框架 |
| 零依赖 | 静态链接 CRT，不需要 .NET / VC++ 运行时 / WebView2 |
| 开源 | 代码可审计，可自行构建，可二次开发 |

## 文档

| 文档 | 内容 |
|---|---|
| [docs/01-需求规格.md](docs/01-需求规格.md) | 产品定位、功能需求、非功能需求、验收标准 |
| [docs/02-架构设计.md](docs/02-架构设计.md) | 技术选型、模块划分、线程模型、核心接口、决策记录 |
| [docs/03-UI原型.md](docs/03-UI原型.md) | 视觉规范、界面线框、交互状态、快捷键 |
| [docs/04-命名规则.md](docs/04-命名规则.md) | 文件名与目录模板语法、校验、冲突策略 |
| [docs/05-摄像头兼容性.md](docs/05-摄像头兼容性.md) | 双后端策略、格式协商、异常处理、测试矩阵 |
| [docs/06-开源与发布.md](docs/06-开源与发布.md) | 许可证、第三方组件、构建、分发、隐私 |
| [docs/07-开发计划.md](docs/07-开发计划.md) | 里程碑、任务分解、风险登记 |

文档总索引见 [docs/README.md](docs/README.md)。

## 构建

> 当前已可构建运行 M1 部分里程碑：Win32 原生外壳、Media Foundation 实时预览、
> 设备切换与热插拔、按命名规则拍照落盘、会话恢复、缩略图列表与 3 秒撤销。
> 增强、DShow 兜底、OCR、导出等功能仍在开发中。

```powershell
# 前置：Rust stable + Visual Studio Build Tools（C++ 桌面开发负载）
git clone https://github.com/<owner>/usbscan.git
cd usbscan
cargo build --release -p usbscan-app
# 产物：target\release\usbscan.exe
```

运行测试：

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

## 目录结构

```
usbscan/
├─ crates/
│  ├─ usbscan-app/      # 程序入口与生命周期
│  ├─ usbscan-capture/  # Media Foundation 采集后端
│  ├─ usbscan-doc/      # 会话、页面、撤销与 session.json 持久化
│  ├─ usbscan-imaging/  # JPEG 编解码与缩略图
│  ├─ usbscan-naming/   # 文件名/目录命名规则引擎
│  └─ usbscan-win/      # Win32 原生界面与消息循环
├─ docs/                # 设计文档
└─ Cargo.toml           # workspace 配置
```

## 许可证

[GNU General Public License v3.0 or later](LICENSE)（GPL-3.0-or-later）。

你可以自由使用、修改、分发本软件；但如果你分发**二进制**，必须同时提供完整源代码，并以同样的许可证授权衍生作品。

Copyright (C) 2026 usbscan contributors

本程序为自由软件，在自由软件联盟发布的 GNU 通用公共许可证条款下发布；无论是第 3 版，还是（由你选择）任何更新版本。本程序基于"有用"的目的分发，但**不提供任何担保**，甚至不包含适销性或特定用途适用性的默示担保。详见 [LICENSE](LICENSE)。

义务清单与合规要求见 [docs/06-开源与发布.md](docs/06-开源与发布.md#1-许可证)。

## 参与贡献

见 [docs/06-开源与发布.md](docs/06-开源与发布.md#7-贡献指南)。
