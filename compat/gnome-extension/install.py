#!/usr/bin/env python3
"""Install/enable the native extension for the invoking desktop user. No restart."""
import argparse
import ast
import datetime
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

UUID = "sidera-native@sidera.local"


def command(*args, check=True):
    return subprocess.run(args, check=check, text=True, stdout=subprocess.PIPE,
                          stderr=subprocess.STDOUT, timeout=15)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--disable", action="store_true", help="禁用扩展，释放其覆盖层")
    options = parser.parse_args()
    if os.geteuid() == 0:
        raise SystemExit("请用已登录桌面的普通用户运行此工具，不要使用 sudo。")
    if options.disable:
        result = command("gnome-extensions", "disable", UUID)
        print(result.stdout or "GNOME 原生扩展已禁用；兼容模式仍可使用。")
        return
    source = Path(__file__).resolve().parent
    if not (source / "metadata.json").is_file():
        source = Path("/usr/share/sidera/gnome-extension")
    if not (source / "metadata.json").is_file():
        raise SystemExit("未找到扩展资源，请重新安装完整的 Sidera 软件包。")
    data = Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local/share"))
    if not data.is_absolute():
        raise SystemExit("XDG_DATA_HOME 必须是绝对路径。")
    extensions = data / "gnome-shell/extensions"
    extensions.mkdir(parents=True, exist_ok=True)
    destination = extensions / UUID
    if destination.is_symlink() or (destination.exists() and not destination.is_dir()):
        raise SystemExit("已有扩展目录不是普通目录，已停止安装。")
    staged = Path(tempfile.mkdtemp(prefix=".sidera-install-", dir=extensions))
    backup = None
    try:
        for name in ("extension.js", "bridge.js", "chrome.js", "keyboard.js", "presentation.js", "metadata.json"):
            shutil.copy2(source / name, staged / name)
        shutil.copytree(source / "schemas", staged / "schemas", ignore=shutil.ignore_patterns("gschemas.compiled"))
        command("glib-compile-schemas", "--strict", str(staged / "schemas"))
        if destination.exists():
            backups = data / "sidera/extension-backups"
            backups.mkdir(parents=True, exist_ok=True)
            backup = backups / (datetime.datetime.now().strftime("%Y%m%d-%H%M%S-") + os.urandom(4).hex())
            destination.rename(backup)
        try:
            staged.rename(destination)
        except OSError:
            if backup is not None:
                backup.rename(destination)
            raise
    finally:
        if staged.exists():
            shutil.rmtree(staged)
    print(f"扩展已安装到 {destination}")
    if backup:
        print(f"旧扩展备份：{backup}")
    # Preserve every other extension. The Shell may only discover a newly
    # installed extension after the next login; record the desired state now.
    current = command("gsettings", "get", "org.gnome.shell", "enabled-extensions").stdout.strip()
    enabled = ast.literal_eval(current.removeprefix("@as "))
    if not isinstance(enabled, list) or any(not isinstance(x, str) for x in enabled):
        raise SystemExit("无法解析当前扩展列表；文件已安装，未修改启用列表。")
    if UUID not in enabled:
        command("gsettings", "set", "org.gnome.shell", "enabled-extensions", repr(enabled + [UUID]))
    result = command("gnome-extensions", "enable", UUID, check=False)
    print(result.stdout.strip())
    if result.returncode:
        print("已登记为启用。请保存内容后注销并重新登录，再打开 Sidera（GNOME 原生）。")
    else:
        print("已提交启用请求。新版扩展或首次安装若尚未加载，请注销并重新登录。")
    print("本工具没有重启桌面，也没有启动或测试 Sidera。")


if __name__ == "__main__":
    main()
