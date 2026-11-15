#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 usbscan contributors
#
# Build, test and package usbscan on macOS and Linux (including UOS/Debian).

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

CONFIGURATION="release"
TARGET=""
SKIP_LINT=0
SKIP_TESTS=0
CLEAN=0
NO_PACKAGE=0
BUILD_DEB=0

print_usage() {
    cat <<EOF
Usage: ./build.sh [OPTIONS]

Options:
  --debug          Build debug profile (default: release)
  --target TRIPLE  Cross-compile for a specific Rust target triple (e.g. aarch64-apple-darwin)
  --skip-lint      Skip cargo fmt and cargo clippy
  --skip-tests     Skip cargo test
  --clean          Run cargo clean before building
  --no-package     Build only, do not package into dist/
  --deb            Build .deb package for UOS / Debian (Linux only)
  -h, --help       Show this help message
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --debug)
            CONFIGURATION="debug"
            shift
            ;;
        --target)
            TARGET="$2"
            shift 2
            ;;
        --skip-lint)
            SKIP_LINT=1
            shift
            ;;
        --skip-tests)
            SKIP_TESTS=1
            shift
            ;;
        --clean)
            CLEAN=1
            shift
            ;;
        --no-package)
            NO_PACKAGE=1
            shift
            ;;
        --deb)
            BUILD_DEB=1
            shift
            ;;
        -h|--help)
            print_usage
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            print_usage
            exit 1
            ;;
    esac
done

command -v cargo >/dev/null 2>&1 || { echo "Error: cargo not found. Please install Rust from https://rustup.rs/" >&2; exit 1; }
command -v rustc >/dev/null 2>&1 || { echo "Error: rustc not found." >&2; exit 1; }

# 依赖的最低 Rust 版本（与 Cargo.toml 的 rust-version 保持一致）。
REQUIRED_RUST="1.88"
RUSTC_VERSION="$(rustc --version | awk '{print $2}' | sed 's/[^0-9.].*$//')"

# 返回 0 表示 $1 < $2；纯 bash 实现，兼容 macOS 的 bash 3.2 / BSD sort。
version_lt() {
    local IFS=.
    local -a left=($1)
    local -a right=($2)
    local i
    for i in 0 1 2; do
        if (( ${left[i]:-0} < ${right[i]:-0} )); then return 0; fi
        if (( ${left[i]:-0} > ${right[i]:-0} )); then return 1; fi
    done
    return 1
}

if version_lt "$RUSTC_VERSION" "$REQUIRED_RUST"; then
    echo "Error: Rust $REQUIRED_RUST or newer is required (found $RUSTC_VERSION)." >&2
    echo "       Install it with: rustup update stable" >&2
    exit 1
fi

OS_TYPE="$(uname -s)"
HOST_ARCH="$(uname -m)"

# 确定目标平台和架构
if [[ -n "$TARGET" ]]; then
    if [[ "$TARGET" =~ apple-darwin ]]; then
        OS_TYPE="Darwin"
    elif [[ "$TARGET" =~ linux ]]; then
        OS_TYPE="Linux"
    fi

    if [[ "$TARGET" =~ aarch64|arm64 ]]; then
        ARCH="arm64"
        DEB_ARCH="arm64"
    elif [[ "$TARGET" =~ x86_64 ]]; then
        ARCH="x64"
        DEB_ARCH="amd64"
    else
        ARCH="$TARGET"
        DEB_ARCH="$TARGET"
    fi
else
    case "$HOST_ARCH" in
        x86_64|amd64) ARCH="x64"; DEB_ARCH="amd64" ;;
        aarch64|arm64) ARCH="arm64"; DEB_ARCH="arm64" ;;
        *) ARCH="$HOST_ARCH"; DEB_ARCH="$HOST_ARCH" ;;
    esac
fi

CARGO_TOML="Cargo.toml"
VERSION="$(grep -m1 '^version' "$CARGO_TOML" | sed -E 's/.*"([^"]+)".*/\1/')"

echo ""
echo "=== usbscan build ==="
echo "  Version : $VERSION"
echo "  Profile : $CONFIGURATION"
echo "  Target  : ${TARGET:-default}"
echo "  OS      : $OS_TYPE"
echo "  Arch    : $ARCH"
echo "  Root    : $ROOT_DIR"
echo ""

if [[ $CLEAN -eq 1 ]]; then
    echo "> cargo clean"
    cargo clean
fi

if [[ $SKIP_LINT -eq 0 && -z "$TARGET" ]]; then
    echo "> cargo fmt --all -- --check"
    cargo fmt --all -- --check
    echo "> cargo clippy --workspace --all-targets -- -D warnings"
    cargo clippy --workspace --all-targets -- -D warnings
fi

if [[ $SKIP_TESTS -eq 0 && -z "$TARGET" ]]; then
    echo "> cargo test --workspace"
    cargo test --workspace
fi

BUILD_ARGS=("build" "-p" "usbscan-app")
if [[ "$CONFIGURATION" == "release" ]]; then
    BUILD_ARGS+=("--release")
fi
if [[ -n "$TARGET" ]]; then
    BUILD_ARGS+=("--target" "$TARGET")
fi

echo "> cargo ${BUILD_ARGS[*]}"
cargo "${BUILD_ARGS[@]}"

# 与 cargo 保持一致：允许通过 CARGO_TARGET_DIR 自定义输出目录。
TARGET_ROOT="${CARGO_TARGET_DIR:-$ROOT_DIR/target}"
if [[ -n "$TARGET" ]]; then
    BUILT_BIN="$TARGET_ROOT/$TARGET/$CONFIGURATION/usbscan"
else
    BUILT_BIN="$TARGET_ROOT/$CONFIGURATION/usbscan"
fi

if [[ ! -f "$BUILT_BIN" ]]; then
    echo "Error: Build succeeded but binary was not found at $BUILT_BIN" >&2
    exit 1
fi

if [[ $NO_PACKAGE -eq 1 || "$CONFIGURATION" != "release" ]]; then
    echo "Binary built: $BUILT_BIN"
    exit 0
fi

DIST_DIR="$ROOT_DIR/dist"
mkdir -p "$DIST_DIR"

if [[ "$OS_TYPE" == "Darwin" ]]; then
    echo "Packaging for macOS (.app & .tar.gz)..."
    APP_BUNDLE="$DIST_DIR/usbscan.app"
    rm -rf "$APP_BUNDLE"
    mkdir -p "$APP_BUNDLE/Contents/MacOS"
    mkdir -p "$APP_BUNDLE/Contents/Resources"

    cp "$BUILT_BIN" "$APP_BUNDLE/Contents/MacOS/usbscan"
    chmod +x "$APP_BUNDLE/Contents/MacOS/usbscan"

    if [[ -f "assets/macos/Info.plist" ]]; then
        cp "assets/macos/Info.plist" "$APP_BUNDLE/Contents/Info.plist"
        # 同步版本号，避免 Info.plist 与 Cargo.toml 版本漂移。
        if command -v plutil >/dev/null 2>&1; then
            plutil -replace CFBundleShortVersionString -string "$VERSION" "$APP_BUNDLE/Contents/Info.plist"
            plutil -replace CFBundleVersion -string "$VERSION" "$APP_BUNDLE/Contents/Info.plist"
        fi
    fi

    # 生成 AppIcon.icns（如果在 macOS 原生环境且有 iconutil）
    if command -v iconutil >/dev/null 2>&1 && [[ -d "assets/icons/usbscan_icons" ]]; then
        ICONSET_DIR="$(mktemp -d)/AppIcon.iconset"
        mkdir -p "$ICONSET_DIR"
        cp "assets/icons/usbscan_icons/usbscan_16x16.png" "$ICONSET_DIR/icon_16x16.png" 2>/dev/null || true
        cp "assets/icons/usbscan_icons/usbscan_32x32.png" "$ICONSET_DIR/icon_16x16@2x.png" 2>/dev/null || true
        cp "assets/icons/usbscan_icons/usbscan_32x32.png" "$ICONSET_DIR/icon_32x32.png" 2>/dev/null || true
        cp "assets/icons/usbscan_icons/usbscan_64x64.png" "$ICONSET_DIR/icon_32x32@2x.png" 2>/dev/null || true
        cp "assets/icons/usbscan_icons/usbscan_128x128.png" "$ICONSET_DIR/icon_128x128.png" 2>/dev/null || true
        cp "assets/icons/usbscan_icons/usbscan_256x256.png" "$ICONSET_DIR/icon_128x128@2x.png" 2>/dev/null || true
        cp "assets/icons/usbscan_icons/usbscan_256x256.png" "$ICONSET_DIR/icon_256x256.png" 2>/dev/null || true
        cp "assets/icons/usbscan_icons/usbscan_512x512.png" "$ICONSET_DIR/icon_256x256@2x.png" 2>/dev/null || true
        cp "assets/icons/usbscan_icons/usbscan_512x512.png" "$ICONSET_DIR/icon_512x512.png" 2>/dev/null || true
        cp "assets/icons/usbscan_icons/usbscan_1024x1024.png" "$ICONSET_DIR/icon_512x512@2x.png" 2>/dev/null || true
        iconutil -c icns "$ICONSET_DIR" -o "$APP_BUNDLE/Contents/Resources/AppIcon.icns" 2>/dev/null || true
        rm -rf "$(dirname "$ICONSET_DIR")"
    fi

    # 未做正式签名/公证，至少做 ad-hoc 签名，减少 Gatekeeper 直接拦截。
    if command -v codesign >/dev/null 2>&1; then
        codesign --force --deep --sign - "$APP_BUNDLE" 2>/dev/null || true
    fi

    ARCHIVE="$DIST_DIR/usbscan-$VERSION-macos-$ARCH.tar.gz"
    tar -czf "$ARCHIVE" -C "$DIST_DIR" "usbscan.app"
    echo "Created: $ARCHIVE"

elif [[ "$OS_TYPE" == "Linux" ]]; then
    echo "Packaging for Linux / UOS..."
    PACKAGE_DIR="$DIST_DIR/usbscan-$VERSION-linux-$ARCH"
    rm -rf "$PACKAGE_DIR"
    mkdir -p "$PACKAGE_DIR/bin"
    mkdir -p "$PACKAGE_DIR/share/applications"
    mkdir -p "$PACKAGE_DIR/share/icons/hicolor/256x256/apps"

    cp "$BUILT_BIN" "$PACKAGE_DIR/bin/usbscan"
    chmod +x "$PACKAGE_DIR/bin/usbscan"

    cp "assets/usbscan.desktop" "$PACKAGE_DIR/share/applications/"
    if [[ -f "assets/icons/usbscan_icons/usbscan_256x256.png" ]]; then
        cp "assets/icons/usbscan_icons/usbscan_256x256.png" "$PACKAGE_DIR/share/icons/hicolor/256x256/apps/usbscan.png"
    fi

    for doc in LICENSE README.md THIRD_PARTY.md; do
        [[ -f "$doc" ]] && cp "$doc" "$PACKAGE_DIR/"
    done

    # 包含安装和卸载简易脚本
    cat <<'INSTALL_SH' > "$PACKAGE_DIR/install.sh"
#!/usr/bin/env bash
set -e
PREFIX="${PREFIX:-/usr/local}"
echo "Installing usbscan to $PREFIX..."
install -Dm755 bin/usbscan "$PREFIX/bin/usbscan"
install -Dm644 share/applications/usbscan.desktop "$PREFIX/share/applications/usbscan.desktop"
if [ -f share/icons/hicolor/256x256/apps/usbscan.png ]; then
    install -Dm644 share/icons/hicolor/256x256/apps/usbscan.png "$PREFIX/share/icons/hicolor/256x256/apps/usbscan.png"
fi
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q || true

# 离线环境提示：缺少运行时库时程序无法启动。
if command -v ldd >/dev/null 2>&1; then
    missing="$(ldd "$PREFIX/bin/usbscan" 2>/dev/null | awk '/not found/{print $1}' | sort -u)"
    if [ -n "$missing" ]; then
        echo ""
        echo "Warning: the following runtime libraries are missing:"
        echo "$missing"
        echo "Install them with your package manager before running usbscan."
    fi
fi

echo ""
echo "Optional components for offline use:"
echo "  OCR       : tesseract-ocr tesseract-ocr-chi-sim tesseract-ocr-eng"
echo "  PDF pages : poppler-utils"
echo "  File dialogs: xdg-desktop-portal"
echo "Installation completed!"
INSTALL_SH
    chmod +x "$PACKAGE_DIR/install.sh"

    ARCHIVE="$DIST_DIR/usbscan-$VERSION-linux-$ARCH.tar.gz"
    tar -czf "$ARCHIVE" -C "$DIST_DIR" "usbscan-$VERSION-linux-$ARCH"
    echo "Created: $ARCHIVE"

    # 打包 UOS / Debian .deb 包
    if [[ $BUILD_DEB -eq 1 ]] || command -v dpkg-deb >/dev/null 2>&1; then
        echo "Building .deb package for UOS ($DEB_ARCH)..."
        # 在系统临时目录中打包，避免 DrvFs 等文件系统不支持 Unix 权限位。
        DEB_WORK="$(mktemp -d)"
        DEB_BUILD_DIR="$DEB_WORK/usbscan-deb"
        mkdir -p "$DEB_BUILD_DIR/DEBIAN"
        mkdir -p "$DEB_BUILD_DIR/usr/bin"
        mkdir -p "$DEB_BUILD_DIR/usr/share/applications"
        mkdir -p "$DEB_BUILD_DIR/usr/share/icons/hicolor/256x256/apps"

        cp "$BUILT_BIN" "$DEB_BUILD_DIR/usr/bin/usbscan"
        chmod +x "$DEB_BUILD_DIR/usr/bin/usbscan"
        cp "assets/usbscan.desktop" "$DEB_BUILD_DIR/usr/share/applications/"
        if [[ -f "assets/icons/usbscan_icons/usbscan_256x256.png" ]]; then
            cp "assets/icons/usbscan_icons/usbscan_256x256.png" "$DEB_BUILD_DIR/usr/share/icons/hicolor/256x256/apps/usbscan.png"
        fi

        # 计算运行时共享库依赖；dpkg-shlibdeps（dpkg-dev）缺失时回退到常见包名。
        SHLIBS_DEPENDS=""
        if command -v dpkg-shlibdeps >/dev/null 2>&1; then
            SHLIBS_TMP="$(mktemp -d)"
            mkdir -p "$SHLIBS_TMP/debian"
            cat > "$SHLIBS_TMP/debian/control" <<DEB_SOURCE
Source: usbscan

Package: usbscan
Architecture: $DEB_ARCH
DEB_SOURCE
            cp "$BUILT_BIN" "$SHLIBS_TMP/usbscan"
            SHLIBS_DEPENDS="$(cd "$SHLIBS_TMP" && dpkg-shlibdeps -O usbscan 2>/dev/null | sed -n 's/^shlibs:Depends=//p')"
            rm -rf "$SHLIBS_TMP"
        fi
        if [[ -z "$SHLIBS_DEPENDS" ]]; then
            # 离线安装时 dpkg -i 不会补齐依赖，这里给出覆盖 UOS 20 / Debian 10-12 的完整回退列表。
            SHLIBS_DEPENDS="libc6, libgcc1 | libgcc-s1, libssl1.1 | libssl3 | libssl3t64, libfontconfig1, libfreetype6, libasound2 | libasound2t64, libexpat1, zlib1g, libpng16-16, libbz2-1.0, libbrotli1, libzstd1"
        fi

        cat <<DEB_CONTROL > "$DEB_BUILD_DIR/DEBIAN/control"
Package: usbscan
Version: $VERSION
Section: utils
Priority: optional
Architecture: $DEB_ARCH
Maintainer: usbscan contributors <https://github.com/Kaiyuan/usbscan>
Description: Lightweight USB Document Scanner
 A modern, offline document scanning and OCR tool for USB cameras and doc scanners.
 Compatible with Deepin / UnionTech OS (UOS) and Debian-based Linux.
Depends: $SHLIBS_DEPENDS
Recommends: libgl1, libegl1, libxkbcommon0, xdg-desktop-portal, poppler-utils
Suggests: tesseract-ocr, tesseract-ocr-chi-sim, tesseract-ocr-eng
DEB_CONTROL

        # 某些文件系统（如 WSL 的 DrvFs）会给出 777 权限，dpkg-deb 要求控制目录为 0755。
        chmod 0755 "$DEB_BUILD_DIR/DEBIAN"
        chmod 0644 "$DEB_BUILD_DIR/DEBIAN/control"

        DEB_FILE="$DIST_DIR/usbscan_${VERSION}_${DEB_ARCH}.deb"
        dpkg-deb --build --root-owner-group "$DEB_BUILD_DIR" "$DEB_FILE"
        rm -rf "$DEB_WORK"
        echo "Created: $DEB_FILE"
    fi
fi

echo ""
echo "Build and packaging completed successfully!"
