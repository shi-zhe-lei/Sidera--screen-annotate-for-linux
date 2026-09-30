#!/usr/bin/env python3
"""Black-box DEB packaging checks; no Rust build, installation or root required.

Run on Linux: python3 rust/tests/test_build_deb.py
Requires bash, binutils, dpkg-deb and standard coreutils. The ELF fixtures carry
architecture metadata only; these tests do not prove native application startup.
"""

from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import unittest


REPO = Path(__file__).resolve().parents[2]
VERSION = "3.0-Electro-testing"


def elf_fixture(machine, elf_class=2, byte_order=1, kind=2):
    ident = b"\x7fELF" + bytes((elf_class, byte_order, 1)) + bytes(9)
    endian = "<" if byte_order == 1 else ">"
    layout = "HHIQQQIHHHHHH" if elf_class == 2 else "HHIIIIIHHHHHH"
    return ident + struct.pack(
        endian + layout, kind, machine, 1, 0, 0, 0, 0,
        64 if elf_class == 2 else 52, 0, 0, 0, 0, 0,
    )


class BuildDebTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        missing = [name for name in ("bash", "readelf", "dpkg-deb")
                   if shutil.which(name) is None]
        if missing:
            raise RuntimeError("Missing test dependencies: " + ", ".join(missing))

    def make_repo(self):
        scratch = tempfile.TemporaryDirectory(prefix="sidera-deb-test-")
        self.addCleanup(scratch.cleanup)
        repo = Path(scratch.name) / "repo with spaces"
        (repo / "rust").mkdir(parents=True)
        # Normalize the Windows checkout's CRLF in the isolated Linux copy.
        (repo / "rust/build_deb.sh").write_text(
            (REPO / "rust/build_deb.sh").read_text(encoding="utf-8"),
            encoding="utf-8",
        )
        for name in ("sidera.svg", "sidera.png", "README.classroom.md", "LICENSE"):
            shutil.copyfile(REPO / name, repo / name)
        shutil.copytree(REPO / "wps-addin", repo / "wps-addin")
        return repo

    def place_binary(self, repo, relative_path, contents):
        binary = repo / "rust/target" / relative_path / "sidera"
        binary.parent.mkdir(parents=True, exist_ok=True)
        binary.write_bytes(contents)

    def build(self, repo, arch):
        return subprocess.run(
            ["bash", str(repo / "rust/build_deb.sh"), arch], cwd=repo,
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            text=True, encoding="utf-8", timeout=30,
        )

    def assert_rejected(self, repo, arch):
        deb_arch = "arm64" if arch in ("arm64", "aarch64") else arch
        package = repo / f"sidera_{VERSION}_{deb_arch}.deb"
        package.write_bytes(b"previous package")
        staging = package.with_suffix("")
        staging.mkdir()
        marker = staging / "previous-build"
        marker.write_bytes(b"keep")
        result = self.build(repo, arch)
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn("错误", result.stdout)
        self.assertEqual(package.read_bytes(), b"previous package")
        self.assertEqual(marker.read_bytes(), b"keep")
        self.assertFalse((staging / "usr").exists())

    def test_matching_architectures_produce_consistent_packages(self):
        cases = (
            ("amd64", "container-amd64/release", 62, 2),
            ("amd64", "release", 62, 3),  # PIE executables use ET_DYN.
            ("aarch64", "container-aarch64/release", 183, 2),
            ("arm64", "container-aarch64/release", 183, 3),
            ("arm64", "release", 183, 2),
        )
        for arch, location, machine, kind in cases:
            with self.subTest(arch=arch, location=location, kind=kind):
                repo = self.make_repo()
                contents = elf_fixture(machine, kind=kind)
                self.place_binary(repo, location, contents)
                if location != "release":
                    # A correctly targeted container build takes priority.
                    other_machine = 183 if machine == 62 else 62
                    self.place_binary(repo, "release", elf_fixture(other_machine))
                result = self.build(repo, arch)
                self.assertEqual(result.returncode, 0, result.stdout)
                deb_arch = "amd64" if arch == "amd64" else "arm64"
                package = repo / f"sidera_{VERSION}_{deb_arch}.deb"
                control_arch = subprocess.check_output(
                    ["dpkg-deb", "-f", str(package), "Architecture"], text=True,
                ).strip()
                self.assertEqual(control_arch, deb_arch)
                unpacked = repo / "unpacked"
                subprocess.run(["dpkg-deb", "-x", str(package), str(unpacked)], check=True)
                installed = unpacked / "usr/bin/sidera"
                self.assertEqual(installed.read_bytes(), contents)
                self.assertEqual(installed.stat().st_mode & 0o777, 0o755)

    def test_wrong_architecture_is_rejected_in_both_locations(self):
        for arch, machine, container in (
            ("arm64", 62, "aarch64"), ("aarch64", 62, "aarch64"),
            ("amd64", 183, "amd64"),
        ):
            for location in ("release", f"container-{container}/release"):
                with self.subTest(arch=arch, location=location):
                    repo = self.make_repo()
                    self.place_binary(repo, location, elf_fixture(machine))
                    self.assert_rejected(repo, arch)

    def test_invalid_elf_is_rejected(self):
        for contents in (b"not an ELF executable\n", b"\x7fELF\x02\x01"):
            with self.subTest(contents=contents):
                repo = self.make_repo()
                self.place_binary(repo, "release", contents)
                self.assert_rejected(repo, "amd64")

    def test_incompatible_elf_format_is_rejected(self):
        for options in ({"elf_class": 1}, {"byte_order": 2}, {"kind": 1}):
            with self.subTest(options=options):
                repo = self.make_repo()
                self.place_binary(repo, "release", elf_fixture(62, **options))
                self.assert_rejected(repo, "amd64")

    def test_missing_binary_is_rejected(self):
        self.assert_rejected(self.make_repo(), "arm64")


if __name__ == "__main__":
    unittest.main(verbosity=2)
