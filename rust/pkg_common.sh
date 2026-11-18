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

  if [ -f "$REPO/rust/assets/fonts/NotoSansCJKsc-Regular.otf" ]; then
    install -Dm644 "$REPO/rust/assets/fonts/NotoSansCJKsc-Regular.otf" "$root/usr/share/sidera/fonts/NotoSansCJKsc-Regular.otf"
    install -Dm644 "$REPO/rust/assets/fonts/LICENSE-OFL.txt" "$root/usr/share/sidera/fonts/LICENSE-OFL.txt"
  fi

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
#
# 交叉编译（--target）时 cargo 把产物放在 <target-dir>/<triple>/release/，
# 而不是 <target-dir>/release/。aarch64 必须先看 triple 目录，否则会误用
# 早期 qemu 构建残留下来的旧二进制（产物日期看起来很新，内容却是旧的）。
pkg_find_bin() {
  local triple
  case "$SRC_ARCH" in
    aarch64) triple=aarch64-unknown-linux-gnu;;
    *)       triple=x86_64-unknown-linux-gnu;;
  esac
  local base="$REPO/rust/target/container-$SRC_ARCH"
  local bin
  for bin in "$base/$triple/release/sidera" "$base/release/sidera" "$REPO/rust/target/release/sidera"; do
    if [ -f "$bin" ]; then
      printf '%s' "$bin"
      return 0
    fi
  done
}
