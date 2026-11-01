#!/usr/bin/env bash
# ============================================================
# 打包 Windows 产物：dist/sidera-windows-x64.zip
# ============================================================
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
DIST="$HERE/dist"
EXE="$HERE/target/x86_64-pc-windows-gnu/release/sidera.exe"

"$HERE/build.sh" "$@"

mkdir -p "$DIST"
cp -f "$EXE" "$DIST/sidera.exe"
if [ -f "$HERE/assets/README.txt" ]; then
  cp -f "$HERE/assets/README.txt" "$DIST/README.txt"
fi
if [ -f "$HERE/assets/sidera.ico" ]; then
  cp -f "$HERE/assets/sidera.ico" "$DIST/sidera.ico"
fi
ADDIN="$HERE/../wps-addin"
if [ -d "$ADDIN" ]; then
  rm -rf "$DIST/wps-addin"
  mkdir -p "$DIST/wps-addin/js"
  for f in publish.xml manifest.xml main.js index.html ribbon.xml; do
    cp -f "$ADDIN/$f" "$DIST/wps-addin/"
  done
  cp -f "$ADDIN/js/bridge.js" "$DIST/wps-addin/js/"
fi

DIST="$DIST" python3 - <<'PY'
import os, zipfile

dist = os.environ["DIST"]
out = os.path.join(dist, "sidera-windows-x64.zip")
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
    z.write(os.path.join(dist, "sidera.exe"), "sidera.exe")
    readme = os.path.join(dist, "README.txt")
    if os.path.exists(readme):
        z.write(readme, "README.txt")
    icon = os.path.join(dist, "sidera.ico")
    if os.path.exists(icon):
        z.write(icon, "sidera.ico")
    addin = os.path.join(dist, "wps-addin")
    if os.path.isdir(addin):
        for root, _, files in os.walk(addin):
            for name in files:
                full = os.path.join(root, name)
                z.write(full, os.path.relpath(full, dist))
print(out)
PY

echo ""
echo "==> 打包完成: $DIST/sidera-windows-x64.zip"
ls -lh "$DIST/sidera-windows-x64.zip"
