#!/bin/bash
# ============================================================
# Rust 版 Sidera DEB 打包脚本
#   用法: ./build_deb.sh [amd64|aarch64]
#   产物: <repo>/sidera_3.1-Electro-stable_<amd64|arm64>.deb
#
# 依赖：dpkg-dev、binutils、python3；必须能解析目标架构的系统库
# 前置：先用 ./build_container.sh <arch> 编译出二进制
#       （或普通 cargo build --release，用架构匹配的 target/release 兜底）
#
# 与 C++ 版打包脚本的区别：
#   - 二进制来自 Rust 构建（target/container-<arch>/release/sidera）
#   - 运行依赖从实际 ELF 推导，不硬编码最低 glibc
# ============================================================
set -e

ARG="${1:-amd64}"
HERE="$(cd "$(dirname "$0")" && pwd)"      # <repo>/rust
REPO="$(cd "$HERE/.." && pwd)"             # <repo>

case "$ARG" in
  amd64)        SRC_ARCH=amd64;  DEB_ARCH=amd64; ELF_MACHINE='Advanced Micro Devices X86-64';;
  aarch64|arm64) SRC_ARCH=aarch64; DEB_ARCH=arm64; ELF_MACHINE='AArch64';;
  *) echo "用法: $0 [amd64|aarch64]"; exit 1;;
esac

PKG_NAME="sidera"
# 注意：dpkg 要求 Version 以数字开头，故采用 "3.1-Electro-stable"；
# 如需改版本只改这里即可。
VERSION="${SIDERA_VERSION:-3.1-Electro-stable}"
dpkg --validate-version "$VERSION" || exit 1
[[ "$VERSION" != */* && "$VERSION" != *[[:space:]]* ]] || exit 1

# 优先容器构建产物，其次普通 release
BIN="$HERE/target/container-$SRC_ARCH/release/sidera"
[ -f "$BIN" ] || BIN="$HERE/target/release/sidera"
if [ ! -f "$BIN" ]; then
  echo "错误: 找不到 $SRC_ARCH 二进制，请先运行: ./build_container.sh $ARG"
  exit 1
fi

# 校验实际 ELF，不依赖文件名或宿主架构；失败时保留已有包和暂存目录。
if ! command -v readelf >/dev/null 2>&1; then
  echo "错误: 缺少 readelf，请安装 binutils 后再打包。" >&2
  exit 1
fi
if ! ELF_HEADER="$(LC_ALL=C readelf -h "$BIN" 2>/dev/null)"; then
  echo "错误: 无法读取 ELF 头，不能打包: $BIN" >&2
  exit 1
fi
if ! LC_ALL=C awk -v expected="$ELF_MACHINE" '
  $1 == "Class:" { elf_class = $2 }
  $1 == "Data:" { little_endian = /little endian/ }
  $1 == "Type:" { elf_type = $2 }
  $1 == "Machine:" {
    sub(/^[[:space:]]*Machine:[[:space:]]*/, "")
    sub(/[[:space:]]*$/, "")
    machine = $0
  }
  END {
    exit !(elf_class == "ELF64" && little_endian && machine == expected &&
           (elf_type == "EXEC" || elf_type == "DYN"))
  }
' <<< "$ELF_HEADER"; then
  echo "错误: $BIN 与目标 $DEB_ARCH 不匹配；需要 64 位小端 $ELF_MACHINE 可执行文件。" >&2
  printf '%s\n' "$ELF_HEADER" >&2
  exit 1
fi

if [ -n "${SIDERA_DEB_DEPENDS:-}" ]; then
  # 显式指定依赖（用于非 Debian/Ubuntu 主机做“容器编译 + 主机打包”）
  DEPENDS="$SIDERA_DEB_DEPENDS"
else
  # 严格模式：ELF 不合法 / 依赖无法确定时直接失败（不静默回退）
  DEPENDS="$(python3 "$REPO/tools/deb_dependencies.py" "$BIN" "$DEB_ARCH")"
fi
OUTPUT="$REPO/${PKG_NAME}_${VERSION}_${DEB_ARCH}.deb"
PKG_DIR="$(mktemp -d "$REPO/.sidera-package.XXXXXXXX")"
trap 'rm -rf -- "$PKG_DIR"; rm -f -- "$PKG_DIR.deb"' EXIT
echo "==> 打包 $BIN -> $OUTPUT"

mkdir -p "$PKG_DIR/DEBIAN" \
         "$PKG_DIR/usr/bin" \
         "$PKG_DIR/usr/share/applications" \
         "$PKG_DIR/usr/share/sidera/wps-addin" \
         "$PKG_DIR/usr/share/doc/sidera" \
         "$PKG_DIR/usr/share/icons/hicolor/scalable/apps" \
         "$PKG_DIR/usr/share/icons/hicolor/256x256/apps" \
         "$PKG_DIR/usr/share/pixmaps"

# ---------- 二进制 ----------
install -m 755 "$BIN" "$PKG_DIR/usr/bin/sidera"

# ---------- systemd 用户服务（由应用在启用进程屏蔽时启动，不默认 enable）----------
mkdir -p "$PKG_DIR/usr/lib/systemd/user"
install -m 644 "$REPO/rust/systemd/sidera-blocker.service" \
  "$PKG_DIR/usr/lib/systemd/user/sidera-blocker.service"

# ---------- WPS 加载项（放到固定路径，Rust 版会自动从 /usr/share/sidera/wps-addin 读取）----------
cp -r "$REPO/wps-addin/." "$PKG_DIR/usr/share/sidera/wps-addin/"
chmod 755 "$PKG_DIR/usr/share/sidera/wps-addin/install.sh" \
          "$PKG_DIR/usr/share/sidera/wps-addin/cleanup-old.sh"
install -m 755 "$REPO/wps-addin/install.sh" "$PKG_DIR/usr/bin/sidera-wps-addin-install"
install -m 755 "$REPO/wps-addin/cleanup-old.sh" "$PKG_DIR/usr/bin/sidera-wps-addin-cleanup"

# GNOME's native overlay needs a per-user Shell extension. Package its source,
# but do not enable it or restart sessions from the root post-install script.
mkdir -p "$PKG_DIR/usr/share/sidera/gnome-extension"
cp -r "$REPO/compat/gnome-extension/." "$PKG_DIR/usr/share/sidera/gnome-extension/"
install -m 755 "$REPO/compat/gnome-extension/install.py" "$PKG_DIR/usr/bin/sidera-gnome-install"
install -m 644 "$REPO/compat/gnome-extension/sidera-gnome.desktop" "$PKG_DIR/usr/share/applications/"
install -m 644 "$REPO/compat/gnome-extension/sidera-compat.desktop" "$PKG_DIR/usr/share/applications/"

# ---------- 文档 ----------
cp "$REPO/README.classroom.md" "$PKG_DIR/usr/share/doc/sidera/README.classroom.md"
cp "$REPO/LICENSE" "$PKG_DIR/usr/share/doc/sidera/copyright"
cp "$REPO/LICENSE" "$PKG_DIR/usr/share/doc/sidera/LICENSE"

# ---------- 图标 ----------
cp "$REPO/sidera.svg" "$PKG_DIR/usr/share/icons/hicolor/scalable/apps/sidera.svg"
cp "$REPO/sidera.svg" "$PKG_DIR/usr/share/pixmaps/sidera.svg"
cp "$REPO/sidera.png" "$PKG_DIR/usr/share/icons/hicolor/256x256/apps/sidera.png"
cp "$REPO/sidera.png" "$PKG_DIR/usr/share/pixmaps/sidera.png"

# GNOME may retain a texture for an absolute icon path after its file changes.
# A content-specific path lets the desktop-entry update invalidate that texture.
# Keep the conventional names above for autostart entries and in-app artwork.
ICON_SHA="$(sha256sum "$REPO/sidera.png")"
ICON_PATH="/usr/share/pixmaps/sidera-${ICON_SHA:0:12}.png"
cp "$REPO/sidera.png" "$PKG_DIR$ICON_PATH"

# ---------- uinput 权限（KWin 等无虚拟键盘协议时，用内核虚拟键盘发翻页键）----------
mkdir -p "$PKG_DIR/lib/udev/rules.d" "$PKG_DIR/etc/modules-load.d"
cat > "$PKG_DIR/lib/udev/rules.d/70-sidera-uinput.rules" << 'EOF'
# Sidera: 允许当前登录用户访问 /dev/uinput（用于注入翻页/退出按键）
KERNEL=="uinput", TAG+="uaccess", OPTIONS+="static_node=uinput"
EOF
cat > "$PKG_DIR/lib/udev/rules.d/70-sidera-input.rules" << 'EOF'
# Sidera: 允许当前登录用户读取输入设备（手背橡皮/多点触摸需要读 evdev）
KERNEL=="event*", SUBSYSTEM=="input", TAG+="uaccess"
EOF
echo "uinput" > "$PKG_DIR/etc/modules-load.d/sidera-uinput.conf"

cat > "$PKG_DIR/DEBIAN/postinst" << 'EOF'
#!/bin/sh
set -e
modprobe uinput 2>/dev/null || true
udevadm control --reload-rules 2>/dev/null || true
udevadm trigger --name-match=uinput 2>/dev/null || true
udevadm trigger --subsystem-match=input 2>/dev/null || true
# 删除旧版 WPS 加载项登记/授权；只清理已知旧名称和同一本地桥，保留当前条目。
# 逐用户执行并保留文件原所有者；失败不阻断安装，应用启动时还会再清理一次。
for home in /root /home/*; do
  [ -d "$home" ] || continue
  [ -x /usr/bin/sidera-wps-addin-cleanup ] || continue
  HOME="$home" /usr/bin/sidera-wps-addin-cleanup >/dev/null 2>&1 || true
done
gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor 2>/dev/null || true
update-desktop-database -q /usr/share/applications 2>/dev/null || true
exit 0
EOF
chmod 755 "$PKG_DIR/DEBIAN/postinst"

cat > "$PKG_DIR/DEBIAN/prerm" << 'EOF'
#!/bin/sh
set -e
if [ "$1" = remove ] && [ -x /usr/bin/sidera-wps-addin-cleanup ]; then
  for home in /root /home/*; do
    [ -d "$home" ] || continue
    HOME="$home" /usr/bin/sidera-wps-addin-cleanup --remove-current >/dev/null 2>&1 || true
  done
fi
exit 0
EOF
chmod 755 "$PKG_DIR/DEBIAN/prerm"

# ---------- DEBIAN/control ----------
cat > "$PKG_DIR/DEBIAN/control" << EOF
Package: ${PKG_NAME}
Version: ${VERSION}
Section: graphics
Priority: optional
Architecture: ${DEB_ARCH}
Maintainer: User <user@localhost>
Depends: ${DEPENDS}
Recommends: fonts-noto-cjk
Suggests: gnome-shell (>= 46), libglib2.0-bin, xdg-desktop-portal
Description: Sidera (Rust) 屏幕批注工具
 纯 Rust 实现，不依赖 Qt / libX11；加载项安装工具使用 Python 3。
 全屏透明画布批注，支持画笔/橡皮/白板，触控屏友好。
 内置 WPS 演示联动（本地 HTTP 127.0.0.1:16666）。
 安装后执行一次 sidera-wps-addin-install 注册加载项。
EOF

# ---------- desktop 入口 ----------
cat > "$PKG_DIR/usr/share/applications/sidera.desktop" << EOF
[Desktop Entry]
Type=Application
Name=Sidera
Comment=Sidera软件
Exec=sidera
Icon=$ICON_PATH
Categories=Graphics;Utility;
Terminal=false
EOF

# All launch modes must use the same refreshed icon, with stable desktop IDs.
for ENTRY in sidera-gnome.desktop sidera-compat.desktop; do
  sed -i "s|^Icon=.*|Icon=$ICON_PATH|" "$PKG_DIR/usr/share/applications/$ENTRY"
done

# ---------- 打包 ----------
dpkg-deb --build --root-owner-group "$PKG_DIR" "$PKG_DIR.deb"
mv -f -- "$PKG_DIR.deb" "$OUTPUT"

echo ""
echo "========== 打包完成 =========="
ls -lh "$OUTPUT"
echo ""
echo "安装: sudo apt install $OUTPUT"
echo "卸载: sudo dpkg -r ${PKG_NAME}"
