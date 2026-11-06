#!/bin/bash
# ============================================================
# Rust 版 Sidera — amd64 (x86_64) 编译 + 打包（一步到位）
#
#   ./build_amd64.sh            编译 + 打包 DEB/RPM/Arch
#   ./build_amd64.sh --no-pkg   只编译，不打包
#
# 依赖：
#   - bwrap + rustup 工具链 stable-x86_64-unknown-linux-gnu + 仓库根 amd64-sysroot/
#   - 打包工具：dpkg-deb / rpmbuild / bsdtar+zstd+fakeroot
#
# 产物：
#   target/container-amd64/release/sidera（低 glibc 二进制）
#   sidera_3.1-Electro-stable_amd64.deb / sidera-3.1-Electro_stable.x86_64.rpm
#   sidera-3.1_electro_stable-1-x86_64.pkg.tar.zst（rust/dist/）
# ============================================================
set -e

HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE"

NO_PKG=0
if [ "$1" = "--no-pkg" ] || [ "$1" = "--no-deb" ]; then
  NO_PKG=1
fi

# ---------- 依赖检查 ----------
miss=0
command -v bwrap >/dev/null 2>&1 || { echo "缺少 bwrap"; miss=1; }
if [ ! -d "$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu" ]; then
  echo "缺少 Rust 工具链 stable-x86_64-unknown-linux-gnu"
  echo "  rustup toolchain install stable-x86_64-unknown-linux-gnu --profile minimal"
  miss=1
fi
if [ ! -d "$HERE/../amd64-sysroot" ]; then
  echo "缺少 $HERE/../amd64-sysroot"
  miss=1
fi
if [ "$miss" -ne 0 ]; then
  echo "依赖不满足，退出。"
  exit 1
fi

# ---------- 编译 ----------
echo "==> 编译 amd64（容器方式）..."
./build_container.sh amd64

# ---------- 打包（从容器编译好的二进制，主机打包）----------
if [ "$NO_PKG" -eq 0 ]; then
  echo "==> 打包 amd64：DEB..."
  if ! ./build_deb.sh amd64; then
    echo "==> 自动依赖计算失败（可能不是 Debian/Ubuntu 主机），改用固定依赖重试 DEB..."
    SIDERA_DEB_DEPENDS="libc6 (>= 2.30), libgcc-s1, python3" ./build_deb.sh amd64
  fi
  echo "==> 打包 amd64：RPM..."
  ./build_rpm.sh amd64
  echo "==> 打包 amd64：Arch..."
  ./build_arch.sh amd64
else
  echo "==> 已跳过打包（--no-pkg）"
fi

echo "==> 完成。"
