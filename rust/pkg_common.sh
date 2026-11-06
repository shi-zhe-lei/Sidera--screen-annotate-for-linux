#!/bin/bash
# ============================================================
# 打包公共：把运行所需文件按布局安装到 $2（二进制取 $1）
# 供 build_rpm.sh / build_arch.sh 复用；DEB 仍走 build_deb.sh。
# ============================================================
_rpkg_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"   # .../rust
REPO="$(cd "$_rpkg_dir/.." && pwd)"                          # 仓库根

# stage_files <binary> <destroot>
stage_files() {
  local bin="$1" root="$2"

  install -Dm755 "$bin" "$root/usr/bin/sidera"
  install -Dm644 "$REPO/rust/systemd/sidera-blocker.service" "$root/usr/lib/systemd/user/sidera-blocker.service"
  install -Dm755 "$REPO/wps-addin/install.sh" "$root/usr/bin/sidera-wps-addin-install"
  install -Dm755 "$REPO/wps-addin/cleanup-old.sh" "$root/usr/bin/sidera-wps-addin-cleanup"

  install -d "$root/usr/share/sidera/wps-addin"
  cp -a "$REPO/wps-addin/." "$root/usr/share/sidera/wps-addin/"

  install -Dm644 "$REPO/LICENSE" "$root/usr/share/licenses/sidera/LICENSE"
  install -Dm644 "$REPO/README.classroom.md" "$root/usr/share/doc/sidera/README.classroom.md"

  local icon_sha icon_path
  icon_sha="$(sha256sum "$REPO/sidera.png" | cut -c1-12)"
  icon_path="/usr/share/pixmaps/sidera-$icon_sha.png"
  install -Dm644 "$REPO/sidera.svg" "$root/usr/share/icons/hicolor/scalable/apps/sidera.svg"
  install -Dm644 "$REPO/sidera.png" "$root/usr/share/icons/hicolor/256x256/apps/sidera.png"
  install -Dm644 "$REPO/sidera.svg" "$root/usr/share/pixmaps/sidera.svg"
  install -Dm644 "$REPO/sidera.png" "$root/usr/share/pixmaps/sidera.png"
  # Content-addressed path invalidates stale desktop icon caches after upgrades.
  install -Dm644 "$REPO/sidera.png" "$root$icon_path"

  install -Dm644 "$REPO/rust/arch/70-sidera-uinput.rules" "$root/usr/lib/udev/rules.d/70-sidera-uinput.rules"
  install -Dm644 "$REPO/rust/arch/70-sidera-input.rules" "$root/usr/lib/udev/rules.d/70-sidera-input.rules"
  install -d "$root/etc/modules-load.d"
  echo uinput > "$root/etc/modules-load.d/sidera-uinput.conf"

  install -d "$root/usr/share/applications"
  cat > "$root/usr/share/applications/sidera.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Sidera
Comment=Sidera 屏幕批注
Exec=sidera
Icon=$icon_path
Categories=Graphics;Utility;
Terminal=false
EOF

  # Native GNOME and X11/XWayland compatibility entries, matching the DEB.
  install -m 644 "$REPO/compat/gnome-extension/sidera-gnome.desktop" "$root/usr/share/applications/"
  install -m 644 "$REPO/compat/gnome-extension/sidera-compat.desktop" "$root/usr/share/applications/"
  sed -i "s|^Icon=.*|Icon=$icon_path|" \
    "$root/usr/share/applications/sidera-gnome.desktop" \
    "$root/usr/share/applications/sidera-compat.desktop"
  install -d "$root/usr/share/sidera/gnome-extension"
  cp -a "$REPO/compat/gnome-extension/." "$root/usr/share/sidera/gnome-extension/"
  install -Dm755 "$REPO/compat/gnome-extension/install.py" "$root/usr/bin/sidera-gnome-install"
}

# 解析架构参数 -> 容器产物架构 / 包架构
# 用法: pkg_parse_arch <amd64|aarch64>
pkg_parse_arch() {
  case "$1" in
    amd64)   SRC_ARCH=amd64;   PKG_ARCH=x86_64;  RPM_ARCH=x86_64;;
    aarch64) SRC_ARCH=aarch64; PKG_ARCH=aarch64; RPM_ARCH=aarch64;;
    *) echo "用法: <amd64|aarch64>"; return 1;;
  esac
}

# 找到容器构建出来的二进制（找不到则退回本机 release）
pkg_find_bin() {
  local bin="$REPO/rust/target/container-$SRC_ARCH/release/sidera"
  [ -f "$bin" ] || bin="$REPO/rust/target/release/sidera"
  printf '%s' "$bin"
}
