#!/bin/bash
# ============================================================
# Rust 版 Sidera — aarch64 (arm64) 编译 / 打包脚本
#
#   ./build_aarch64.sh            编译 + 打包 DEB/RPM/Arch
#   ./build_aarch64.sh --no-pkg   只编译，不打包
#
# 编译方式：在 amd64-sysroot 容器里用 x86_64 rustc 交叉编译到
#           aarch64-unknown-linux-gnu；链接使用 rustup 自带的 ld.lld，
#           目标库来自 aarch64-sysroot，产物保持低 glibc 基线。
#           不再用 qemu 运行 aarch64 rustc，编译速度接近 amd64。
#
# 依赖：
#   - bwrap
#   - rustup 工具链 1.93.1-x86_64-unknown-linux-gnu
#   - 该工具链的 aarch64 标准库：
#       rustup target add --toolchain 1.93.1 aarch64-unknown-linux-gnu
#   - 仓库根 amd64-sysroot/（编译容器）与 aarch64-sysroot/（目标库）
#
# 产物：
#   target/container-aarch64/release/sidera
#   <repo>/sidera_3.2-Electro-stable_arm64.deb
#   rust/dist/sidera-3.2-Electro.stable.aarch64.rpm
#   rust/dist/sidera-3.2_electro_stable-1-aarch64.pkg.tar.zst
#
# 注意：本脚本会先交叉编译再打包。只跑 build_deb.sh / build_rpm.sh /
#       build_arch.sh 不会重新编译，只会把 target/container-aarch64 里的
#       旧二进制重新封装，得到“新日期、旧内容”的包。
# ============================================================
set -e

HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE"

TOOLCHAIN="${SIDERA_RUST_TOOLCHAIN:-1.93.1}"
HOST_TRIPLE=x86_64-unknown-linux-gnu
REPO="$(cd "$HERE/.." && pwd)"

# Prefer whichever rustup already has the aarch64 target std; RUSTUP_HOME wins.
has_target_std() {
  [ -d "$1/toolchains/$TOOLCHAIN-$HOST_TRIPLE/lib/rustlib/aarch64-unknown-linux-gnu/lib" ]
}
if [ -n "${RUSTUP_HOME:-}" ]; then
  RUSTUP_HOME_EFFECTIVE="$RUSTUP_HOME"
elif has_target_std "$REPO/.toolchains/rustup"; then
  RUSTUP_HOME_EFFECTIVE="$REPO/.toolchains/rustup"
elif has_target_std "$HOME/.rustup"; then
  RUSTUP_HOME_EFFECTIVE="$HOME/.rustup"
elif [ -d "$REPO/.toolchains/rustup/toolchains/$TOOLCHAIN-$HOST_TRIPLE" ]; then
  RUSTUP_HOME_EFFECTIVE="$REPO/.toolchains/rustup"
else
  RUSTUP_HOME_EFFECTIVE="$HOME/.rustup"
fi

NO_PKG=0
if [ "$1" = "--no-pkg" ] || [ "$1" = "--no-deb" ]; then
  NO_PKG=1
fi

# ---------- 依赖检查 ----------
miss=0
command -v bwrap >/dev/null 2>&1 || { echo "缺少 bwrap"; miss=1; }
if [ ! -d "$HERE/../amd64-sysroot" ]; then
  echo "缺少 $HERE/../amd64-sysroot（交叉编译容器）"
  miss=1
fi
if [ ! -d "$HERE/../aarch64-sysroot" ]; then
  echo "缺少 $HERE/../aarch64-sysroot（目标库）"
  miss=1
fi
if [ ! -d "$RUSTUP_HOME_EFFECTIVE/toolchains/$TOOLCHAIN-$HOST_TRIPLE" ]; then
  echo "缺少 Rust 工具链 $TOOLCHAIN-$HOST_TRIPLE（RUSTUP_HOME=$RUSTUP_HOME_EFFECTIVE）"
  echo "  rustup toolchain install $TOOLCHAIN-$HOST_TRIPLE --profile minimal"
  miss=1
elif [ ! -d "$RUSTUP_HOME_EFFECTIVE/toolchains/$TOOLCHAIN-$HOST_TRIPLE/lib/rustlib/aarch64-unknown-linux-gnu/lib" ]; then
  echo "缺少 aarch64-unknown-linux-gnu 标准库（RUSTUP_HOME=$RUSTUP_HOME_EFFECTIVE）"
  echo "  rustup target add --toolchain $TOOLCHAIN aarch64-unknown-linux-gnu"
  miss=1
fi
if [ "$miss" -ne 0 ]; then
  echo "依赖不满足，退出。"
  exit 1
fi

# ---------- 编译 ----------
echo "==> 编译 aarch64（amd64 容器交叉编译）..."
./build_container.sh aarch64

# ---------- 打包（从容器编译好的二进制，主机打包）----------
if [ "$NO_PKG" -eq 0 ]; then
  DEB_FALLBACK_DEPENDS="libc6 (>= 2.30), libgcc-s1, python3"
  echo "==> 打包 arm64：DEB..."
  # build_deb.sh 内部已能自动回退依赖；这里再显式重试一次，兼容旧版脚本。
  if ! ./build_deb.sh aarch64; then
    echo "==> DEB 打包失败，改用固定运行依赖重试..."
    if ! SIDERA_DEB_DEPENDS="$DEB_FALLBACK_DEPENDS" ./build_deb.sh aarch64; then
      echo "错误: DEB 打包失败，已中止。" >&2
      echo "      不继续打包 RPM/Arch，避免留下“只有其他格式更新”的半新产物。" >&2
      exit 1
    fi
  fi
  echo "==> 打包 arm64：RPM..."
  ./build_rpm.sh aarch64
  echo "==> 打包 arm64：Arch..."
  ./build_arch.sh aarch64

  echo ""
  echo "========== 产物 =========="
  for f in "../sidera_3.2-Electro-stable_arm64.deb" \
           dist/sidera-3.2-Electro.stable.aarch64.rpm \
           dist/sidera-3.2_electro_stable-1-aarch64.pkg.tar.zst; do
    [ -f "$f" ] && ls -lh "$f"
  done
else
  echo "==> 已跳过打包（--no-pkg）"
fi

echo "==> 完成。"
