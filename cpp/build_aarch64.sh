#!/bin/bash
# ============================================================
# C++ / Qt5 版 Sidera — aarch64 (arm64) 编译 + 打包（一步到位）
#
#   ./build_aarch64.sh
#
# 依赖：bwrap + qemu-aarch64-static（已注册 binfmt）
#       + 仓库根 aarch64-sysroot/（容器内含 gcc-9 + Qt5.12）
# 产物：annotate_aarch64、sidera_3.2-Electro-stable~beta1_arm64.deb
# ============================================================
set -e
cd "$(dirname "$0")"

./build_container.sh aarch64
./build_deb_aarch64.sh

echo "==> 完成。"
