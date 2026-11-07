#!/bin/bash
# arm64 deb 打包脚本
# 用法: ./build_deb_aarch64.sh
# 产物: sidera_3.0-Electro-stable_arm64.deb

set -e
cd "$(dirname "$0")"

BINARY="annotate_aarch64"
PKG_NAME="sidera"
VERSION="${SIDERA_VERSION:-3.0-Electro-stable}"
dpkg --validate-version "$VERSION" || exit 1
[[ "$VERSION" != */* && "$VERSION" != *[[:space:]]* ]] || exit 1
ARCH="arm64"

if [ ! -f "$BINARY" ]; then
  echo "错误: 找不到 $BINARY，请先编译"
  exit 1
fi

DEPENDS="$(python3 ../tools/deb_dependencies.py "$BINARY" "$ARCH")"
OUTPUT="$PWD/${PKG_NAME}_${VERSION}_${ARCH}.deb"
PKG_DIR="$(mktemp -d "$PWD/.sidera-package.XXXXXXXX")"
trap 'rm -rf -- "$PKG_DIR"; rm -f -- "$PKG_DIR.deb"' EXIT

# 清理并创建目录结构
mkdir -p "${PKG_DIR}/DEBIAN"
mkdir -p "${PKG_DIR}/usr/bin"
mkdir -p "${PKG_DIR}/usr/share/applications"

# 拷贝二进制
cp "$BINARY" "${PKG_DIR}/usr/bin/sidera"
chmod 755 "${PKG_DIR}/usr/bin/sidera"

# ---------- 拷贝 WPS 加载项部署包（本 deb 已含桥接加载项 + 安装脚本）----------
mkdir -p "${PKG_DIR}/usr/share/sidera/wps-addin"
cp -r ../wps-addin/* "${PKG_DIR}/usr/share/sidera/wps-addin/"
chmod 755 "${PKG_DIR}/usr/share/sidera/wps-addin/install.sh"
install -m 755 ../wps-addin/install.sh "${PKG_DIR}/usr/bin/sidera-wps-addin-install"

# ---------- 教室部署说明文档 ----------
mkdir -p "${PKG_DIR}/usr/share/doc/sidera"
cp ../README.classroom.md "${PKG_DIR}/usr/share/doc/sidera/README.classroom.md"
cp ../LICENSE "${PKG_DIR}/usr/share/doc/sidera/copyright"
cp ../LICENSE "${PKG_DIR}/usr/share/doc/sidera/LICENSE"

# ---------- DEBIAN/control ----------
cat > "${PKG_DIR}/DEBIAN/control" << EOF
Package: ${PKG_NAME}
Version: ${VERSION}
Section: graphics
Priority: optional
Architecture: ${ARCH}
Maintainer: User <user@localhost>
Depends: ${DEPENDS}
Recommends: fonts-noto-cjk
Description: Sidera 软件
 全屏透明画布批注工具，支持画笔/橡皮擦与白板，触控屏友好。
 启动后显示左右两个胶囊形侧边栏，侧边栏 ⛶ 按钮可退出全屏。
 附带 WPS 演示联动加载项：设置里开启“WPS接口调试”后，
 批注缓存随真实换页驱动（页内动画不动批注）。
 安装后执行一次 sidera-wps-addin-install 注册加载项。
EOF

# ---------- desktop 入口文件 ----------
ICON_SHA="$(sha256sum ../sidera.png | cut -c1-12)"
ICON_PATH="/usr/share/pixmaps/sidera-${ICON_SHA}.png"
cat > "${PKG_DIR}/usr/share/applications/sidera.desktop" << EOF
[Desktop Entry]
Type=Application
Name=Sidera
Comment=Sidera软件
Exec=sidera
Icon=${ICON_PATH}
Categories=Graphics;Utility;
Terminal=false
EOF

# ---------- 图标：sidera.svg（scalable + pixmaps 绝对路径）+ sidera.png（256px） ----------
mkdir -p "${PKG_DIR}/usr/share/icons/hicolor/scalable/apps"
mkdir -p "${PKG_DIR}/usr/share/icons/hicolor/256x256/apps"
mkdir -p "${PKG_DIR}/usr/share/pixmaps"
cp ../sidera.svg "${PKG_DIR}/usr/share/icons/hicolor/scalable/apps/sidera.svg"
cp ../sidera.svg "${PKG_DIR}/usr/share/pixmaps/sidera.svg"
cp ../sidera.png "${PKG_DIR}/usr/share/icons/hicolor/256x256/apps/sidera.png"
cp ../sidera.png "${PKG_DIR}/usr/share/pixmaps/sidera.png"
cp ../sidera.png "${PKG_DIR}${ICON_PATH}"

# ---------- 打包（--root-owner-group 消除 owner 警告）----------
dpkg-deb --build --root-owner-group "$PKG_DIR" "$PKG_DIR.deb"
mv -f -- "$PKG_DIR.deb" "$OUTPUT"

echo ""
echo "========== 打包完成 =========="
ls -lh "$OUTPUT"
echo ""
echo "安装: sudo apt install $OUTPUT"
echo "卸载: sudo dpkg -r ${PKG_NAME}"
