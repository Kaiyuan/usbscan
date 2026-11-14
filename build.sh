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
    cargo fmt --all -- --check || true
    echo "> cargo clippy --workspace"
    cargo clippy --workspace || true
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

if [[ -n "$TARGET" ]]; then
    BUILT_BIN="target/$TARGET/$CONFIGURATION/usbscan"
else
    BUILT_BIN="target/$CONFIGURATION/usbscan"
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
echo "Installation completed!"
INSTALL_SH
    chmod +x "$PACKAGE_DIR/install.sh"

    ARCHIVE="$DIST_DIR/usbscan-$VERSION-linux-$ARCH.tar.gz"
    tar -czf "$ARCHIVE" -C "$DIST_DIR" "usbscan-$VERSION-linux-$ARCH"
    echo "Created: $ARCHIVE"

    # 打包 UOS / Debian .deb 包
    if [[ $BUILD_DEB -eq 1 ]] || command -v dpkg-deb >/dev/null 2>&1; then
        echo "Building .deb package for UOS ($DEB_ARCH)..."
        DEB_BUILD_DIR="$DIST_DIR/deb-build-$DEB_ARCH"
        rm -rf "$DEB_BUILD_DIR"
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
Depends: libv4l-0, poppler-utils
Suggests: tesseract-ocr, tesseract-ocr-chi-sim
DEB_CONTROL

        DEB_FILE="$DIST_DIR/usbscan_${VERSION}_${DEB_ARCH}.deb"
        dpkg-deb --build --root-owner-group "$DEB_BUILD_DIR" "$DEB_FILE"
        rm -rf "$DEB_BUILD_DIR"
        echo "Created: $DEB_FILE"
    fi
fi

echo ""
echo "Build and packaging completed successfully!"
