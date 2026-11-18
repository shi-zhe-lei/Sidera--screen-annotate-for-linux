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
# aarch64 采用 x86_64 rustc 交叉编译（不再用 qemu 运行 aarch64 rustc）：
#   - 编译容器仍是 amd64-sysroot
#   - 目标库和链接运行时来自 aarch64-sysroot
#   - 链接器使用 rustup 自带的 ld.lld（x86_64 原生，速度快）
#
# 依赖：
#   - bwrap（免 root）
#   - rustup 工具链 1.93.1-x86_64-unknown-linux-gnu
#   - aarch64 交叉编译还需该工具链的 aarch64-unknown-linux-gnu 标准库：
#       rustup target add --toolchain 1.93.1 aarch64-unknown-linux-gnu
#   - 首次请在有网环境先 cargo fetch（依赖已缓存在 ~/.cargo，容器内离线编译）
# ============================================================
set -e

ARCH="${1:-amd64}"
ROOT="$(cd "$(dirname "$0")" && pwd)"      # .../rust
REPO="$(cd "$ROOT/.." && pwd)"             # repository root
HOST_CARGO="${CARGO_HOME:-$HOME/.cargo}"
TOOLCHAIN="${SIDERA_RUST_TOOLCHAIN:-1.93.1}"
HOST_TRIPLE=x86_64-unknown-linux-gnu

# Prefer the project-local rustup when it exists and RUSTUP_HOME was not set.
# For a cross build, also prefer whichever toolchain already has the target std.
has_target_std() {
  [ -d "$1/toolchains/$TOOLCHAIN-$HOST_TRIPLE/lib/rustlib/aarch64-unknown-linux-gnu/lib" ]
}
if [ -n "${RUSTUP_HOME:-}" ]; then
  HOST_RUSTUP="$RUSTUP_HOME"
elif [ "$ARCH" = aarch64 ] && has_target_std "$REPO/.toolchains/rustup"; then
  HOST_RUSTUP="$REPO/.toolchains/rustup"
elif [ "$ARCH" = aarch64 ] && has_target_std "$HOME/.rustup"; then
  HOST_RUSTUP="$HOME/.rustup"
elif [ -d "$REPO/.toolchains/rustup/toolchains/$TOOLCHAIN-$HOST_TRIPLE" ]; then
  HOST_RUSTUP="$REPO/.toolchains/rustup"
else
  HOST_RUSTUP="$HOME/.rustup"
fi

case "$ARCH" in
  amd64)
    SYSROOT="$REPO/amd64-sysroot"; TRIPLE=x86_64-unknown-linux-gnu
    TARGET=""; TARGET_SYSROOT=""
    OUT=container-amd64; RUN=()
    ;;
  aarch64)
    SYSROOT="$REPO/amd64-sysroot"; TRIPLE=x86_64-unknown-linux-gnu
    TARGET=aarch64-unknown-linux-gnu
    TARGET_SYSROOT="$REPO/aarch64-sysroot"
    OUT=container-aarch64
    # The mountpoint must exist in the amd64 rootfs; /mnt is the repo bind.
    RUN=(--ro-bind "$TARGET_SYSROOT" /mnt/aarch64-sysroot)
    ;;
  *) echo "用法: $0 {amd64|aarch64}"; exit 1;;
esac

[ -d "$SYSROOT" ] || { echo "错误: 找不到 $SYSROOT；仓库不包含 rootfs，见 docs/TEST_ENVIRONMENT.md 的构建输入说明。"; exit 1; }
[ -d "$HOST_RUSTUP/toolchains/$TOOLCHAIN-$TRIPLE" ] || {
  echo "错误: 缺少工具链 $TOOLCHAIN-$TRIPLE（RUSTUP_HOME=$HOST_RUSTUP）"
  echo "  rustup toolchain install $TOOLCHAIN-$TRIPLE --profile minimal"
  exit 1
}

if [ "$ARCH" = aarch64 ]; then
  [ -d "$TARGET_SYSROOT" ] || { echo "错误: 找不到 $TARGET_SYSROOT"; exit 1; }
  [ -d "$HOST_RUSTUP/toolchains/$TOOLCHAIN-$TRIPLE/lib/rustlib/$TARGET/lib" ] || {
    echo "错误: 缺少 $TARGET 标准库（工具链 $TOOLCHAIN，RUSTUP_HOME=$HOST_RUSTUP）"
    echo "  rustup target add --toolchain $TOOLCHAIN $TARGET"
    exit 1
  }
fi

# 供容器内 bind 的挂载点（放在 git 忽略的 target/ 下）
mkdir -p "$ROOT/target/.cargo-home" "$ROOT/target/.rustup-home" "$ROOT/target/$OUT"
TC="/mnt/rust/target/.rustup-home/toolchains/$TOOLCHAIN-$TRIPLE"
CARGO="$TC/bin/cargo"

BUILD_ARGS=(build --release --offline --locked --target-dir "target/$OUT")
EXTRA_ENV=()
if [ "$ARCH" = aarch64 ]; then
  # Linker wrapper: rustc -> ld.lld -> aarch64-sysroot crt/libc.
  WRAPPER="$ROOT/target/$OUT/aarch64-linker.sh"
  WRAPPER_IN="/mnt/rust/target/$OUT/aarch64-linker.sh"
  cat > "$WRAPPER" <<'EOF'
#!/bin/bash
set -e
LLD="${SIDERA_AARCH64_LLD:?}"
SYSROOT="${SIDERA_AARCH64_SYSROOT:?}"
pie=0
# rustc's lld flavor adds "-flavor gnu"; rust-lld takes the flavor once.
args=()
skip=0
for arg in "$@"; do
  if [ "$skip" = 1 ]; then skip=0; continue; fi
  if [ "$arg" = "-flavor" ]; then skip=1; continue; fi
  [ "$arg" = "-pie" ] && pie=1
  # The sysroot's absolute dev symlinks point outside --sysroot; resolve the
  # runtime libraries explicitly so lld never falls back to the static .a.
  case "$arg" in
    -lpthread) args+=("$SYSROOT/lib/aarch64-linux-gnu/libpthread.so.0");;
    -ldl)      args+=("$SYSROOT/lib/aarch64-linux-gnu/libdl.so.2");;
    -lm)       args+=("$SYSROOT/lib/aarch64-linux-gnu/libm.so.6");;
    -lrt)      args+=("$SYSROOT/lib/aarch64-linux-gnu/librt.so.1");;
    -lutil)    args+=("$SYSROOT/lib/aarch64-linux-gnu/libutil.so.1");;
    -lc)       args+=("$SYSROOT/lib/aarch64-linux-gnu/libc.so.6"
                      "$SYSROOT/usr/lib/aarch64-linux-gnu/libc_nonshared.a");;
    *)         args+=("$arg");;
  esac
done
if [ "$pie" = 1 ]; then
  START="$SYSROOT/usr/lib/aarch64-linux-gnu/Scrt1.o"
  BEGIN="$SYSROOT/usr/lib/gcc/aarch64-linux-gnu/9/crtbeginS.o"
  END="$SYSROOT/usr/lib/gcc/aarch64-linux-gnu/9/crtendS.o"
else
  START="$SYSROOT/usr/lib/aarch64-linux-gnu/crt1.o"
  BEGIN="$SYSROOT/usr/lib/gcc/aarch64-linux-gnu/9/crtbegin.o"
  END="$SYSROOT/usr/lib/gcc/aarch64-linux-gnu/9/crtend.o"
fi
exec "$LLD" -flavor gnu -m aarch64linux --sysroot="$SYSROOT" \
  --dynamic-linker=/lib/ld-linux-aarch64.so.1 \
  -L"$SYSROOT/usr/lib/aarch64-linux-gnu" \
  -L"$SYSROOT/lib/aarch64-linux-gnu" \
  -L"$SYSROOT/usr/lib/gcc/aarch64-linux-gnu/9" \
  "$START" "$SYSROOT/usr/lib/aarch64-linux-gnu/crti.o" "$BEGIN" \
  "${args[@]}" "$END" "$SYSROOT/usr/lib/aarch64-linux-gnu/crtn.o"
EOF
  chmod +x "$WRAPPER"
  BUILD_ARGS+=(--target "$TARGET")
  EXTRA_ENV=(
    --setenv CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER "$WRAPPER_IN"
    --setenv CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS "-C linker-flavor=ld.lld"
    --setenv SIDERA_AARCH64_LLD "$TC/lib/rustlib/$HOST_TRIPLE/bin/rust-lld"
    --setenv SIDERA_AARCH64_SYSROOT /mnt/aarch64-sysroot
  )
fi

echo "==> 在 $ARCH 容器内编译（离线）..."
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
  "${EXTRA_ENV[@]}" \
  "$CARGO" "${BUILD_ARGS[@]}"

# 交叉编译（--target）产物的实际位置：<target-dir>/<triple>/release/
if [ -n "$TARGET" ]; then
  OUT_BIN="$ROOT/target/$OUT/$TARGET/release/sidera"
else
  OUT_BIN="$ROOT/target/$OUT/release/sidera"
fi
[ -f "$OUT_BIN" ] || { echo "错误: 编译后找不到二进制 $OUT_BIN" >&2; exit 1; }

echo "==> 完成: $OUT_BIN"
file "$OUT_BIN"
{
  printf 'toolchain=%s-%s\n' "$TOOLCHAIN" "$TRIPLE"
  printf 'target=%s\n' "${TARGET:-$TRIPLE}"
  printf 'source=%s\n' "$(git -C "$REPO" rev-parse HEAD)"
  git -C "$REPO" status --porcelain
  sha256sum "$ROOT/Cargo.lock" "$OUT_BIN"
  [ ! -f "$SYSROOT/var/lib/dpkg/status" ] || sha256sum "$SYSROOT/var/lib/dpkg/status"
  [ ! -f "$SYSROOT/etc/os-release" ] || cat "$SYSROOT/etc/os-release"
  if [ -n "$TARGET_SYSROOT" ]; then
    printf 'target_sysroot=%s\n' "$TARGET_SYSROOT"
    [ ! -f "$TARGET_SYSROOT/var/lib/dpkg/status" ] || sha256sum "$TARGET_SYSROOT/var/lib/dpkg/status"
    [ ! -f "$TARGET_SYSROOT/etc/os-release" ] || cat "$TARGET_SYSROOT/etc/os-release"
  fi
} > "$ROOT/target/$OUT/BUILDINFO.txt"

# 检查最高 glibc 依赖
if command -v readelf >/dev/null; then
  echo -n "最高 GLIBC 依赖: "
  readelf -V "$OUT_BIN" 2>/dev/null \
    | grep -oE "GLIBC_[0-9.]+" | sort -uV | tail -1
fi
