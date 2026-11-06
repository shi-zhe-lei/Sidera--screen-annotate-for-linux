#!/usr/bin/env bash
# ============================================================
# 安装/更新 Sidera Windows 交叉编译工具链（项目内，不动系统 rust）
#   产物：../.toolchains/rustup (1.93.1) + x86_64-pc-windows-gnu target
# ============================================================
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
TOOLCHAIN="${SIDERA_RUST_TOOLCHAIN:-1.93.1}"

export RUSTUP_HOME="${SIDERA_RUSTUP_HOME:-$ROOT/.toolchains/rustup}"
export CARGO_HOME="${SIDERA_CARGO_HOME:-$ROOT/.toolchains/cargo}"
mkdir -p "$RUSTUP_HOME" "$CARGO_HOME"

if ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
  echo "缺少 mingw-w64，请先执行: sudo pacman -S --needed mingw-w64-gcc" >&2
  exit 1
fi

if [ ! -x "$CARGO_HOME/bin/rustup" ]; then
  echo "==> 安装 rustup 到 $CARGO_HOME ..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup-init.sh
  sh /tmp/rustup-init.sh -y --profile minimal \
    --default-toolchain "$TOOLCHAIN" --no-modify-path
fi

echo "==> 确保 Rust $TOOLCHAIN 已安装 ..."
"$CARGO_HOME/bin/rustup" toolchain install "$TOOLCHAIN" --profile minimal

echo "==> 添加 Windows target ..."
"$CARGO_HOME/bin/rustup" target add x86_64-pc-windows-gnu --toolchain "$TOOLCHAIN"

# 避免每次调用 rustup 都联网检查自更新（在受限网络下会卡住）。
"$CARGO_HOME/bin/rustup" set auto-self-update disable >/dev/null 2>&1 || true

echo ""
"$CARGO_HOME/bin/rustup" run "$TOOLCHAIN" rustc --version
x86_64-w64-mingw32-gcc --version | head -1
"$CARGO_HOME/bin/rustup" target list --installed --toolchain "$TOOLCHAIN"
