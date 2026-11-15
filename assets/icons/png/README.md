# UI 图标 PNG

`assets/icons/png/` 中的 PNG 由 `assets/icons/svg/` 的 Tabler 图标预渲染而来。

- **Windows / macOS**：UI 直接使用 `assets/icons/svg/` 中的矢量 SVG，按目标尺寸
  光栅化，边缘更平滑。
- **Linux / UOS**：UI 使用这里的 PNG，避免部分桌面环境或图形驱动在运行时解码
  SVG 失败，导致工具栏按钮和右侧栏图标空白。

PNG 为 32×32、透明背景，接近图标在界面中的实际显示尺寸（16–17px），减少因大幅
缩小导致的锯齿。普通图标烘焙了中性灰 `#8a8f98`；相机、停止和警告图标分别烘焙为
白、红、黄，保证即使某个后端忽略了 `colorize`，图标仍然可见。正常情况下
`colorize` 会按主题重新着色。

## 重新生成

新增或修改 SVG 后，在仓库根目录运行：

```bash
cargo run --example render_icons -p usbscan-app
```
