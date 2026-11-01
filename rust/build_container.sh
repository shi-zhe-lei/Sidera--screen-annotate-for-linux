#!/bin/bash
# ============================================================
# 在仓库内的 Ubuntu 20.04 rootfs 容器里编译 Rust 版 Sidera
#   ./build_container.sh amd64      -> rust/target/container-amd64/release/sidera
#   ./build_container.sh aarch64    -> rust/target/container-aarch64/release/sidera
#
# 原理：sysroot 里没有 Rust，且 root 只读，无法安装。所以改成
#   把宿主机的 rustup 工具链 bind 进容器，在容器（glibc 2.31）里链接，
#   最低运行 ABI 以实际产物的 readelf/dpkg-shlibdeps 结果为准。
#
# 依赖：
#   - bwrap（免 root）
#   - amd64:   rustup 工具链 1.93.1-x86_64-unknown-linux-gnu
#   - aarch64: 宿主机 qemu-aarch64-static + 已注册 binfmt + 工具链
#                rustup toolchain install 1.93.1-aarch64-unknown-linux-gnu \
#                       --profile minimal --force-non-host
#   - 首次请在有网环境先 cargo fetch（依赖已缓存在 ~/.cargo，容器内离线编译）
# ============================================================
set -e

ARCH="${1:-amd64}"
ROOT="$(cd "$(dirname "$0")" && pwd)"      # .../rust
REPO="$(cd "$ROOT/.." && pwd)"             # repository root
HOST_CARGO="${CARGO_HOME:-$HOME/.cargo}"
HOST_RUSTUP="${RUSTUP_HOME:-$HOME/.rustup}"
TOOLCHAIN="${SIDERA_RUST_TOOLCHAIN:-1.93.1}"

case "$ARCH" in
  amd64)
    SYSROOT="$REPO/amd64-sysroot"; TRIPLE=x86_64-unknown-linux-gnu
    OUT=container-amd64; RUN=(); QEMU=""
    ;;
  aarch64)
    SYSROOT="$REPO/aarch64-sysroot"; TRIPLE=aarch64-unknown-linux-gnu
    OUT=container-aarch64
    QEMU_BIN="$(command -v qemu-aarch64-static || true)"
    [ -z "$QEMU_BIN" ] && { echo "错误: 找不到 qemu-aarch64-static"; exit 1; }
    RUN=(--ro-bind "$QEMU_BIN" /usr/bin/qemu-aarch64-static)
    QEMU=/usr/bin/qemu-aarch64-static
    ;;
  *) echo "用法: $0 {amd64|aarch64}"; exit 1;;
esac

[ -d "$SYSROOT" ] || { echo "错误: 找不到 $SYSROOT；仓库不包含 rootfs，见 docs/TEST_ENVIRONMENT.md 的构建输入说明。"; exit 1; }
[ -d "$HOST_RUSTUP/toolchains/$TOOLCHAIN-$TRIPLE" ] || {
  echo "错误: 缺少工具链 $TOOLCHAIN-$TRIPLE"
  echo "  rustup toolchain install $TOOLCHAIN-$TRIPLE --profile minimal $( [ "$ARCH" = aarch64 ] && echo --force-non-host )"
  exit 1
}

# 供容器内 bind 的挂载点（放在 git 忽略的 target/ 下）
mkdir -p "$ROOT/target/.cargo-home" "$ROOT/target/.rustup-home"
TC="/mnt/rust/target/.rustup-home/toolchains/$TOOLCHAIN-$TRIPLE"
CARGO="$TC/bin/cargo"

echo "==> 在 $ARCH 容器内编译（离线，可能较慢）..."
bwrap \
  --ro-bind "$SYSROOT" / \
  --bind "$REPO" /mnt \
  --dev /dev --proc /proc --tmpfs /tmp \
  "${RUN[@]}" \
  --bind "$HOST_CARGO" /mnt/rust/target/.cargo-home \
  --ro-bind "$HOST_RUSTUP" /mnt/rust/target/.rustup-home \
  --chdir /mnt/rust \
  --setenv HOME /root \
  --setenv CARGO_HOME /mnt/rust/target/.cargo-home \
  --setenv CARGO_NET_OFFLINE true \
  --setenv PATH "$TC/bin:/usr/bin:/bin" \
  $QEMU "$CARGO" build --release --offline --locked --target-dir "target/$OUT"

echo "==> 完成: $ROOT/target/$OUT/release/sidera"
file "$ROOT/target/$OUT/release/sidera"
{
  printf 'toolchain=%s-%s\n' "$TOOLCHAIN" "$TRIPLE"
  printf 'source=%s\n' "$(git -C "$REPO" rev-parse HEAD)"
  git -C "$REPO" status --porcelain
  sha256sum "$ROOT/Cargo.lock" "$ROOT/target/$OUT/release/sidera"
  [ ! -f "$SYSROOT/var/lib/dpkg/status" ] || sha256sum "$SYSROOT/var/lib/dpkg/status"
  [ ! -f "$SYSROOT/etc/os-release" ] || cat "$SYSROOT/etc/os-release"
} > "$ROOT/target/$OUT/BUILDINFO.txt"

# 检查最高 glibc 依赖
if command -v readelf >/dev/null; then
  echo -n "最高 GLIBC 依赖: "
  readelf -V "$ROOT/target/$OUT/release/sidera" 2>/dev/null \
    | grep -oE "GLIBC_[0-9.]+" | sort -uV | tail -1
fi
