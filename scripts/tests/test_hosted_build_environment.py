#!/usr/bin/env python3
"""Hosted native identity data/CLI/wiring fixtures; never launch external tools."""

from contextlib import redirect_stderr, redirect_stdout
import copy
import importlib.util
import io
import json
import os
from pathlib import Path
import re
import tempfile
import unittest
from unittest.mock import patch


REPO = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location(
    "hosted_build_environment", REPO / "scripts/debian-build-environment.py")
identity = importlib.util.module_from_spec(spec)
spec.loader.exec_module(identity)
IMAGE = {"ImageOS": "ubuntu24", "ImageVersion": "20261018.339.1"}
PACKAGES = "libc6\t2.39-0ubuntu8.6\tamd64\tinstalled\nclang\t18.0\tamd64\tinstalled\n"
TOOL = {"sha256": "a" * 64, "version": "compiler fixture 1.0"}


class HostedIdentityTests(unittest.TestCase):
    def setUp(self):
        # Any unexpected command invocation fails the fixture instead of touching
        # the host. Probe responses are data; compiler execution is not mocked
        # as a claim about the real build environment.
        blocked = patch.object(identity.subprocess, "run", side_effect=AssertionError("external command"))
        blocked.start()
        self.addCleanup(blocked.stop)
        self.packages = PACKAGES
        self.architecture = "amd64"
        self.target = "x86_64-linux-gnu"
        self.tools = {name: dict(TOOL) for name in identity.TOOLS}
        capture = patch.object(identity, "capture", side_effect=self.capture)
        capture.start()
        self.addCleanup(capture.stop)
        tools = patch.object(identity, "tool_identity", side_effect=lambda name, _: dict(self.tools[name]))
        tools.start()
        self.addCleanup(tools.stop)

    def capture(self, argv):
        if argv == ["dpkg", "--print-architecture"]:
            return self.architecture
        if argv == ["dpkg-query", "-W", f"-f={identity.PACKAGE_FORMAT}"]:
            return self.packages
        if argv == ["cc", "-dumpmachine"]:
            return self.target
        self.fail(f"unexpected probe: {argv}")

    def digest(self, environment=None):
        return identity.identity_digest(identity.hosted_identity(IMAGE if environment is None else environment))

    def test_stable_across_commits_checkouts_and_unrelated_environment(self):
        environment = {**IMAGE, "GITHUB_SHA": "another-commit", "GITHUB_WORKSPACE": "/another/checkout",
                       "SECRET_TOKEN": "must-not-appear"}
        self.assertEqual(self.digest(), self.digest(environment))
        receipt = json.dumps(identity.hosted_identity(environment))
        for value in ("another-commit", "/another/checkout", "must-not-appear"):
            self.assertNotIn(value, receipt)

    def test_runner_image_and_version_independently_change_identity(self):
        for key, value in (("ImageOS", "ubuntu26"), ("ImageVersion", "20261019.1.1")):
            with self.subTest(key=key):
                self.assertNotEqual(self.digest(), self.digest({**IMAGE, key: value}))

    def test_missing_or_malformed_image_metadata_fails_before_probing(self):
        with patch.object(identity, "native_environment", side_effect=AssertionError("unexpected probe")):
            for key in IMAGE:
                for value in ("", " ", "one\ntwo", "x" * 129, "../private"):
                    with self.subTest(key=key, value=value):
                        with self.assertRaises(ValueError):
                            identity.hosted_identity({**IMAGE, key: value})
                environment = dict(IMAGE)
                del environment[key]
                with self.assertRaises(ValueError):
                    identity.hosted_identity(environment)

    def test_non_ubuntu_runner_is_not_silently_qualified(self):
        with patch.object(identity, "native_environment", side_effect=AssertionError("unexpected probe")):
            with self.assertRaisesRegex(ValueError, "Ubuntu"):
                identity.hosted_identity({**IMAGE, "ImageOS": "macos15"})

    def test_package_order_and_removed_packages_leave_identity_unchanged(self):
        original = self.digest()
        self.packages = "removed\t1\tamd64\tconfig-files\n" + "\n".join(reversed(PACKAGES.splitlines()))
        self.assertEqual(original, self.digest())

    def test_package_change_invalidates_without_changing_image_label(self):
        original = self.digest()
        self.packages = PACKAGES.replace("2.39-0ubuntu8.6", "2.39-0ubuntu8.7")
        self.assertNotEqual(original, self.digest())

    def test_actual_native_or_rust_tool_bytes_invalidate(self):
        original = self.digest()
        for name in ("rustc", "cc", "clang", "ld"):
            with self.subTest(name=name):
                self.tools[name]["sha256"] = "b" * 64
                self.assertNotEqual(original, self.digest())
                self.tools[name] = dict(TOOL)

    def test_target_and_architecture_invalidate(self):
        original = self.digest()
        self.target = "aarch64-linux-gnu"
        self.assertNotEqual(original, self.digest())
        self.target = "x86_64-linux-gnu"
        self.architecture = "arm64"
        self.assertNotEqual(original, self.digest())

    def test_malformed_native_inventory_is_not_a_usable_identity(self):
        for value in ("", "malformed", PACKAGES * 2):
            with self.subTest(value=value):
                self.packages = value
                with self.assertRaises(ValueError):
                    self.digest()

    def test_container_schema_and_digest_are_preserved(self):
        image_id = "sha256:" + "1" * 64
        expected = {
            "schema_version": 1, "base_image_id": image_id,
            "architecture": self.architecture,
            "packages": identity.installed_packages(PACKAGES),
            "tools": copy.deepcopy(self.tools), "cc_target": self.target,
        }
        observed = identity.native_identity(image_id)
        self.assertEqual(expected, observed)
        self.assertEqual(identity.identity_digest(expected), identity.identity_digest(observed))
        self.assertNotEqual(identity.identity_digest(observed), self.digest())

    def run_cli(self, output, environment=IMAGE, extra=()):
        stdout, stderr = io.StringIO(), io.StringIO()
        argv = ["debian-build-environment.py", "--hosted-ubuntu", "--output", str(output), *extra]
        with patch.object(identity.sys, "argv", argv), patch.dict(os.environ, environment, clear=True), \
                redirect_stdout(stdout), redirect_stderr(stderr):
            status = identity.main()
        return status, stdout.getvalue(), stderr.getvalue()

    def test_cli_writes_complete_receipt_before_returning_digest(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "receipt.json"
            status, stdout, stderr = self.run_cli(output)
            self.assertEqual(status, 0, stderr)
            receipt = json.loads(output.read_text())
            self.assertEqual(stdout, receipt["sha256"] + "\n")
            self.assertEqual(receipt["sha256"], self.digest())
            self.assertEqual(receipt["identity"]["runner_image"], IMAGE)

    def test_cli_failure_emits_no_digest_or_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "receipt.json"
            status, stdout, _ = self.run_cli(output, {})
            self.assertNotEqual(status, 0)
            self.assertEqual(stdout, "")
            self.assertFalse(output.exists())

    def test_cli_preserves_existing_receipt_and_rejects_symlink(self):
        with tempfile.TemporaryDirectory() as directory:
            original = Path(directory) / "existing.json"
            original.write_text("preserved")
            link = Path(directory) / "symlink.json"
            link.symlink_to(original)
            for output in (original, link):
                with self.subTest(output=output):
                    status, stdout, _ = self.run_cli(output)
                    self.assertNotEqual(status, 0)
                    self.assertEqual(stdout, "")
                    self.assertEqual(original.read_text(), "preserved")

    def test_cli_requires_one_identity_source(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "receipt.json"
            with self.assertRaises(SystemExit) as error:
                self.run_cli(output, extra=("--image-id", "sha256:" + "1" * 64))
            self.assertEqual(error.exception.code, 2)
            self.assertFalse(output.exists())


class HostedWorkflowTests(unittest.TestCase):
    def setUp(self):
        self.document = (REPO / ".github/workflows/ci.yml").read_text()
        self.jobs = dict(re.findall(r"^  ([a-z]+):\n(.*?)(?=^  [a-z]+:|\Z)", self.document, re.M | re.S))
        self.native = self.jobs["native"]

    def test_all_native_package_and_toolchain_preparation_precedes_identity_and_cache(self):
        order = [self.native.index(text) for text in (
            "name: Install GPUI build dependencies", "name: Install pinned Rust toolchain",
            "name: Identify prepared native build environment", "name: Cache Cargo",
            "name: Test every crate with all features")]
        self.assertEqual(order, sorted(order))
        before, after = self.native.split("name: Identify prepared native build environment", 1)
        self.assertIn("desktop-file-utils", before)
        self.assertNotIn("apt-get", after)

    def test_native_digest_is_a_prefix_not_a_lockfile_only_suffix(self):
        self.assertIn("prefix-key: hosted-native-v1-${{ steps.native-environment.outputs.sha256 }}", self.native)
        self.assertIn("id: native-environment", self.native)
        self.assertIn("--hosted-ubuntu --output", self.native)
        self.assertIn('[[ "$digest" =~ ^[0-9a-f]{64}$ ]]', self.native)
        self.assertLess(self.native.index('cat "$receipt"'), self.native.index("printf 'sha256="))
        self.assertNotIn("continue-on-error", self.native)
        self.assertNotIn("shared-key:", self.native)

    def test_other_jobs_do_not_claim_hosted_native_identity(self):
        for job in ("check", "macos"):
            with self.subTest(job=job):
                self.assertNotIn("prefix-key:", self.jobs[job])
                self.assertNotIn("--hosted-ubuntu", self.jobs[job])
        self.assertIn("python3 scripts/tests/test_hosted_build_environment.py -v", self.jobs["check"])

    def test_native_validation_commands_remain_required(self):
        for command in ("cargo test --locked --workspace --all-features",
                        "cargo clippy --locked --workspace --all-features --all-targets -- -D warnings",
                        'make install-dev BIN="$bin" "${dirs[@]}"',
                        'make uninstall-dev "${dirs[@]}"', "desktop-file-validate"):
            with self.subTest(command=command):
                self.assertIn(command, self.native)


if __name__ == "__main__":
    unittest.main()
