#!/usr/bin/env python3
"""Validate the actual ELF and derive Debian dependencies in its build environment.

Cross-architecture binaries must be packaged in a matching Debian/Ubuntu rootfs
with dpkg-dev and the target libraries installed; missing libraries are fatal.
Never execute the supplied binary (or use ldd).

Exit status:
  0  成功，stdout 为依赖列表
  3  二进制本身无效（ELF 格式不符、没有动态依赖等）——调用方必须停止打包
  4  环境无法推导依赖（缺少 dpkg-shlibdeps 或目标库清单）——调用方可显式回退
"""
from pathlib import Path
import argparse
import os
import re
import subprocess
import sys
import tempfile


class InvalidBinary(ValueError):
    """The file is not a usable ELF for this package (must never be packaged)."""


def dependencies(binary, architecture):
    binary = Path(binary).resolve(strict=True)
    env = {**os.environ, "LC_ALL": "C"}
    try:
        header = subprocess.check_output(["readelf", "-h", str(binary)], env=env, text=True)
        dynamic = subprocess.check_output(["readelf", "-d", str(binary)], env=env, text=True)
    except FileNotFoundError as error:
        raise RuntimeError("缺少 readelf，请安装 binutils 后再打包") from error
    except subprocess.CalledProcessError as error:
        raise InvalidBinary("无法解析 ELF 头或动态段；不是有效的可执行文件") from error
    fields = dict(re.findall(r"^\s*([^:]+):\s*(.*?)\s*$", header, re.MULTILINE))
    expected = {"amd64": "Advanced Micro Devices X86-64", "arm64": "AArch64"}[architecture]
    kind = fields.get("Type", "").split()
    if fields.get("Machine") != expected or fields.get("Class") != "ELF64" or "little endian" not in fields.get("Data", "") or not kind or kind[0] not in ("EXEC", "DYN"):
        raise InvalidBinary("ELF 架构/格式与目标安装包不匹配")
    if "(NEEDED)" not in dynamic:
        raise InvalidBinary("没有有效的动态依赖；拒绝仅有 ELF 头的文件或未经验证的静态产物")
    with tempfile.TemporaryDirectory(prefix="sidera-dependencies-") as temporary:
        root = Path(temporary)
        (root / "debian").mkdir()
        (root / "debian/control").write_text("Source: sidera\nSection: graphics\nPriority: optional\nMaintainer: Sidera contributors <noreply@localhost>\n\nPackage: sidera\nArchitecture: any\nDescription: screen annotation\n", encoding="utf-8")
        result = subprocess.run(["dpkg-shlibdeps", "-O", "-e" + str(binary)], cwd=root, env=env,
                                text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
        if result.returncode:
            raise RuntimeError("不能确定运行依赖，请在安装了目标架构依赖的构建环境打包：\n" + result.stderr)
        line = next((s for s in result.stdout.splitlines() if s.startswith("shlibs:Depends=")), None)
        if line is None or not line.split("=", 1)[1].strip():
            raise RuntimeError("依赖计算结果为空，停止打包")
        return line.split("=", 1)[1] + ", python3"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary")
    parser.add_argument("architecture", choices=("amd64", "arm64"))
    args = parser.parse_args()
    try:
        print(dependencies(args.binary, args.architecture))
        return 0
    except InvalidBinary as error:
        print("错误: " + str(error), file=sys.stderr)
        return 3
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print("错误: " + str(error), file=sys.stderr)
        return 4


if __name__ == "__main__":
    raise SystemExit(main())
