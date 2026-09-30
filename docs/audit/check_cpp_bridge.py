#!/usr/bin/env python3
"""Reproduce audit findings without touching the user's WPS registration.

Linux prerequisites: g++, Qt5Core/Qt5Network development files, pkg-config,
bubblewrap and Python 3. The harness uses the original wps_bridge.cpp.
"""
import argparse
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import time
import urllib.parse
import urllib.request


def inside(repo, binary, scratch):
    scratch = Path(scratch)
    out_path = scratch / "bridge.stdout"
    with out_path.open("w") as out, (scratch / "bridge.stderr").open("w") as err:
        process = subprocess.Popen([binary], cwd=scratch, stdout=out, stderr=err)
        try:
            # Check readiness through harness output, avoiding a heartbeat request.
            for _ in range(100):
                if "ACTIVE_TIMERS" in out_path.read_text():
                    break
                if process.poll() is not None:
                    raise RuntimeError("bridge exited before becoming ready")
                time.sleep(0.02)
            else:
                raise RuntimeError("bridge readiness timed out")

            def get(path):
                with urllib.request.urlopen("http://127.0.0.1:16666" + path, timeout=2) as response:
                    return response.status, response.read().decode("utf8")

            missing = get("/manifest.xml")
            suite = subprocess.run(["python3", str(repo / "cpp/tests/test_bridge_api.py")], capture_output=True, text=True)
            (scratch / "existing-suite.log").write_text(suite.stdout + suite.stderr)
            queue = get("/poll")
            get("/push?m=" + urllib.parse.quote("EVENT SlideShowBegin pos=3 click=0"))
            get("/push?m=" + urllib.parse.quote("EVENT SlideShowEnd pos=-1 click=-1"))
            get("/push?m=" + urllib.parse.quote("EVENT SlideShowState pos=3 click=0"))
            time.sleep(0.1)
            callbacks = out_path.read_text().splitlines()
        finally:
            process.terminate()
            process.wait(timeout=3)

    # Use the original installer, with only the test user's mounted home visible.
    registration = Path.home() / ".local/share/Kingsoft/wps/jsaddons/publish.xml"
    registration.parent.mkdir(parents=True, exist_ok=True)
    registration.write_text('<jsplugins><jspluginonline name="other-addin"/></jsplugins>')
    # Normalize the diagnostic copy for Windows checkouts; preserve script logic
    # and its adjacent publish.xml lookup, without editing repository files.
    installer = scratch / "install.sh"
    installer.write_text((repo / "wps-addin/install.sh").read_text())
    (scratch / "publish.xml").write_text((repo / "wps-addin/publish.xml").read_text())
    subprocess.run(["bash", str(installer)], check=True, capture_output=True)
    print(json.dumps({
        "missingManifest": {"status": missing[0], "body": missing[1]},
        "existingSuiteWithMissingManifest": {"exitCode": suite.returncode, "reportedFivePasses": "5/5" in suite.stdout},
        "pollAfterExistingSuite": {"status": queue[0], "body": queue[1]},
        "callbacksAndTimers": callbacks,
        "installerPreservesOtherAddin": "other-addin" in registration.read_text(),
    }, ensure_ascii=False, indent=2))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--inside", nargs=2, metavar=("BINARY", "SCRATCH"))
    args = parser.parse_args()
    repo = args.repo.resolve()
    if args.inside:
        inside(repo, *args.inside)
        return
    with tempfile.TemporaryDirectory(prefix="sidera-bridge-audit-", dir="/var/tmp") as temp:
        scratch = Path(temp)
        binary = scratch / "bridge-probe"
        flags = shlex.split(subprocess.check_output(["pkg-config", "--cflags", "--libs", "Qt5Core", "Qt5Network"], text=True))
        subprocess.run(["g++", "-std=c++17", "-O0", "-fPIC", "-I", str(repo / "cpp/src"),
                        str(repo / "docs/audit/bridge_probe.cpp"), str(repo / "cpp/src/wps_bridge.cpp"),
                        *flags, "-o", str(binary)], check=True)
        test_home = scratch / "home"
        test_home.mkdir()
        empty_install = scratch / "empty-install"
        empty_install.mkdir()
        env = os.environ.copy()
        env.pop("WPS_ADDIN_DIR", None)
        mounts = ["--bind", str(scratch), str(scratch),
                  "--bind", str(test_home), str(Path.home()),
                  # Make a checkout inside the real home readable after isolation.
                  "--ro-bind", str(repo), str(repo)]
        if Path("/usr/share/sidera").is_dir():
            mounts += ["--ro-bind", str(empty_install), "/usr/share/sidera"]
        subprocess.run(["bwrap", "--ro-bind", "/", "/", *mounts,
                        "--unshare-net", "--die-with-parent",
                        "--", "python3", str(Path(__file__).resolve()), "--repo", str(repo),
                        "--inside", str(binary), str(scratch)], check=True, env=env)


if __name__ == "__main__":
    main()
