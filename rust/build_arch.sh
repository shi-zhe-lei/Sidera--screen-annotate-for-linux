#!/bin/bash
# ============================================================
# Rust 版 Sidera — 生成 Arch 包 (.pkg.tar.zst，主机打包，二进制来自容器编译)
#
#   ./build_arch.sh [amd64|aarch64]     # 默认 amd64
#
# 依赖：bsdtar（libarchive）、zstd、fakeroot
# 产物：rust/dist/sidera-<ver>-<rel>-<arch>.pkg.tar.zst
#
# 说明：不重新编译；手工组装 .PKGINFO/.MTREE/.INSTALL + 文件树，
#       与 makepkg 产物格式一致。Arch 的源码 PKGBUILD 见 arch/（供 AUR 用户）。
# ============================================================
set -e

HERE="$(cd "$(dirname "$0")" && pwd)"
source "$HERE/pkg_common.sh"

ARCH="${1:-amd64}"
pkg_parse_arch "$ARCH" || exit 1

BIN="$(pkg_find_bin)"
[ -f "$BIN" ] || { echo "错误: 找不到二进制 $BIN，请先 ./build_container.sh $SRC_ARCH"; exit 1; }
command -v bsdtar >/dev/null || { echo "错误: 缺少 bsdtar"; exit 1; }

VER=3.1_electro_stable
REL=1
DIST="$HERE/dist"
mkdir -p "$DIST"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
STAGE="$WORK/root"
stage_files "$BIN" "$STAGE"

# .PKGINFO
SIZE=$(find "$STAGE" -type f -printf '%s\n' | awk '{s+=$1} END{print s+0}')
cat > "$STAGE/.PKGINFO" <<EOF
pkgname = sidera
pkgbase = sidera
pkgver = ${VER}-${REL}
pkgdesc = Linux 屏幕批注与教学演示工具（Rust，X11/Wayland，WPS 联动）
url = https://github.com/offduckling4825/Sidera--screen-annotate-for-linux
builddate = $(date +%s)
packager = Carl_Jin
size = ${SIZE}
arch = ${PKG_ARCH}
license = GPL-3.0-only
depend = glibc
depend = gcc-libs
depend = python
EOF

# .INSTALL
cat > "$STAGE/.INSTALL" <<'EOF'
_sidera_cleanup_old_addin() {
  # 只清理旧名称/同一本地桥的旧条目，保留当前 sidera-bridge。
  for home in /root /home/*; do
    [ -d "$home" ] || continue
    [ -x /usr/bin/sidera-wps-addin-cleanup ] || continue
    HOME="$home" /usr/bin/sidera-wps-addin-cleanup "$@" >/dev/null 2>&1 || true
  done
}
post_install() {
  udevadm control --reload-rules 2>/dev/null || true
  udevadm trigger --subsystem-match=input 2>/dev/null || true
  modprobe uinput 2>/dev/null || true
  _sidera_cleanup_old_addin
  gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor 2>/dev/null || true
  update-desktop-database -q /usr/share/applications 2>/dev/null || true
  echo ">>> Sidera: 触摸权限已配置，若刚安装请重新登录后再用触屏批注。"
}
post_upgrade() {
  post_install
}
pre_remove() {
  _sidera_cleanup_old_addin --remove-current
}

post_remove() {
  udevadm control --reload-rules 2>/dev/null || true
}
EOF

# .MTREE（用 bsdtar 生成，与 makepkg 一致）
( cd "$STAGE" && bsdtar -czf .MTREE --format=mtree \
    --options='!all,use-set,type,uid,gid,mode,time,size,md5,sha256,link' \
    --exclude .MTREE . )

OUT="$DIST/sidera-${VER}-${REL}-${PKG_ARCH}.pkg.tar.zst"
echo "==> 打包 Arch 包 ($PKG_ARCH)..."
( cd "$STAGE" && fakeroot -- bsdtar -c --zstd -f "$OUT" .PKGINFO .MTREE .INSTALL usr etc )

echo "==> 完成，产物："
ls -lh "$OUT"
