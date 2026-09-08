#!/usr/bin/env python3
"""Exercise the real Debian tools with a tiny native ELF, without compiling GPUI."""

import io
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tarfile
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[2]
ASSETS = (
    "usr/bin/diffz",
    "usr/share/applications/io.github.zzwong.Diffz.desktop",
    "usr/share/metainfo/io.github.zzwong.Diffz.metainfo.xml",
    "usr/share/icons/hicolor/scalable/apps/io.github.zzwong.Diffz.svg",
)


class DebianPackageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="diffz-deb-test-")
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name) / "checkout with spaces"
        (self.repo / "scripts").mkdir(parents=True)
        (self.repo / "packaging/debian").mkdir(parents=True)
        for name in ("scripts/package-linux-deb.sh", "packaging/debian/control"):
            source = REPO / name
            if source.exists():
                shutil.copy2(source, self.repo / name)
        (self.repo / "Cargo.toml").write_text('[workspace.package]\nversion = "0.1.0"\n')
        (self.repo / "LICENSE").write_text("MIT license fixture\n")
        self.stage = self.repo / "stage with spaces"
        for name in ASSETS:
            path = self.stage / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(f"fixture for {name}\n")
        # A real ELF makes dpkg-shlibdeps and architecture validation meaningful.
        subprocess.run(
            ["cc", "-x", "c", "-o", str(self.stage / ASSETS[0]), "-"],
            input='#include <stdio.h>\nint main(void) { puts("diffz fixture"); return 0; }\n',
            text=True, check=True,
        )
        self.dist = self.repo / "output with spaces"
        self.env = {k: v for k, v in os.environ.items()
                    if not k.startswith(("DIFFZ_", "DEB_"))}
        self.env.update(DIFFZ_STAGE=str(self.stage), DIFFZ_DIST=str(self.dist),
                        SOURCE_DATE_EPOCH="1700000000", LC_ALL="C")

    def package(self, **overrides):
        return subprocess.run(
            ["bash", "scripts/package-linux-deb.sh"], cwd=self.repo,
            env={**self.env, **overrides}, text=True, capture_output=True,
        )

    def built_package(self, **overrides):
        result = self.package(**overrides)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        packages = list(self.dist.glob("*.deb"))
        self.assertEqual(len(packages), 1)
        return packages[0]

    def field(self, package, name):
        return subprocess.check_output(
            ["dpkg-deb", "--field", str(package), name], text=True).strip()

    def test_builds_native_package_with_real_dependencies_and_payload(self):
        package = self.built_package()
        arch = subprocess.check_output(["dpkg", "--print-architecture"], text=True).strip()
        self.assertEqual(package.name, f"diffz_0.1.0-1_{arch}.deb")
        self.assertEqual(self.field(package, "Package"), "diffz")
        self.assertEqual(self.field(package, "Architecture"), arch)
        self.assertEqual(self.field(package, "Version"), "0.1.0-1")
        depends = self.field(package, "Depends")
        self.assertRegex(depends, r"libc6 \(>= [^)]+\)")
        for dependency in ("libvulkan1", "libxkbcommon-x11-0", "libwayland-client0"):
            self.assertIn(dependency, depends)
        self.assertNotIn("-dev", depends)
        self.assertGreater(int(self.field(package, "Installed-Size")), 0)
        archive = subprocess.check_output(["dpkg-deb", "--fsys-tarfile", str(package)])
        with tarfile.open(fileobj=io.BytesIO(archive)) as payload:
            names = {entry.name.removeprefix("./"): entry for entry in payload}
            for name in (*ASSETS, "usr/share/doc/diffz/copyright"):
                self.assertIn(name, names)
            for entry in names.values():
                self.assertEqual((entry.uid, entry.gid), (0, 0))
            self.assertEqual(names[ASSETS[0]].mode & 0o777, 0o755)
            self.assertEqual(payload.extractfile(names["usr/share/doc/diffz/copyright"]).read(),
                             b"MIT license fixture\n")
        self.assertFalse((self.stage / "DEBIAN").exists())
        self.assertFalse((self.repo / "debian").exists())
        self.assertFalse(list(self.dist.glob(".deb-build.*")))

    def test_builds_under_setgid_output_without_special_directory_bits(self):
        # Shared workspaces commonly inherit SGID. umask alone cannot clear it.
        self.dist.mkdir()
        self.dist.chmod(0o2775)
        package = self.built_package()
        for option in ("--ctrl-tarfile", "--fsys-tarfile"):
            archive = subprocess.check_output(["dpkg-deb", option, str(package)])
            with tarfile.open(fileobj=io.BytesIO(archive)) as payload:
                for entry in payload:
                    if entry.isdir():
                        self.assertEqual(entry.mode & 0o7000, 0, entry.name)
        # Only the private package tree may be normalized, not the output dir.
        self.assertEqual(self.dist.stat().st_mode & 0o2777, 0o2775)
        self.assertFalse(list(self.dist.glob(".deb-build.*")))

    def test_distro_revision_and_prerelease_version(self):
        (self.repo / "Cargo.toml").write_text('[workspace.package]\nversion = "1.2.3-rc.1"\n')
        package = self.built_package(DIFFZ_DEB_REVISION="2~ubuntu24.04")
        self.assertEqual(self.field(package, "Version"), "1.2.3~rc.1-2~ubuntu24.04")

    def test_rejects_missing_staged_binary_without_removing_input(self):
        (self.stage / ASSETS[0]).unlink()
        result = self.package()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("staged executable", result.stderr)
        self.assertTrue((self.stage / ASSETS[1]).exists())
        self.assertFalse(list(self.dist.glob("*.deb")))

    def test_rejects_incomplete_desktop_payload(self):
        (self.stage / ASSETS[1]).unlink()
        result = self.package()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("staged file", result.stderr)
        self.assertFalse(list(self.dist.glob("*.deb")))

    def test_rejects_non_elf_executable(self):
        (self.stage / ASSETS[0]).write_text("#!/bin/sh\nexit 0\n")
        result = self.package()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ELF", result.stderr)

    def test_rejects_foreign_architecture_instead_of_mislabelling(self):
        binary = self.stage / ASSETS[0]
        data = bytearray(binary.read_bytes())
        endian = "<" if data[5] == 1 else ">"
        machine = struct.unpack_from(endian + "H", data, 18)[0]
        struct.pack_into(endian + "H", data, 18, 183 if machine == 62 else 62)
        binary.write_bytes(data)
        result = self.package()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("architecture", result.stderr)

    def test_rejects_invalid_version_before_creating_an_archive(self):
        result = self.package(DIFFZ_VERSION="invalid version")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("version", result.stderr)
        self.assertFalse(list(self.dist.glob("*.deb")))

    def test_rebuild_is_reproducible_with_source_date_epoch(self):
        package = self.built_package()
        first = package.read_bytes()
        self.built_package()
        self.assertEqual(package.read_bytes(), first)


if __name__ == "__main__":
    unittest.main()
