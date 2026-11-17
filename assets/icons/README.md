# 图标资源

- `svg/`：来自 [Tabler Icons](https://tabler.io/icons) 的原始 SVG，只作为图标来源保留。
  UI 实际绘制的是 `crates/usbscan-app/ui/icons.slint` 里的矢量路径，与这里的文件一一对应；
  修改图标时两者要一起改。
- `usbscan_icons/`：应用自身图标（窗口图标、macOS `.icns`、Linux `.desktop` 图标）。
- `usbscan.ico` / `usbscan_box.ico`：Windows 可执行文件图标。
- `tabler-icons.ttf`：Win32 侧使用的字体图标，由 `crates/usbscan-win/src/icons.rs` 嵌入。

UI 图标不再走「图片 + colorize」这条路径：部分 Linux / UOS 桌面环境（旧版 Mesa、
llvmpipe 等）上 colorize 依赖离屏纹理和 SourceIn 混合，可能渲染成空白。
`Path` 矢量绘制只用到最基础的填充/描边，在所有后端上表现一致，并且任意 DPI 下都锐利。
