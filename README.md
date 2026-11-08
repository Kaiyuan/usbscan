# usbscan

> 把任意 USB 摄像头变成文档扫描仪。

Windows 10 / 11 · Rust · Slint 界面 · 完全离线 · 开源

[项目主页](https://kaiyuan.github.io/usbscan/) · [下载最新版本](https://github.com/Kaiyuan/usbscan/releases/latest)

项目主页支持简体中文、繁體中文、English、Deutsch、Français、Italiano、Español、日本語、한국어，并按浏览器语言自动选择。

---

## 这是什么

usbscan 是一款面向 Windows 的文档采集软件。插上任一 UVC 摄像头（网络摄像头、高拍仪、工业模组、笔记本内置摄像头），即可完成 **拍摄 → 自动校正 → 增强 → OCR → 归档** 的全流程，输出可直接归档的 PDF 或图片。

产品不做云端、不做注册、不做遥测。所有识别与处理都在本机完成。

## 核心特性

| 特性 | 说明 |
|---|---|
| 任意摄像头 | Media Foundation 为主、DirectShow 兜底，覆盖杂牌驱动与非标格式；可在界面选择设备支持的分辨率 |
| 边拍边存 | 每次快门立即生成成品文件，断电崩溃不丢已拍内容 |
| 连拍 / 定时 / 录像 | 连拍张数与间隔、定时秒数可配置；录像保存为 MP4（H.264），帧率可选 25/30/50/60 fps（默认 25） |
| 页面管理 | 缩略图按最新优先排列、选择、删除、上移/下移、双击查看、右键菜单操作、在资源管理器中定位、新建文档、打开其他工程；可隐藏左右面板让预览全宽 |
| 页面编辑 | 顺时针/逆时针旋转、水平镜像、手动裁剪、手写/矩形批注、复制图片到剪贴板、使用保留原图重新处理增强；右键菜单支持重命名、自动裁边纠畸变、自动纠偏；图片编辑支持撤销/重做 |
| 扫描设置 | 输出目录、文件名/子目录模板、重名策略、JPEG 质量，实时示例预览 |
| 单页导出 | 把选中页面复制到指定目录 |
| 多页 PDF | 导出 PDF（JPEG 原图嵌入），可选纸张、方向、页边距与缩放；已 OCR 页面自动写入可搜索文本层 |
| Word 导出 | 把全部页面按顺序导出为 `.docx`，每页一张图片、自动分页，可直接用 Word/WPS 打开 |
| 批量导出 | 导出全部页面 JPG，或合并全部页面 OCR 文本为单个 `.txt` |
| 导入图片 | 从系统文件对话框导入 JPG/PNG/BMP，自动转成 JPEG 并加入当前会话；支持 Ctrl+V 粘贴位图 |
| 导入 PDF | 使用 Windows 自带 PDF 渲染器逐页转成 JPEG，自动加入当前会话 |
| 打印 | 原生打印对话框 + 打印预览，支持整份会话逐页输出到打印机 |
| 自动校正 | 自动文档边界检测（亮纸张连通域 + 形态学过滤 + 投影收缩 + 覆盖率/边缘校验）；勾选后实时在预览上显示裁剪框，拍摄时自动裁边并透视纠正畸变；检测不可信时安全跳过，可手动拖动四角微调 |
| 增强预设 | 内置“文字文档 / 票据 / 照片”预设，一键切换增强选项与强度 |
| 离线 OCR | 调用本机 Tesseract 5，结果窗口叠加识别行框可点击定位，并按版面段落分组；支持便携 `tesseract\` 目录 |
| 命名规则 | 模板化文件名与目录规则，拍摄瞬间即确定落盘路径 |
| 界面 | Slint 自绘的 macOS 风格界面（cupertino）：圆角卡片、macOS 开关、系统亮/暗主题；左上角显示 UsbScan 标识；刷新/录像/拍照带 SVG 图标；设备、分辨率与录像帧率同处顶栏；窗口可自由缩放并记忆大小/位置；预览实时绘制文档裁剪框 |
| 操作反馈 | 拍照时播放快门声、预览闪白并显示“已拍照”；录像开始/停止播放提示音并显示状态提示 |
| 自动更新 | 点击左上角 logo 打开关于窗口，可勾选“自动检查更新”；启动时检查 GitHub Releases，发现新版本在右下角提示，点击下载安装包并启动安装 |
| 页面操作 | 在页面缩略图上单击右键打开菜单：重命名、删除、自动裁边、自动纠偏、OCR、导出 JPG |
| 视频裁剪 | 自动识别并裁掉部分 UVC 驱动补出的上下黑边，预览、拍照与录像都不再带黑边 |
| 图标 | 使用 [Tabler Icons](https://github.com/tabler/tabler-icons) 的 SVG（MIT），并随包保留许可证 |
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

> 当前 `usbscan-app` 已使用 Slint 重写界面，默认提供：Media Foundation 实时预览、
> 设备切换与热插拔、DirectShow 兜底采集、按命名规则拍照落盘、会话恢复、
> 页面缩略图与选择、重命名/删除/自动裁边/自动纠偏、扫描设置、JPEG/PDF 导出、
> 图片导入、自动裁边/透视校正/纠偏/去阴影/白底净化、采集分辨率选择、
> MP4/H.264 录像，以及基于本机 Tesseract 5 的离线 OCR。
> 旧 Win32 界面保留在 `usbscan-win` 中作为参考，打印、批注、Word 导出等能力
> 仍在逐步迁移到 Slint 界面。

前置条件：Rust stable + Visual Studio Build Tools（“使用 C++ 的桌面开发”负载）。

```powershell
git clone https://github.com/Kaiyuan/usbscan.git
cd usbscan
```

### 一键构建（推荐）

```powershell
powershell -ExecutionPolicy Bypass -File .\build.ps1
```

脚本依次执行格式检查、Clippy、全部测试和 Release 构建，并在 `dist\` 生成：

- `usbscan.exe`
- `usbscan-<版本>-win-x64.zip`
- `SHA256SUMS.txt`

常用参数：

```powershell
.\build.ps1 -SkipTests      # 跳过测试
.\build.ps1 -SkipLint       # 跳过 fmt / clippy
.\build.ps1 -Configuration debug
.\build.ps1 -Clean          # 先 cargo clean
.\build.ps1 -NoPackage      # 只构建，不生成 dist
```

### 手工构建

```powershell
cargo build --release -p usbscan-app
# 产物：target\release\usbscan.exe
```

运行测试：

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

## 发布

在 GitHub 创建并发布 Release（tag 与 `Cargo.toml` 的工作区版本一致，例如 `v0.1.0`）后，`.github/workflows/release.yml` 会自动构建并上传：

- `usbscan-<版本>-setup.exe`：Inno Setup 安装版（`installer/usbscan.iss`）
- `usbscan-<版本>-portable.zip`：便携版
- `SHA256SUMS.txt`：SHA-256 校验和

## 目录结构

```
usbscan/
├─ crates/
│  ├─ usbscan-app/      # Slint 界面（ui/main.slint）与控制器
│  ├─ usbscan-capture/  # Media Foundation / DirectShow 采集后端
│  ├─ usbscan-doc/      # 会话、页面与 session.json 持久化
│  ├─ usbscan-imaging/  # JPEG 编解码、文档检测、增强、黑边裁剪
│  ├─ usbscan-naming/   # 文件名/目录命名规则引擎
│  └─ usbscan-win/      # 录像、配置、主题与旧 Win32 界面参考
├─ docs/                # 设计文档
├─ build.ps1            # 一键构建、检查与打包
├─ dist/                # 构建产物（git 忽略）
└─ Cargo.toml           # workspace 配置
```

## 摄像头无法打开时

1. 确认摄像头没有被“相机”、Teams、Zoom、OBS 等程序占用，关闭后按 `F5` 重新检测。
2. 打开 **Windows 设置 → 隐私和安全性 → 相机**，确认“相机访问”和“允许桌面应用访问相机”已开启。
3. 在设备管理器中确认摄像头没有黄色感叹号；必要时重新插拔 USB。
4. 程序会枚举原生格式，并尝试把 MJPEG / YUY2 / NV12 自动转换为 RGB32；若高级转换被驱动拒绝，会自动回退到兼容转换路径。
5. 查看诊断日志：`%APPDATA%\usbscan\usbscan.log`，其中记录设备列表、枚举结果和失败步骤。
6. 若日志显示 **“激活摄像头失败：… 0x80070057”**，该设备通常是虚拟摄像头（OBS、投屏、会议软件）或 MF 驱动不兼容；请先关闭虚拟摄像头软件，或换用物理 USB 摄像头。DirectShow 兜底后端会自动尝试。
7. 若日志显示 **“相机访问被拒绝（0x80070005）”**，但 Windows“相机”应用能正常预览，请先把 `usbscan.exe` 移动到系统盘的用户目录（例如 `C:\Users\<用户名>\AppData\Local\Programs\usbscan\`）再运行；部分机器的隐私策略或安全软件会拒绝非系统盘路径的应用访问摄像头。

## 许可证

[GNU General Public License v3.0 or later](LICENSE)（GPL-3.0-or-later）。

你可以自由使用、修改、分发本软件；但如果你分发**二进制**，必须同时提供完整源代码，并以同样的许可证授权衍生作品。

Copyright (C) 2026 usbscan contributors

本程序为自由软件，在自由软件联盟发布的 GNU 通用公共许可证条款下发布；无论是第 3 版，还是（由你选择）任何更新版本。本程序基于"有用"的目的分发，但**不提供任何担保**，甚至不包含适销性或特定用途适用性的默示担保。详见 [LICENSE](LICENSE)。

义务清单与合规要求见 [docs/06-开源与发布.md](docs/06-开源与发布.md#1-许可证)。

## 参与贡献

见 [docs/06-开源与发布.md](docs/06-开源与发布.md#7-贡献指南)。
