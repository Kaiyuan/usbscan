# UI 图标 PNG

`assets/icons/png/` 中的 PNG 由 `assets/icons/svg/` 的 Tabler 图标预渲染而来，
UI（`crates/usbscan-app/ui/main.slint`）只引用这里的 PNG。

这样做的原因：部分 Linux / UOS 桌面环境或图形驱动下，运行时解码 SVG 图标可能
失败，导致工具栏按钮和右侧栏图标空白。PNG 解码在 Slint 的各渲染后端上都更稳定。

图标为 96×96、透明背景。普通图标烘焙了中性灰 `#8a8f98`；相机、停止和警告
图标分别烘焙为白、红、黄，保证即使某个后端忽略了 `colorize`，图标仍然可见。
正常情况下 `colorize` 会按主题重新着色。

## 重新生成

新增或修改 SVG 后，在仓库根目录运行：

```bash
cargo run --example render_icons -p usbscan-app
```
