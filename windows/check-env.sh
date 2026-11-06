#!/usr/bin/env bash
# ============================================================
# 检查 Sidera Windows 交叉编译环境
# ============================================================
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
TC="$ROOT/.toolchains/rustup/toolchains/1.93.1-x86_64-unknown-linux-gnu"
RUSTUP="$ROOT/.toolchains/cargo/bin/rustup"
export RUSTUP_HOME="$ROOT/.toolchains/rustup"
ok=0

check() {
  local name="$1"; shift
  if "$@" >/dev/null 2>&1; then
    printf '  [OK]   %s\n' "$name"
  else
    printf '  [FAIL] %s\n' "$name"
    ok=1
  fi
}

printf 'Sidera Windows 交叉编译环境\n'
check "mingw-w64 (x86_64-w64-mingw32-gcc)" command -v x86_64-w64-mingw32-gcc
check "rustc 1.93.1" test -x "$TC/bin/rustc"
check "cargo 1.93.1" test -x "$TC/bin/cargo"
check "Windows target std" test -d "$TC/lib/rustlib/x86_64-pc-windows-gnu"
check "KVM (/dev/kvm)" test -e /dev/kvm
check "Docker" command -v docker
check "Win10 镜像" docker image inspect ghcr.io/dockur/windows:6.05

if [ -x "$TC/bin/rustc" ]; then
  printf '  rustc:  %s\n' "$("$TC/bin/rustc" --version)"
fi
if command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
  printf '  mingw:  %s\n' "$(x86_64-w64-mingw32-gcc --version | head -1)"
fi
if [ -x "$RUSTUP" ]; then
  printf '  targets: %s\n' "$(timeout 10 "$RUSTUP" target list --installed --toolchain 1.93.1 2>/dev/null | tr '\n' ' ')"
fi

exit "$ok"
