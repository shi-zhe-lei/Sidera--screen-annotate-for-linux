#!/bin/bash
# ============================================================
# C++ / Qt5 版 Sidera — amd64 编译 + 打包（一步到位）
#
#   ./build_amd64.sh
#
# 依赖：bwrap + 仓库根 amd64-sysroot/（容器内含 gcc-9 + Qt5.12）
# 产物：annotate_amd64、sidera_3.2-Electro-stable~beta1_amd64.deb
# ============================================================
set -e
cd "$(dirname "$0")"

./build_container.sh amd64
./build_deb_amd64.sh

echo "==> 完成。"
