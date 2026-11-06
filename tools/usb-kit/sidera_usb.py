#!/usr/bin/env python3
"""Sidera classroom USB manager. Python 3.8+, standard library only, no root.

Installs verified program files side by side, never runs DEB maintainer scripts,
never changes /usr or system libraries. Autostart changes are opt-in and journaled.
"""
import argparse
import datetime
import fcntl
from functools import cmp_to_key
import hashlib
import html
import io
import json
import os
from pathlib import Path, PurePosixPath
import platform
import re
import shutil
import struct
import subprocess
import sys
import tarfile
import tempfile
import time
import uuid
from xml.dom import minidom
from usb_autostart import Autostart

KIT_ID = "sidera-school-usb-20261025-v2"
ORIGINAL_BINARY = Path("/usr/bin/sidera")
MAX_UNPACK = 250 * 1024 * 1024


def digest(path):
    result = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def atomic(path, content):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.is_symlink():
        raise RuntimeError("拒绝替换符号链接：" + str(path))
    fd, temporary = tempfile.mkstemp(prefix=".pending-", dir=str(path.parent))
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(content.encode("utf-8") if isinstance(content, str) else content)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        directory = os.open(str(path.parent), os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def save_json(path, value):
    atomic(path, json.dumps(value, ensure_ascii=False, indent=2) + "\n")


def load_json(path, default=None):
    try:
        return json.loads(Path(path).read_text(encoding="utf-8"))
    except FileNotFoundError:
        return {} if default is None else default


def project_nodes(document):
    return [node for node in document.documentElement.childNodes
            if node.nodeType == node.ELEMENT_NODE and node.tagName == "jspluginonline"
            and not node.namespaceURI and node.getAttribute("name") in ("sidera-bridge", "screen-annotate-bridge")]


def registration(data):
    document = minidom.parseString(data)
    if document.doctype or document.documentElement.tagName != "jsplugins" or document.documentElement.namespaceURI:
        raise ValueError("WPS 登记格式异常，原文件保持不变")
    return document


def merge_registration(before, template):
    target = registration(before) if before is not None else registration(b'<jsplugins/>')
    source = registration(template)
    entries = project_nodes(source)
    if len(entries) != 1 or entries[0].getAttribute("name") != "sidera-bridge":
        raise ValueError("WPS 登记模板异常")
    for node in project_nodes(target):
        target.documentElement.removeChild(node)
    target.documentElement.appendChild(target.importNode(entries[0], deep=True))
    return target.toxml(encoding="UTF-8")


def version_tuple(value):
    return tuple(int(n) for n in value.split("."))


def bounded_lock(stream):
    deadline = time.monotonic() + 5
    while True:
        try:
            fcntl.flock(stream.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
            return
        except BlockingIOError:
            if time.monotonic() >= deadline:
                raise RuntimeError("WPS 登记正在被其他操作使用，未修改；请稍后重试。")
            time.sleep(0.1)


def capture(args, timeout=15, env=None):
    result = subprocess.run(args, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, timeout=timeout, env=env)
    output = result.stdout.decode("utf-8", "replace")
    if result.returncode:
        raise RuntimeError("命令未通过：{}\n{}".format(" ".join(map(str, args)), output[-6000:]))
    return output


def unpack(stream, destination, deb=False):
    """Only ordinary files/directories; no extractall, links, traversal or scripts."""
    destination = Path(destination)
    total = 0
    seen = set()
    with tarfile.open(fileobj=stream, mode="r|*") as archive:
        for index, member in enumerate(archive):
            if index > 10000:
                raise ValueError("安装包文件数量异常")
            path = PurePosixPath(member.name)
            if path.is_absolute() or ".." in path.parts:
                raise ValueError("安装包包含不安全路径")
            name = str(path)
            if member.isdir():
                continue
            if not member.isfile():
                raise ValueError("安装包包含链接或特殊文件：" + name)
            if deb:
                if name == "usr/bin/sidera":
                    name = "bin/sidera"
                elif name.startswith("usr/share/sidera/wps-addin/"):
                    name = "share/wps-addin/" + name.split("wps-addin/", 1)[1]
                elif name == "usr/share/icons/hicolor/256x256/apps/sidera.png":
                    name = "icon.png"
                else:
                    continue
            elif not (name == "bin/sidera" or name == "icon.png" or name.startswith("share/")):
                raise ValueError("未知便携包文件：" + name)
            if name in seen or member.size < 0:
                raise ValueError("安装包文件重复或大小异常")
            seen.add(name)
            total += member.size
            if total > MAX_UNPACK:
                raise ValueError("安装包解压后超过限制")
            target = destination / name
            target.parent.mkdir(parents=True, exist_ok=True)
            with archive.extractfile(member) as source, target.open("xb") as output:
                shutil.copyfileobj(source, output)
            target.chmod(0o700 if name == "bin/sidera" else 0o600)
    if not (destination / "bin/sidera").is_file():
        raise ValueError("安装包缺少主程序")


def inspect_elf(path, architecture, glibc):
    data = Path(path).read_bytes()
    machine = {"arm64": 183, "amd64": 62}.get(architecture)
    if len(data) < 64 or data[:6] != b"\x7fELF\x02\x01" or struct.unpack_from("<H", data, 18)[0] != machine:
        raise ValueError("程序实际架构不匹配")
    offset = struct.unpack_from("<Q", data, 32)[0]
    size, count = struct.unpack_from("<HH", data, 54)
    interpreter = None
    if size < 56 or count > 256 or offset + count * size > len(data):
        raise ValueError("ELF 程序头不完整")
    for number in range(count):
        at = offset + number * size
        if struct.unpack_from("<I", data, at)[0] == 3:
            start = struct.unpack_from("<Q", data, at + 8)[0]
            length = struct.unpack_from("<Q", data, at + 32)[0]
            interpreter = data[start:start + length].rstrip(b"\0").decode("ascii")
    allowed = {"arm64": "/lib/ld-linux-aarch64.so.1", "amd64": "/lib64/ld-linux-x86-64.so.2"}
    if interpreter != allowed.get(architecture):
        raise ValueError("程序加载器不符合预期")
    versions = [tuple(map(int, match.split(b"."))) for match in re.findall(rb"GLIBC_([0-9]+\.[0-9]+(?:\.[0-9]+)?)\x00", data)]
    if not versions:
        raise ValueError("无法确认 glibc 要求")
    required = max(versions)
    if required > version_tuple(glibc):
        raise ValueError("程序要求 glibc {}，本机为 {}".format(".".join(map(str, required)), glibc))
    return interpreter, ".".join(map(str, required))


class Manager:
    def __init__(self, bundle, home=None):
        self.bundle = Path(bundle).resolve()
        self.home = Path(home or Path.home()).resolve()
        self.base = self.home / ".local/share/sidera-usb"
        for part in (self.home / ".local", self.home / ".local/share", self.base):
            if part.is_symlink():
                raise RuntimeError("安装目录不能是符号链接：" + str(part))
        self.base.mkdir(parents=True, exist_ok=True, mode=0o700)
        if self.base.stat().st_uid != os.getuid():
            raise RuntimeError("安装目录不属于当前用户")
        self.base.chmod(0o700)
        if not (self.bundle / "catalog.json").is_file():
            remembered = load_json(self.base / "usb-location.json")
            location = Path(remembered.get("path", "/nonexistent"))
            try:
                if (location / "kit-id.txt").read_text().strip() == KIT_ID:
                    self.bundle = location
            except OSError:
                pass
        self.lock = (self.base / "manager.lock").open("a+")
        try:
            fcntl.flock(self.lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            self.lock.close()
            raise RuntimeError("已有安装或启动管理器正在运行，请先关闭它。")
        stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S-") + uuid.uuid4().hex[:6]
        self.report = self.base / "logs" / stamp
        self.report.mkdir(parents=True, mode=0o700)
        self.events = []
        self.result = "进行中"
        self.system = {}
        self.original = load_json(self.base / "original.json")
        if (self.bundle / "kit-id.txt").is_file() and (self.bundle / "kit-id.txt").read_text().strip() == KIT_ID:
            save_json(self.base / "usb-location.json", {"path": str(self.bundle), "id": KIT_ID})
        config = Path(os.environ.get("XDG_CONFIG_HOME", str(self.home / ".config"))) if home is None else self.home / ".config"
        if not config.is_absolute():
            config = self.home / ".config"
        self.autostart = Autostart(self.base, config, atomic, save_json, load_json, self.log)
        self.autostart.recover()

    def log(self, message):
        line = datetime.datetime.now().strftime("%H:%M:%S ") + str(message)
        self.events.append(line)
        print(line, flush=True)
        with (self.report / "run.log").open("a", encoding="utf-8") as stream:
            stream.write(line + "\n")

    def finish(self):
        autostart_state = self.autostart.state()
        value = {"result": self.result, "system": self.system, "events": self.events,
                 "original": self.original, "state": load_json(self.base / "state.json"),
                 "autostart": {"phase": autostart_state.get("phase", "untouched"), "new_enabled": self.autostart.enabled()},
                 "desktop_test": "未自动验收；窗口、穿透、触摸、WPS 由用户测试"}
        save_json(self.report / "report.json", value)
        body = "结果：{}\n\n{}\n\n系统：\n{}\n\n原版程序及原配置未被安装器覆盖。\n真实课堂功能仍需人工验收。\n".format(
            self.result, "\n".join(self.events), json.dumps(self.system, ensure_ascii=False, indent=2))
        atomic(self.report / "report.txt", body)
        atomic(self.report / "report.html", '<!doctype html><meta charset="utf-8"><title>Sidera 安装报告</title>'
               '<style>body{max-width:960px;margin:40px auto;padding:0 24px;font:16px/1.8 sans-serif;background:#f6f8f7;color:#243633}pre{white-space:pre-wrap;overflow-wrap:anywhere}</style>'
               '<h1>Sidera 安装与运行报告</h1><pre>' + html.escape(body) + '</pre>')
        location = load_json(self.base / "usb-location.json")
        try:
            usb = Path(location.get("path", "/nonexistent"))
            if (usb / "kit-id.txt").read_text().strip() == KIT_ID:
                # On reconnect, also export reports recorded while the USB was absent.
                for source in (self.base / "logs").iterdir():
                    destination = usb / "logs" / source.name
                    destination.parent.mkdir(exist_ok=True)
                    if source.is_dir():
                        shutil.copytree(str(source), str(destination), dirs_exist_ok=True)
                if self.autostart.journal.is_file():
                    name = hashlib.sha256((platform.node() + str(self.home)).encode()).hexdigest()[:16]
                    backup = usb / "backups" / name
                    backup.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(self.autostart.journal, backup / "autostart-journal.json")
                print("U 盘报告：" + str(usb / "logs" / self.report.name))
        except (OSError, ValueError):
            print("U 盘未连接或不可写；完整报告保留在本机。")
        print("本机报告：" + str(self.report))

    def examine(self):
        architecture = capture(["dpkg", "--print-architecture"]).strip()
        libc = os.confstr("CS_GNU_LIBC_VERSION").split()[-1]
        self.system = {"architecture": architecture, "glibc": libc, "kernel": platform.release(),
                       "session": os.environ.get("XDG_SESSION_TYPE", ""),
                       "desktop": os.environ.get("XDG_CURRENT_DESKTOP", ""),
                       "display": os.environ.get("DISPLAY", ""),
                       "os_release": Path("/etc/os-release").read_text(encoding="utf-8", errors="replace"),
                       "free_mb": shutil.disk_usage(str(self.base)).free // 1024 // 1024}
        if architecture not in ("arm64", "amd64"):
            raise RuntimeError("这套工具只提供 ARM64 / amd64；保留原版")
        self.log("环境：{} / glibc {} / {}".format(architecture, libc, self.system["session"] or "未知会话"))
        for command in (["dpkg-query", "-W", "-f=${Package}\t${Version}\t${Architecture}\t${Status}\n", "sidera", "wps-office"],
                        ["xdpyinfo", "-queryExtensions"]):
            try:
                output = capture(command, timeout=8)
                atomic(self.report / ("package-versions.txt" if command[0] == "dpkg-query" else "display.txt"), output)
            except (OSError, RuntimeError, subprocess.TimeoutExpired) as error:
                self.log("补充检测未完成：" + str(error))
        if self.original:
            binary = Path(self.original["binary"])
            if binary.is_file() and digest(binary) == self.original["sha256"]:
                self.log("原版主程序校验一致")
            else:
                self.log("注意：原版主程序已被其他操作改变；本工具未覆盖它。")

    def backup(self):
        if self.original:
            return
        destination = self.base / "backup-original"
        if destination.exists():
            if destination.is_symlink():
                raise RuntimeError("原版备份目录不能是链接")
            incomplete = self.base / ("backup-incomplete-" + uuid.uuid4().hex[:12])
            os.rename(str(destination), str(incomplete))
            self.log("上次备份未完成，已保留其文件并重新备份原版。")
        destination.mkdir(mode=0o700)
        binary = ORIGINAL_BINARY
        record = {"binary": str(binary), "sha256": "", "files": {}, "available": binary.is_file()}
        try:
            files = capture(["dpkg-query", "-L", "sidera"]).splitlines() if binary.is_file() else []
            files += [str(binary)] if binary.is_file() else []
            for name in sorted(set(files)):
                source = Path(name)
                if not source.is_absolute() or not source.is_file() or source.is_symlink():
                    continue
                target = destination / "system-files" / str(source).lstrip("/")
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(str(source), str(target))
                checksum = digest(source)
                if digest(target) != checksum:
                    raise RuntimeError("原版备份校验失败")
                record["files"][name] = checksum
            for relative in (".config/sidera", ".config/autostart/sidera.desktop",
                             ".local/share/Kingsoft/wps/jsaddons/publish.xml"):
                source = self.home / relative
                if source.exists():
                    target = destination / "user-files" / relative
                    target.parent.mkdir(parents=True, exist_ok=True)
                    if source.is_dir():
                        shutil.copytree(str(source), str(target), symlinks=True)
                    else:
                        shutil.copy2(str(source), str(target), follow_symlinks=False)
            if binary.is_file():
                record["sha256"] = digest(binary)
            save_json(self.base / "original.json", record)
            self.original = record
            self.log("原版文件和相关配置已备份；备份校验通过。")
            # Local snapshot is mandatory; removable-media redundancy is best effort.
            try:
                location = load_json(self.base / "usb-location.json")
                usb = Path(location.get("path", "/nonexistent"))
                if (usb / "kit-id.txt").read_text().strip() == KIT_ID:
                    name = hashlib.sha256((platform.node() + str(self.home)).encode()).hexdigest()[:16]
                    mirror = usb / "backups" / name
                    mirror.parent.mkdir(exist_ok=True)
                    shutil.copytree(str(destination), str(mirror))
                    save_json(mirror / "original.json", record)
                    self.log("U 盘也已保存一份原版备份。")
            except OSError as error:
                self.log("U 盘备份副本未完成，本机备份仍完整：" + str(error))
        except Exception:
            self.log("备份未完整完成，停止安装；原版没有被修改。")
            raise

    def candidates(self):
        self.log("完全离线模式：只检查 U 盘内的已校验安装包，不连接网络。")
        options = []
        catalog = load_json(self.bundle / "catalog.json", {"packages": []})
        for entry in catalog["packages"]:
            if entry["architecture"] != self.system["architecture"]:
                continue
            entry = dict(entry)
            path = (self.bundle / entry["file"]).resolve()
            if self.bundle not in path.parents:
                raise ValueError("离线包路径越界")
            entry["file"] = str(path)
            capture(["dpkg", "--validate-version", entry["version"]])
            options.append(entry)
        def compare(left, right):
            if left["version"] == right["version"]:
                return 0
            newer = subprocess.run(["dpkg", "--compare-versions", left["version"], "gt", right["version"]],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            return -1 if newer.returncode == 0 else 1
        return sorted(options, key=cmp_to_key(compare))

    def stage(self, entry):
        package = Path(entry["file"])
        if digest(package) != entry["sha256"]:
            raise ValueError("安装包校验失败：" + package.name)
        versions = self.base / "versions"
        versions.mkdir(exist_ok=True)
        destination = versions / entry["sha256"][:20]
        if destination.exists():
            self.verify(destination)
            loader, _ = inspect_elf(destination / "bin/sidera", self.system["architecture"], self.system["glibc"])
            capture([loader, "--list", str(destination / "bin/sidera")])
            return destination
        temporary = Path(tempfile.mkdtemp(prefix=".stage-", dir=str(versions)))
        try:
            if entry["kind"] == "deb":
                fields = [capture(["dpkg-deb", "-f", str(package), field]).strip() for field in ("Package", "Version", "Architecture")]
                if fields != ["sidera", entry["version"], self.system["architecture"]]:
                    raise ValueError("DEB 包名、版本或架构与记录不一致")
                with tempfile.TemporaryFile() as data:
                    subprocess.run(["dpkg-deb", "--fsys-tarfile", str(package)], stdout=data,
                                   stderr=subprocess.PIPE, check=True, timeout=30)
                    data.seek(0)
                    unpack(data, temporary, deb=True)
            else:
                with package.open("rb") as data:
                    unpack(data, temporary)
            loader, minimum = inspect_elf(temporary / "bin/sidera", self.system["architecture"], self.system["glibc"])
            output = capture([loader, "--list", str(temporary / "bin/sidera")])
            atomic(self.report / ("dependencies-" + entry["variant"] + ".txt"), output)
            # Every variant gets an independently bundled CJK font.
            fonts = self.bundle / "fonts"
            if fonts.is_dir():
                shutil.copytree(str(fonts), str(temporary / "share/fonts"), dirs_exist_ok=True)
            files = {str(p.relative_to(temporary)): digest(p) for p in temporary.rglob("*") if p.is_file()}
            save_json(temporary / "complete.json", {"entry": entry, "minimum_glibc": minimum, "files": files})
            os.rename(str(temporary), str(destination))
            self.log("架构、glibc 和加载依赖检查通过：" + entry["version"] + " / " + entry["variant"])
            return destination
        finally:
            if temporary.exists():
                shutil.rmtree(str(temporary))

    def verify(self, directory):
        directory = Path(directory)
        record = load_json(directory / "complete.json")
        if not record.get("files"):
            raise ValueError("安装目录没有完整性记录")
        for relative, checksum in record["files"].items():
            path = directory / relative
            if PurePosixPath(relative).is_absolute() or ".." in PurePosixPath(relative).parts or path.is_symlink() or not path.is_file() or digest(path) != checksum:
                raise ValueError("已安装文件损坏：" + relative)
        return record

    def shortcuts(self):
        script = self.base / "manager.py"
        if Path(__file__).resolve() != script:
            atomic(script, Path(__file__).read_bytes())
            atomic(self.base / "usb_autostart.py", Path(__file__).with_name("usb_autostart.py").read_bytes())
        applications = self.home / ".local/share/applications"
        applications.mkdir(parents=True, exist_ok=True)
        icon = self.base / "icon.png"
        state = load_json(self.base / "state.json")
        source = self.base / "versions" / state.get("current", "missing") / "icon.png"
        if source.is_file():
            atomic(icon, source.read_bytes())
        def quote(value):
            return '"' + str(value).replace('\\', '\\\\').replace('"', '\\"').replace('`', '\\`').replace('$', '\\$').replace('%', '%%') + '"'
        for suffix, name, action, terminal in (
                ("new", "Sidera 课堂新版", "launch", "false"),
                ("original", "Sidera 原版（保留）", "original", "false"),
                ("manager", "Sidera 安装与恢复", "menu", "true")):
            entry = "[Desktop Entry]\nType=Application\nName={}\nExec=/usr/bin/python3 {} {}\nIcon={}\nTerminal={}\nCategories=Education;Utility;\n".format(
                name, quote(script), action, icon, terminal)
            atomic(applications / ("sidera-usb-" + suffix + ".desktop"), entry)

    def install(self):
        self.examine()
        if self.system["free_mb"] < 300:
            raise RuntimeError("本机可用空间不足 300 MB；保留原版")
        self.backup()
        previous = load_json(self.base / "state.json")
        for entry in self.candidates():
            try:
                if previous.get("version") and subprocess.run(["dpkg", "--compare-versions", entry["version"], "lt", previous["version"]]).returncode == 0:
                    self.log("跳过比已安装新版更旧的候选：" + entry["version"])
                    continue
                destination = self.stage(entry)
                state = {"current": destination.name, "version": entry["version"],
                         "previous": previous.get("current"), "mode": "new", "variant": entry["variant"]}
                # Program/profile staging is finished before changing the chosen version.
                self.prepare_profile(destination)
                save_json(self.base / "state.json", state)
                try:
                    self.shortcuts()
                except Exception:
                    save_json(self.base / "state.json", previous)
                    raise
                self.result = "新版并存安装完成；等待用户启动验收"
                self.log("安装完成。应用菜单打开“Sidera 课堂新版”；原来的 Sidera 入口仍然保留。")
                return True
            except Exception as error:
                self.log("候选包未采用，继续备用路径：" + str(error))
        self.result = "没有可用的新包；原版及此前安装保持原状"
        self.log(self.result)
        return False

    def prepare_profile(self, directory):
        profile = self.base / "profiles" / directory.name
        for relative in (".config/sidera", ".local/share/fonts", ".local/state", ".cache", "runtime", "tmp"):
            (profile / relative).mkdir(parents=True, exist_ok=True, mode=0o700)
        fonts = directory / "share/fonts"
        if fonts.is_dir():
            shutil.copytree(str(fonts), str(profile / ".local/share/fonts"), dirs_exist_ok=True)
        self.profile_autostart_flag(profile, self.autostart.enabled())
        return profile

    def profile_autostart_flag(self, profile, enabled):
        flag = profile / ".config/autostart/sidera.desktop"
        if enabled:
            atomic(flag, "[Desktop Entry]\nType=Application\nName=Sidera\nExec=sidera\n")
        elif flag.exists():
            flag.unlink()

    def sync_autostart_toggle(self, profile, previous):
        desired = (profile / ".config/autostart/sidera.desktop").exists()
        if desired != previous:
            try:
                self.autostart.enable() if desired else self.autostart.set_enabled(False)
            except Exception as error:
                self.log("自启动切换失败，已还原开关状态：" + str(error))
                self.profile_autostart_flag(profile, self.autostart.enabled())
                return self.autostart.enabled()
        return desired

    def busy(self):
        result = []
        for path in Path("/proc").glob("[0-9]*/comm"):
            try:
                if path.stat().st_uid == os.getuid() and path.read_text().strip() == "sidera":
                    result.append(path.parent.name)
            except OSError:
                pass
        if result:
            raise RuntimeError("Sidera 已在运行（{}）。请先保存批注并正常退出，再切换版本；工具不会强制结束它。".format(", ".join(result)))

    def run(self, executable, env, label, profile=None, autostart=False):
        self.log("启动：" + label)
        args = [str(executable)]
        if autostart:
            args.append("--autostart")
        with (self.report / "app-output.log").open("ab") as output:
            child = subprocess.Popen(args, env=env, cwd=str(executable.parent),
                                     stdin=subprocess.DEVNULL, stdout=output, stderr=subprocess.STDOUT)
            self.log("程序进程 {} 已建立；是否可正常操作需人工确认。".format(child.pid))
            previous = self.autostart.enabled()
            while True:
                try:
                    status = child.wait(timeout=0.5)
                except subprocess.TimeoutExpired:
                    if profile is not None:
                        previous = self.sync_autostart_toggle(profile, previous)
                    continue
                if profile is not None:
                    self.sync_autostart_toggle(profile, previous)
                break
        self.log("{} 退出码：{}".format(label, status))
        return status

    def original_launch(self):
        self.busy()
        state = load_json(self.base / "state.json")
        state["mode"] = "original"
        save_json(self.base / "state.json", state)
        try:
            self.autostart.restore()
        except Exception as error:
            self.log("原版自启动恢复未完成；保留记录，仍可启动原版程序：" + str(error))
        try:
            self.restore_wps()
        except Exception as error:
            self.log("WPS 登记恢复未完成，保留报告；仍可启动原版主程序：" + str(error))
        binary = Path(self.original.get("binary", "/usr/bin/sidera"))
        if not binary.is_file():
            backup = self.base / "backup-original/system-files" / str(binary).lstrip("/")
            if not backup.is_file() or digest(backup) != self.original.get("sha256"):
                raise RuntimeError("找不到可用原版，已保留报告；不修改系统")
            binary = backup
            self.log("系统原版缺失，使用备份主程序；其系统依赖仍必须存在。")
        code = self.run(binary, dict(os.environ), "原版 Sidera")
        self.result = "原版正常退出" if code == 0 else "原版启动/运行失败，退出码 " + str(code)

    def launch(self, autostart=False):
        self.busy()
        if not os.environ.get("DISPLAY"):
            raise RuntimeError("请在本机图形桌面内运行；当前没有 DISPLAY")
        if os.environ.get("XDG_SESSION_TYPE") not in ("", "x11"):
            raise RuntimeError("这套学校部署入口用于 X11。当前不是 X11，请使用原来的应用入口；未调整桌面设置。")
        state = load_json(self.base / "state.json")
        if state.get("mode") == "original" or not state.get("current"):
            return self.original_launch()
        directory = self.base / "versions" / state["current"]
        try:
            self.verify(directory)
            profile = self.prepare_profile(directory)
            env = dict(os.environ)
            env.update({"HOME": str(profile), "XDG_CONFIG_HOME": str(profile / ".config"),
                        "XDG_DATA_HOME": str(profile / ".local/share"), "XDG_CACHE_HOME": str(profile / ".cache"),
                        "XDG_STATE_HOME": str(profile / ".local/state"), "TMPDIR": str(profile / "tmp"),
                        "SIDERA_BACKEND": "x11", "SIDERA_ICON": str(directory / "icon.png"),
                        "WPS_ADDIN_DIR": str(directory / "share/wps-addin")})
            # Preserve X authentication before isolating HOME; no keys copied into the USB.
            if "XAUTHORITY" not in env and (self.home / ".Xauthority").is_file():
                env["XAUTHORITY"] = str(self.home / ".Xauthority")
            # This kit targets native X11; keep desktop D-Bus but use a private app socket.
            if os.environ.get("XDG_SESSION_TYPE") == "x11":
                env["XDG_RUNTIME_DIR"] = str(profile / "runtime")
            for name in list(env):
                if name.startswith("SIDERA_DEBUG") or name in ("SIDERA_SMOKE", "SIDERA_TEST_XTEST"):
                    env.pop(name)
            code = self.run(directory / "bin/sidera", env, "课堂新版 " + state["version"], profile, autostart)
            if code == 0:
                self.result = "新版正常退出；功能验收由用户完成"
                return
            raise RuntimeError("新版异常退出：" + str(code))
        except (OSError, RuntimeError, ValueError) as error:
            self.log("新版不可用：" + str(error))
            state["mode"] = "original"
            save_json(self.base / "state.json", state)
            if self.original.get("available") and os.environ.get("XDG_SESSION_TYPE") == "x11":
                self.log("自动回退原版。新版日志和配置保留用于排查。")
                self.original_launch()
            else:
                raise RuntimeError("新版失败，已切回原版选择；请通过原来的应用入口启动。")

    def wps(self):
        self.busy()
        for path in Path("/proc").glob("[0-9]*/comm"):
            try:
                if path.stat().st_uid == os.getuid() and path.read_text().strip() in ("wps", "wpp", "et", "wpscloudsvr"):
                    raise RuntimeError("请先保存并关闭 WPS，再启用联动登记。")
            except OSError:
                pass
        state = load_json(self.base / "state.json")
        directory = self.base / "versions" / state.get("current", "missing")
        self.verify(directory)
        destination = self.home / ".local/share/Kingsoft/wps/jsaddons/publish.xml"
        if destination.is_symlink():
            raise RuntimeError("WPS 登记为链接，停止修改")
        existing = load_json(self.base / "wps-transaction.json")
        if existing.get("active"):
            self.log("本工具已登记 WPS 联动；无需重复修改。")
            return
        destination.parent.mkdir(parents=True, exist_ok=True)
        descriptor = os.open(str(destination.parent / ".sidera-registration.lock"), os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
        with os.fdopen(descriptor, "r+b") as lock:
            bounded_lock(lock)
            before = destination.read_bytes() if destination.exists() else None
            after = merge_registration(before, (directory / "share/wps-addin/publish.xml").read_bytes())
            record = {"active": True, "had_original": before is not None, "before": (before or b"").hex(),
                      "after_sha256": hashlib.sha256(after).hexdigest(), "after": after.hex()}
            # Expected post-state is durable before mutation: interruptions are recoverable.
            save_json(self.base / "wps-transaction.json", record)
            atomic(destination, after)
        self.log("WPS 登记完成。重新打开 WPS；切回原版时会还原本次登记。")

    def restore_wps(self):
        record = load_json(self.base / "wps-transaction.json")
        if not record.get("active"):
            return
        destination = self.home / ".local/share/Kingsoft/wps/jsaddons/publish.xml"
        destination.parent.mkdir(parents=True, exist_ok=True)
        descriptor = os.open(str(destination.parent / ".sidera-registration.lock"), os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
        with os.fdopen(descriptor, "r+b") as lock:
            bounded_lock(lock)
            self.restore_registration(record, destination)

    def restore_registration(self, record, destination):
        current = digest(destination) if destination.is_file() and not destination.is_symlink() else None
        before = bytes.fromhex(record["before"])
        if current == hashlib.sha256(before).hexdigest() or (current is None and not record["had_original"]):
            record["active"] = False
        elif current and current == record.get("after_sha256"):
            if record["had_original"]:
                atomic(destination, before)
            else:
                destination.unlink()
            record["active"] = False
            self.log("已还原本次 WPS 登记；重新打开 WPS 后生效。")
        else:
            # Preserve changes to unrelated add-ins; replace only unchanged project entries.
            if current:
                document = registration(destination.read_bytes())
                expected = registration(bytes.fromhex(record["after"]))
                def attributes(document):
                    return [sorted(node.attributes.items()) for node in project_nodes(document)]
                if attributes(document) == attributes(expected):
                    for node in project_nodes(document):
                        document.documentElement.removeChild(node)
                    if record["had_original"]:
                        for node in project_nodes(registration(before)):
                            document.documentElement.appendChild(document.importNode(node, deep=True))
                    atomic(destination, document.toxml(encoding="UTF-8"))
                    record["active"] = False
                    self.log("已还原 Sidera 的 WPS 条目，保留其他加载项的后续更改。")
            if record["active"]:
                self.log("WPS 的 Sidera 登记被外部修改，未覆盖；原版主程序仍可启动。原登记保存在 wps-transaction.json。")
        save_json(self.base / "wps-transaction.json", record)

    def restore(self):
        self.busy()
        try:
            self.autostart.restore()
        except Exception as error:
            self.log("自启动入口被外部改动，未覆盖；原版程序保持可用：" + str(error))
        try:
            self.restore_wps()
        except Exception as error:
            self.log("WPS 登记恢复未完成，未覆盖其文件；原版主程序仍可使用：" + str(error))
        state = load_json(self.base / "state.json")
        state["mode"] = "original"
        save_json(self.base / "state.json", state)
        self.result = "已选择原版；新版及日志保留"
        self.log("已切回原版。原来的系统应用入口一直保持不变；不自动启动、卸载或关闭程序。")


def main():
    parser = argparse.ArgumentParser(description="Sidera U 盘安装与恢复（无需 sudo）")
    parser.add_argument("action", nargs="?", default="menu", choices=("menu", "install", "offline", "launch", "original", "restore", "diagnose", "wps", "auto-new", "auto-off", "auto-original", "autostart"))
    args = parser.parse_args()
    if sys.platform != "linux" or os.geteuid() == 0:
        print("请在目标 Linux 图形桌面以普通用户运行，不要使用 sudo。")
        return 1
    manager = None
    try:
        manager = Manager(Path(__file__).parent)
        action = args.action
        if action == "menu":
            print("\nSidera 学校安装工具\n原版保留 · 新版独立配置 · 失败可恢复\n")
            print("1  离线安装 / 更新\n2  启动课堂新版\n3  启动原版\n4  切回原版（含恢复原自启）\n5  检测并导出报告\n6  启用 WPS 联动（可选；需先关闭 WPS）\n7  开机启动新版（接管旧版自启）\n8  关闭已接管的 Sidera 自启\n9  恢复原版自启动设置\n0  退出\n")
            choice = input("输入数字，按回车：").strip()
            action = {"1": "install", "2": "launch", "3": "original", "4": "restore", "5": "diagnose", "6": "wps", "7": "auto-new", "8": "auto-off", "9": "auto-original", "0": "exit"}.get(choice)
            if not action:
                raise ValueError("无效选项，请重新运行。")
        if action in ("install", "offline"):
            if not manager.install():
                return 2
        elif action == "launch":
            manager.launch()
        elif action == "original":
            manager.original_launch()
        elif action == "restore":
            manager.restore()
        elif action == "diagnose":
            manager.examine()
            manager.result = "只读检测完成"
        elif action == "wps":
            manager.wps()
            manager.result = "WPS 登记完成"
        elif action == "auto-new":
            state = load_json(manager.base / "state.json")
            manager.verify(manager.base / "versions" / state.get("current", "missing"))
            manager.autostart.enable()
            state["mode"] = "new"
            save_json(manager.base / "state.json", state)
            manager.result = "新版自启动已开启；旧版自启已备份停用"
        elif action == "auto-off":
            manager.autostart.set_enabled(False)
            manager.result = "受管理的自启动已关闭；原设置仍可恢复"
        elif action == "auto-original":
            manager.autostart.restore()
            manager.result = "原版自启动设置已恢复"
        elif action == "autostart":
            if manager.autostart.recovered:
                manager.original_launch()
            elif manager.autostart.enabled():
                manager.launch(autostart=True)
            else:
                manager.result = "自启动未启用；未启动任何应用"
        else:
            manager.result = "用户退出"
        return 0
    except (Exception, KeyboardInterrupt) as error:
        if manager:
            manager.result = "未完成：" + (str(error) or "用户中断")
            manager.log(manager.result)
            if args.action in ("launch", "original") and shutil.which("notify-send"):
                try:
                    subprocess.run(["notify-send", "Sidera 安装与恢复", manager.result], timeout=3, check=False)
                except (OSError, subprocess.TimeoutExpired):
                    pass
        else:
            print("未完成：" + str(error))
        return 1
    finally:
        if manager:
            try:
                manager.finish()
            finally:
                manager.lock.close()


if __name__ == "__main__":
    sys.exit(main())
