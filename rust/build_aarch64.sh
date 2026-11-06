#!/bin/bash
# ============================================================
# Rust 版 Sidera — aarch64 (arm64) 编译 / 打包脚本
#
#   ./build_aarch64.sh            编译 + 打包 DEB/RPM/Arch
#   ./build_aarch64.sh --no-pkg   只编译，不打包
#
# 编译方式：复用 build_container.sh，在仓库内 aarch64-sysroot 容器里
#          以 qemu-aarch64-static 运行，产物仅需较低 glibc，可跑在老机器。
#
# 依赖：
#   - bwrap
#   - qemu-aarch64-static（需已注册 binfmt）
#   - rustup 工具链 stable-aarch64-unknown-linux-gnu：
#       rustup toolchain install stable-aarch64-unknown-linux-gnu \
#              --profile minimal --force-non-host
#
# 产物：
#   target/container-aarch64/release/sidera
#   ../sidera_3.1-Electro-stable_arm64.deb（打包时）
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
command -v qemu-aarch64-static >/dev/null 2>&1 || { echo "缺少 qemu-aarch64-static"; miss=1; }
if [ ! -d "$HOME/.rustup/toolchains/stable-aarch64-unknown-linux-gnu" ]; then
  echo "缺少 Rust 工具链 stable-aarch64-unknown-linux-gnu"
  echo "  rustup toolchain install stable-aarch64-unknown-linux-gnu --profile minimal --force-non-host"
  miss=1
fi
if [ ! -d "$HERE/../aarch64-sysroot" ]; then
  echo "缺少 $HERE/../aarch64-sysroot"
  miss=1
fi
if [ "$miss" -ne 0 ]; then
  echo "依赖不满足，退出。"
  exit 1
fi

# ---------- 编译 ----------
echo "==> 编译 aarch64（容器方式）..."
./build_container.sh aarch64

# ---------- 打包（从容器编译好的二进制，主机打包）----------
if [ "$NO_PKG" -eq 0 ]; then
  echo "==> 打包 arm64：DEB..."
  if ! ./build_deb.sh aarch64; then
    echo "==> 自动依赖计算失败（可能不是 Debian/Ubuntu 主机），改用固定依赖重试 DEB..."
    SIDERA_DEB_DEPENDS="libc6 (>= 2.30), libgcc-s1, python3" ./build_deb.sh aarch64
  fi
  echo "==> 打包 arm64：RPM..."
  ./build_rpm.sh aarch64
  echo "==> 打包 arm64：Arch..."
  ./build_arch.sh aarch64
else
  echo "==> 已跳过打包（--no-pkg）"
fi

echo "==> 完成。"
