#!/usr/bin/env bash
# ============================================================
# 启动 Windows 10 Pro 验收虚拟机（dockur/windows）
#
#   ./run-win10.sh
#
#   浏览器打开 http://127.0.0.1:8006
#   dist/ 会映射为 Windows 的 Z: 盘（桌面 Shared）
#   默认用户 Docker / 密码 admin，可用 RDP 连 127.0.0.1:3389
# ============================================================
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
STORAGE="${SIDERA_WIN10_STORAGE:-$HOME/win10}"
IMAGE="${SIDERA_WIN10_IMAGE:-ghcr.io/dockur/windows:6.05}"
VERSION="${SIDERA_WIN10_VERSION:-10}"

if [ ! -e /dev/kvm ]; then
  echo "错误: 没有 /dev/kvm，无法启用 KVM 加速" >&2
  exit 1
fi

mkdir -p "$STORAGE" "$HERE/dist"

echo "==> 镜像: $IMAGE"
echo "==> 存储: $STORAGE"
echo "==> 共享: $HERE/dist -> Windows Z:"
echo "==> 安装完成后打开 http://127.0.0.1:8006"

exec docker run -it --rm --name sidera-win10 \
  --device=/dev/kvm \
  --device=/dev/net/tun \
  --cap-add NET_ADMIN \
  -p 8006:8006 \
  -p 3389:3389/tcp \
  -p 3389:3389/udp \
  -v "$STORAGE:/storage" \
  -v "$HERE/dist:/shared" \
  -e "VERSION=$VERSION" \
  -e LANGUAGE=Chinese \
  -e REGION=zh-CN \
  -e KEYBOARD=zh-CN \
  -e RAM_SIZE="${SIDERA_WIN10_RAM:-8G}" \
  -e CPU_CORES="${SIDERA_WIN10_CPU:-4}" \
  --stop-timeout 120 \
  "$IMAGE"
