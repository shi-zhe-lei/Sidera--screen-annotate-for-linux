#!/usr/bin/env bash
# ============================================================
# Sidera Windows — 交叉编译（Linux 宿主 -> x86_64-pc-windows-gnu）
#
#   ./build.sh              只编译
#   ./build.sh --offline    强制离线（默认复用 ~/.cargo 缓存）
# ============================================================
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
TC="$ROOT/.toolchains/rustup/toolchains/1.93.1-x86_64-unknown-linux-gnu"
OUT="$HERE/target/x86_64-pc-windows-gnu/release/sidera.exe"

if [ ! -x "$TC/bin/cargo" ]; then
  echo "错误: 找不到 Rust 1.93.1，请先运行 $HERE/setup-toolchain.sh" >&2
  exit 1
fi
if ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
  echo "错误: 找不到 mingw-w64，请先安装 mingw-w64-gcc" >&2
  exit 1
fi

export PATH="$TC/bin:$PATH"
export CARGO_HOME="${SIDERA_CARGO_HOME:-$HOME/.cargo}"
export CARGO_NET_OFFLINE="${CARGO_NET_OFFLINE:-true}"

cd "$HERE"
echo "==> 交叉编译 sidera.exe ..."
cargo build --release --locked --target x86_64-pc-windows-gnu "$@"

cp -f "$HERE/assets/sidera.ico" "$(dirname "$OUT")/" 2>/dev/null || true

echo ""
echo "==> 产物: $OUT"
file "$OUT" 2>/dev/null || true
echo ""
echo "Windows 依赖 DLL:"
x86_64-w64-mingw32-objdump -p "$OUT" 2>/dev/null \
  | awk '/DLL Name/{print "  " $3}' | sort -u || true
