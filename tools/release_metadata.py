#!/usr/bin/env python3
"""Record a native release build without modifying its application sources."""
import hashlib
import os
from pathlib import Path
import re
import subprocess
import sys
import tarfile


def output(*command):
    return subprocess.check_output(command, text=True, env={**os.environ, "LC_ALL": "C"}).strip()


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    source, artifacts = (Path(value).resolve(strict=True) for value in sys.argv[1:])
    tools_root = Path(__file__).resolve().parents[1]
    tag = os.environ["RELEASE_TAG"]
    version = os.environ["SIDERA_VERSION"]
    architecture = os.environ.get("DEB_ARCH", "arm64")
    machine = {"amd64": "Advanced Micro Devices X86-64", "arm64": "AArch64"}[architecture]
    legacy = re.fullmatch(r"v3\.0-Electro-testing-custom[0-9]{8}\.[0-9]+", tag)
    current = re.fullmatch(r"v3\.2-Electro(-stable|\.rc(\.[0-9]+)?)", tag)
    if not (legacy or current):
        raise ValueError("Invalid release tag")
    if version != tag[1:].replace("-custom", "+custom"):
        raise ValueError("Package version does not match release tag")
    commit = output("git", "-C", str(source), "rev-parse", "HEAD")
    if commit != output("git", "-C", str(source), "rev-parse", f"refs/tags/{tag}^{{commit}}"):
        raise ValueError("Application checkout does not match release tag")
    support_commit = output("git", "-C", str(tools_root), "rev-parse", "HEAD")
    package = artifacts / f"sidera_{version}_{architecture}.deb"
    if output("dpkg-deb", "-f", str(package), "Architecture") != architecture:
        raise ValueError("Package architecture mismatch")
    if output("dpkg-deb", "-f", str(package), "Version") != version:
        raise ValueError("Package version mismatch")
    binary = source / "rust/target/release/sidera"
    header = output("readelf", "-h", str(binary))
    if not re.search(r"Machine:\s+" + re.escape(machine) + r"\s*$", header, re.MULTILINE):
        raise ValueError("Executable architecture mismatch")
    dependencies = output("dpkg-deb", "-f", str(package), "Depends")
    tests = (artifacts / f"STARTUP-TESTS-{architecture}.txt").read_text(encoding="utf-8")
    passed = re.search(r"Ran (\d+) tests in [\d.]+s\s+OK\s*$", tests)
    if not passed or int(passed[1]) < 1:
        raise ValueError("Successful native startup test report is missing")
    extra_tests = []
    for suite in ("RUST", "X11", "WAYLAND"):
        path = artifacts / f"{suite}-TESTS-{architecture}.txt"
        if os.environ.get("REQUIRE_FULL_TESTS") == "1" or path.exists():
            report = path.read_text(encoding="utf-8")
            # test_rust.py runs every test executable. Some binaries (for
            # example the bin target filtered to x11::) legitimately report
            # "0 passed" before the library executable reports its results.
            # Sum the whole report instead of trusting only the first line.
            suite_passed = sum(int(m.group(1)) for m in
                               re.finditer(r"test result: ok\. (\d+) passed; 0 failed;", report))
            if suite_passed < 1:
                raise ValueError(f"Successful {suite} test report is missing")
            extra_tests.append(f"| Native isolated {suite} tests | {suite_passed} passed; {path.name} |")

    # Include the packaging/verification sources alongside the already-published
    # tagged application + vendored dependency archive. No executable is changed.
    with tarfile.open(artifacts / f"BUILD-SCRIPTS-{architecture}.tar.gz", "w:gz") as archive:
        for name in ("rust/build_deb.sh", "tools/deb_dependencies.py",
                     "tools/run_isolated.py", "tools/release_metadata.py",
                     "tools/test_rust.py", "tools/run_wayland_isolated.py", "LICENSE",
                     ".github/scripts/prepare-isolation.sh", ".github/workflows/release.yml",
                     ".github/workflows/release-arm64.yml"):
            archive.add(tools_root / name, arcname=name)
    report = f"""# {architecture} build record for {tag}

The application sources match the release tag recorded below.

| Item | Value |
| --- | --- |
| Application source | `{commit}` |
| Packaging / verification source | `{support_commit}` |
| Architecture | Native {architecture} / {machine} (no emulation) |
| Build OS | {output('lsb_release', '-ds')} |
| System libc | {output('getconf', 'GNU_LIBC_VERSION')} |
| Rust | {output('rustc', '+' + os.environ['RUST_VERSION'], '--version')} |
| Cargo | {output('cargo', '+' + os.environ['RUST_VERSION'], '--version')} |
| Package version | `{version}` |
| Actual DEB dependencies | `{dependencies}` |
| Cargo.lock SHA256 | `{sha256(source / 'rust/Cargo.lock')}` |
| DEB SHA256 | `{sha256(package)}` |
| Executable SHA256 | `{sha256(binary)}` |
| Native isolated startup tests | {passed[1]} passed; report in STARTUP-TESTS-{architecture}.txt |
{chr(10).join(extra_tests)}
| Workflow | https://github.com/{os.environ['GITHUB_REPOSITORY']}/actions/runs/{os.environ['GITHUB_RUN_ID']} |

## Rebuild

Use the native Ubuntu build environment recorded above with Rust 1.93.1 and the dependencies
listed in the workflow. Check out the application source commit above; the
release source-with-dependencies archive contains the same Rust sources
and locked dependency sources. Run `cargo +1.93.1 build --release --locked` in
`rust/` (add `--offline` when using the vendored source archive).

The archived `rust/build_deb.sh` and `tools/deb_dependencies.py` replace only the
old packaging helpers. They derive dependencies from the actual ELF with
dpkg-shlibdeps. Copy them into the corresponding paths of the source checkout,
then run `SIDERA_VERSION={version} bash rust/build_deb.sh {architecture}` from its root.
See the archived workflow for exact build, package extraction and test commands.
These records describe provenance; byte-for-byte reproducibility is not claimed.

## Verification and limits

The binary was compiled and executed on a native {architecture} runner. The package's ELF
architecture/version and extracted binary match the build; dependency resolution
was simulated by APT. Startup checks use a private home, network and Xvfb/Wayland
protocol fixtures. No real classroom desktop, touchscreen or WPS was exercised.
A matching CPU architecture does not by itself establish compatibility with every Kylin
release or GNOME version. Match both architecture and the listed dependencies.
GNOME extension metadata still applies. WPS and system graphics fixes are not
bundled. Later custom branch features are not backported into this release.

The project remains GPL v3. Dependency notices are in THIRD_PARTY_NOTICES.md;
the accompanying source archive is still applicable to this application build.
SHA256SUMS-{architecture} is an unsigned integrity manifest for this build.
"""
    (artifacts / f"BUILDINFO-{architecture}.md").write_text(report, encoding="utf-8")
    files = sorted(path for path in artifacts.iterdir() if path.is_file() and path.name != f"SHA256SUMS-{architecture}")
    (artifacts / f"SHA256SUMS-{architecture}").write_text(
        "".join(f"{sha256(path)}  {path.name}\n" for path in files), encoding="utf-8")
    print(report)


if __name__ == "__main__":
    main()
