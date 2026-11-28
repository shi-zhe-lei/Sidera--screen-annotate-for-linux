#!/bin/bash
# 只合并本项目的 WPS 加载项登记，保留其他加载项
# 前提：批注 app 已开启“WPS 联动”（内容由 app 的 16666 服务提供）
set -e
if ! command -v python3 >/dev/null 2>&1; then
  echo "错误：登记 WPS 加载项需要 python3，请先安装。" >&2
  exit 1
fi
DST="${HOME}/.local/share/Kingsoft/wps/jsaddons"
SRC="$(dirname "$(readlink -f "$0")")/publish.xml"
if [ ! -f "$SRC" ]; then
  SRC="/usr/share/sidera/wps-addin/publish.xml"
fi
REPAIR=0
for arg in "$@"; do
  [ "$arg" = "--repair" ] && REPAIR=1
done
python3 - "$SRC" "$DST/publish.xml" "$REPAIR" <<'PY'
import os
import fcntl
from pathlib import Path
import stat
import sys
import tempfile
from xml.dom import minidom
from xml.parsers.expat import ExpatError
import re


def parse_registration(data):
    document = minidom.parseString(data)
    root = document.documentElement
    if document.doctype or root.tagName != "jsplugins" or root.namespaceURI:
        raise ValueError("登记文件必须使用无命名空间、无 DOCTYPE 的 jsplugins 根节点")
    return document


def project_entries(document):
    return [node for node in document.documentElement.childNodes
            if node.nodeType == node.ELEMENT_NODE
            and node.tagName == "jspluginonline" and not node.namespaceURI
            and node.getAttribute("name") in ("sidera-bridge", "screen-annotate-bridge")]


def salvage_registration(data):
    """Rebuild from complete <jspluginonline> elements when the wrapper is damaged."""
    text = data.decode("utf-8-sig", errors="replace")
    elements = re.findall(
        r"<jspluginonline\b[^>]*?/>|<jspluginonline\b[^>]*?>.*?</jspluginonline>",
        text, re.DOTALL)
    valid = []
    for element in elements:
        try:
            node = minidom.parseString(element).documentElement
            if node.tagName == "jspluginonline" and not node.namespaceURI:
                valid.append(element)
        except ExpatError:
            continue
    rebuilt = '<?xml version="1.0" encoding="UTF-8"?><jsplugins>' + "".join(valid) + "</jsplugins>"
    return parse_registration(rebuilt.encode("utf-8")), len(valid)


def write_temp(destination, contents, prefix, mode):
    # Same directory guarantees os.replace stays on the same filesystem.
    with tempfile.NamedTemporaryFile(dir=destination.parent, prefix=prefix, delete=False) as stream:
        path = Path(stream.name)
        try:
            stream.write(contents)
            stream.flush()
            os.fchmod(stream.fileno(), mode)
            os.fsync(stream.fileno())
        except OSError:
            path.unlink()
            raise
    return path


def install(source, destination, repair):
    template = parse_registration(source.read_bytes())
    entries = project_entries(template)
    if len(entries) != 1 or entries[0].getAttribute("name") != "sidera-bridge":
        raise ValueError("安装模板必须包含一个 sidera-bridge 登记")
    expected = entries[0]
    if destination.is_symlink():
        raise ValueError(f"登记文件是符号链接，请检查后重试：{destination}")
    try:
        original = destination.read_bytes()
    except FileNotFoundError:
        original = None
    if original is None:
        document = template
        changed = True
    else:
        try:
            document = parse_registration(original)
            changed = False
        except ExpatError as error:
            if not repair:
                raise
            document, recovered = salvage_registration(original)
            changed = True
            print(f"登记文件损坏，已备份并从 {recovered} 个可恢复条目重建：{error}")
    entries = project_entries(document)
    if entries:
        for key, value in expected.attributes.items():
            if entries[0].getAttribute(key) != value or not entries[0].hasAttribute(key):
                entries[0].setAttribute(key, value)
                changed = True
        for duplicate in entries[1:]:
            document.documentElement.removeChild(duplicate).unlink()
            changed = True
    else:
        document.documentElement.appendChild(document.importNode(expected, deep=True))
        changed = True
    if not changed:
        print(f"登记已是最新：{destination}")
        return

    contents = document.toxml(encoding="UTF-8")
    mode = stat.S_IMODE(destination.stat().st_mode) if original is not None else 0o644
    destination.parent.mkdir(parents=True, exist_ok=True)
    backup = None
    if original is not None:
        backup = write_temp(destination, original, "publish.xml.sidera-backup-", 0o600)
    pending = write_temp(destination, contents, ".publish.xml-", mode)
    try:
        os.replace(pending, destination)
    finally:
        if pending.exists():
            pending.unlink()
    print(f"已更新本项目登记：{destination}")
    if backup:
        print(f"原文件备份：{backup}")
    print("重新打开 WPS 演示后生效。")


try:
    destination = Path(sys.argv[2])
    repair = len(sys.argv) > 3 and sys.argv[3] == "1"
    destination.parent.mkdir(parents=True, exist_ok=True)
    # Serialize our manual/automatic registration paths before reading XML.
    # Keep the lock inode in place; unlinking it would let a third writer bypass it.
    descriptor = os.open(destination.parent / ".sidera-registration.lock",
                         os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "r+b") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        install(Path(sys.argv[1]), destination, repair)
except (OSError, ValueError, ExpatError) as error:
    print(f"错误：WPS 加载项登记失败，原文件未替换：{error}", file=sys.stderr)
    sys.exit(1)
PY

# 同一加载项目录里还残留 2.6/更早版本的登记与授权时一并清理。
# 自动登记超时会结束整个进程组，因此这里保留子进程调用而不是 exec。
CLEANUP="$(dirname "$(readlink -f "$0")")/cleanup-old.sh"
[ -f "$CLEANUP" ] || CLEANUP="/usr/share/sidera/wps-addin/cleanup-old.sh"
if [ -f "$CLEANUP" ]; then
  bash "$CLEANUP" || true
fi
