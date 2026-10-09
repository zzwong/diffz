#!/usr/bin/env python3
"""Check fail-closed smoke orchestration without RPM tools, root, or a build.

Package-manager and ldd commands are mocked. The fresh Fedora release job
covers successful installation and the real installed binary/payload.
"""

import hashlib
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest


REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "scripts/smoke-linux-rpm.sh"
PACKAGE = "diffz-0.3.1-1.fc44.x86_64.rpm"
IDENTITY = "diffz-0.3.1-1.fc44.x86_64"


class RpmSmokeTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix="diffz-rpm-smoke-")
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.packages = self.root / "packages with spaces"
        self.packages.mkdir()
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.log = self.root / "install-args"
        self.package = self.packages / PACKAGE
        self.package.write_bytes(b"mock RPM fixture\n")
        self.manifest(self.package)
        self.command("dnf", """#!/bin/sh
printf '%s\\n' "$@" > "$INSTALL_LOG"
exit "${INSTALL_STATUS:-0}"
""")
        self.command("rpm", """#!/bin/sh
if [ "$1" = -qp ] && [ "$3" = '%{NAME}' ]; then
  printf '%s' "${PACKAGE_NAME:-diffz}"
elif [ "$1" = -qp ]; then
  printf '%s' "$PACKAGE_IDENTITY"
else
  printf '%s' "${INSTALLED_IDENTITY:-$PACKAGE_IDENTITY}"
fi
exit "${RPM_STATUS:-0}"
""")
        # Every otherwise-successful mock run stops here, before any absolute
        # installed-binary invocation. Never execute a host's /usr/bin/diffz.
        self.command("ldd", """#!/bin/sh
printf '%s\\n' "${LDD_OUTPUT:-mock loader failure}"
exit "${LDD_STATUS:-71}"
""")
        self.env = {key: value for key, value in os.environ.items()
                    if key not in ("BASH_ENV", "ENV")}
        self.env.update(
            PATH=f"{self.bin}{os.pathsep}{self.env['PATH']}",
            INSTALL_LOG=str(self.log),
            PACKAGE_IDENTITY=IDENTITY,
            INSTALL_STATUS="0",
            RPM_STATUS="0",
            PACKAGE_NAME="diffz",
            INSTALLED_IDENTITY=IDENTITY,
            LDD_STATUS="71",
            LDD_OUTPUT="mock loader failure",
        )

    def command(self, name, source):
        command = self.bin / name
        command.write_text(source)
        command.chmod(0o755)

    def manifest(self, package):
        digest = hashlib.sha256(package.read_bytes()).hexdigest()
        (self.packages / "SHA256SUMS").write_text(f"{digest}  {package.name}\n")

    def smoke(self, **overrides):
        return subprocess.run(
            ["bash", str(SCRIPT), str(self.packages)],
            cwd=self.root, env={**self.env, **overrides},
            text=True, capture_output=True,
        )

    def assert_no_install(self, result):
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse(self.log.exists(), result.stdout + result.stderr)

    def test_rejects_no_rpm(self):
        self.package.unlink()
        self.assert_no_install(self.smoke())

    def test_rejects_ambiguous_rpms(self):
        (self.packages / "second.rpm").write_bytes(b"another RPM")
        self.assert_no_install(self.smoke())

    def test_rejects_missing_manifest(self):
        (self.packages / "SHA256SUMS").unlink()
        self.assert_no_install(self.smoke())

    def test_rejects_corrupted_rpm_before_install(self):
        self.package.write_bytes(b"corrupt RPM")
        self.assert_no_install(self.smoke())

    def test_rejects_manifest_that_only_checks_tarball(self):
        archive = self.packages / "diffz.tar.gz"
        archive.write_bytes(b"mock archive")
        self.manifest(archive)
        result = self.smoke()
        self.assert_no_install(result)
        self.assertIn("RPM checksum is missing", result.stderr)

    def test_rejects_malformed_manifest(self):
        with (self.packages / "SHA256SUMS").open("a") as manifest:
            manifest.write("not a checksum\n")
        self.assert_no_install(self.smoke())

    def test_rejects_other_package_name(self):
        self.assert_no_install(self.smoke(PACKAGE_NAME="unrelated"))

    def test_rejects_failed_rpm_query_even_with_matching_output(self):
        self.assert_no_install(self.smoke(RPM_STATUS="19"))

    def test_installs_exact_artifact_with_only_required_dependencies(self):
        result = self.smoke()
        self.assertEqual(result.returncode, 71, result.stdout + result.stderr)
        self.assertEqual(self.log.read_text().splitlines(), [
            "install", "-y", "--setopt=install_weak_deps=False",
            "--setopt=tsflags=", str(self.package),
        ])

    def test_propagates_failed_install(self):
        result = self.smoke(INSTALL_STATUS="23")
        self.assertEqual(result.returncode, 23, result.stdout + result.stderr)

    def test_rejects_installed_identity_mismatch(self):
        result = self.smoke(INSTALLED_IDENTITY="diffz-0.2.0-1.fc44.x86_64")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("does not match", result.stderr)

    def test_propagates_ldd_error(self):
        result = self.smoke(LDD_STATUS="29")
        self.assertEqual(result.returncode, 29, result.stdout + result.stderr)

    def test_rejects_missing_library_even_if_ldd_exits_successfully(self):
        result = self.smoke(LDD_STATUS="0", LDD_OUTPUT="libmissing.so => not found")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unresolved shared libraries", result.stderr)


class ReleaseWiringTests(unittest.TestCase):
    def test_fresh_smoke_consumes_build_artifact_and_gates_existing_publish(self):
        workflow = (REPO / ".github/workflows/linux-release.yml").read_text()

        def job(name):
            match = re.search(
                rf"(?ms)^  {re.escape(name)}:\n(.*?)(?=^  \S|\Z)", workflow)
            self.assertIsNotNone(match, name)
            return match.group(1)

        smoke = job("linux-smoke")
        self.assertIn("    needs: linux\n", smoke)
        self.assertIn("    container: fedora:44\n", smoke)
        self.assertIn("name: diffz-linux-release\n", smoke)
        self.assertIn("run: bash scripts/smoke-linux-rpm.sh dist\n", smoke)
        self.assertNotIn("continue-on-error", smoke)
        self.assertNotIn("cargo ", smoke)
        self.assertNotIn("dnf ", smoke)
        publish = job("publish")
        needs = re.search(r"(?m)^    needs: \[([^\]]+)\]$", publish)
        self.assertIsNotNone(needs)
        self.assertIn("linux-smoke", [name.strip() for name in needs.group(1).split(",")])
        self.assertNotIn("always()", publish)
        self.assertEqual(workflow.count('gh release create '), 1)


if __name__ == "__main__":
    unittest.main()
