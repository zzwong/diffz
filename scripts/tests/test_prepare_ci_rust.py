#!/usr/bin/env python3
"""Offline setup regressions; the compiler fixtures are not performance trials."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "prepare-ci-rust.py"
SPEC = importlib.util.spec_from_file_location("prepare_ci_rust", SCRIPT)
setup = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(setup)

FAKE_TOOL = r'''#!PYTHON
import json
import os
from pathlib import Path
import shutil
import sys

home = Path(os.environ["RUSTUP_HOME"])
args = sys.argv[1:]
name = Path(sys.argv[0]).name
with open(os.environ["FAKE_LOG"], "a") as log:
    log.write(json.dumps({"name": name, "args": args, "home": str(home)}) + "\n")
if name == "rustc":
    channel = (home / "selected").read_text()
    if Path(sys.argv[0]).parent == Path(os.environ["FAKE_BIN"]):
        channel = os.environ.get("FAKE_PATH_VERSION", channel)
    print(f"rustc {channel}\nbinary: rustc\ncommit-hash: fixture-{channel}\nhost: fixture-host\nrelease: {channel}")
elif args[:2] == ["toolchain", "install"]:
    if os.environ.get("FAKE_INSTALL_FAIL"):
        sys.exit(17)
    channel = args[-1]
    (home / "selected").write_text(channel)
    compiler = home / "toolchains" / (channel + "-fixture-host") / "bin" / "rustc"
    compiler.parent.mkdir(parents=True)
    shutil.copy2(sys.argv[0], compiler)
elif args[:3] == ["toolchain", "list", "--quiet"]:
    for child in sorted((home / "toolchains").iterdir()):
        print(child.name)
    if os.environ.get("FAKE_EXTRA_TOOLCHAIN"):
        print("unused-1.99.1-fixture-host")
elif args == ["show", "active-toolchain"]:
    print(os.environ.get("FAKE_ACTIVE", (home / "selected").read_text() + "-fixture-host") + " (overridden by rust-toolchain.toml)")
elif args == ["which", "rustc"]:
    compiler = home / "toolchains" / ((home / "selected").read_text() + "-fixture-host") / "bin" / "rustc"
    print(os.environ.get("FAKE_COMPILER", str(compiler)))
else:
    raise SystemExit("unexpected fake rustup invocation: " + repr(args))
'''


class PrepareRustTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="diffz rust setup ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.config = self.repo / "rust-toolchain.toml"
        self.config.write_text(
            '[toolchain]\nchannel = "1.99.0"\nprofile = "minimal"\n'
            'components = ["rustfmt", "clippy"]\ntargets = ["fixture-target"]\n'
        )
        self.runner = self.root / "runner temp"
        self.runner.mkdir()
        self.env_file = self.runner / "set env"
        self.env_file.write_text("EXISTING=preserved\n")
        self.bin = self.root / "bin"
        self.bin.mkdir()
        for name in ("rustup", "rustc"):
            tool = self.bin / name
            tool.write_text(FAKE_TOOL.replace("#!PYTHON", "#!" + sys.executable, 1))
            tool.chmod(0o755)
        self.shared = self.root / "image rustup"
        self.shared.mkdir()
        (self.shared / "unused-stable").write_text("1.99.1 image sentinel")
        self.log = self.root / "commands.jsonl"
        self.env = dict(os.environ)
        for key in ("RUSTUP_TOOLCHAIN", "RUSTC"):
            self.env.pop(key, None)
        self.env.update(
            PATH=str(self.bin) + os.pathsep + self.env.get("PATH", ""),
            RUNNER_TEMP=str(self.runner), GITHUB_ENV=str(self.env_file),
            RUSTUP_HOME=str(self.shared), FAKE_LOG=str(self.log), FAKE_BIN=str(self.bin),
        )

    def prepare(self):
        return setup.prepare(self.repo, self.env)

    def assert_no_export(self):
        self.assertEqual(self.env_file.read_text(), "EXISTING=preserved\n")

    def test_owned_home_preserves_image_and_pinned_configuration(self):
        home = self.prepare()
        self.assertEqual(home, self.runner / "diffz-rustup")
        self.assertEqual((self.shared / "unused-stable").read_text(), "1.99.1 image sentinel")
        self.assertEqual(list(self.shared.iterdir()), [self.shared / "unused-stable"])
        commands = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertTrue(all(command["home"] == str(home) for command in commands))
        self.assertEqual(commands[0]["args"], [
            "toolchain", "install", "--no-self-update", "--profile=minimal",
            "--component=rustfmt", "--component=clippy", "--target=fixture-target", "--", "1.99.0",
        ])
        self.assertEqual(
            self.env_file.read_text(), f"EXISTING=preserved\n\nRUSTUP_HOME={home}\n"
        )

    def test_existing_home_is_never_reused_or_deleted(self):
        home = self.runner / "diffz-rustup"
        home.mkdir()
        sentinel = home / "keep"
        sentinel.write_text("another step owns this")
        with self.assertRaisesRegex(setup.SetupError, "refusing existing"):
            self.prepare()
        self.assertEqual(sentinel.read_text(), "another step owns this")
        self.assertFalse(self.log.exists())
        self.assert_no_export()

    def test_symlink_home_is_rejected(self):
        (self.runner / "diffz-rustup").symlink_to(self.shared, target_is_directory=True)
        with self.assertRaisesRegex(setup.SetupError, "refusing existing"):
            self.prepare()
        self.assertFalse(self.log.exists())
        self.assert_no_export()

    def test_environment_file_outside_runner_is_rejected(self):
        outside = self.root / "outside env"
        outside.write_text("keep")
        self.env["GITHUB_ENV"] = str(outside)
        with self.assertRaisesRegex(setup.SetupError, "inside RUNNER_TEMP"):
            self.prepare()
        self.assertEqual(outside.read_text(), "keep")
        self.assertFalse((self.runner / "diffz-rustup").exists())

    def test_line_break_in_export_path_is_rejected(self):
        self.env["RUNNER_TEMP"] += "\nINJECTED=value"
        with self.assertRaisesRegex(setup.SetupError, "without line breaks"):
            self.prepare()
        self.assert_no_export()

    def test_inherited_toolchain_override_is_rejected(self):
        self.env["RUSTUP_TOOLCHAIN"] = "stable"
        with self.assertRaisesRegex(setup.SetupError, "unset RUSTUP_TOOLCHAIN"):
            self.prepare()
        self.assertFalse(self.log.exists())
        self.assert_no_export()

    def test_failed_install_does_not_publish_environment(self):
        self.env["FAKE_INSTALL_FAIL"] = "1"
        with self.assertRaises(setup.subprocess.CalledProcessError) as raised:
            self.prepare()
        self.assertEqual(raised.exception.returncode, 17)
        self.assert_no_export()

    def test_multiple_installed_compilers_are_rejected(self):
        self.env["FAKE_EXTRA_TOOLCHAIN"] = "1"
        with self.assertRaisesRegex(setup.SetupError, "expected one installed"):
            self.prepare()
        self.assert_no_export()

    def test_different_active_compiler_is_rejected(self):
        self.env["FAKE_ACTIVE"] = "unused-stable"
        with self.assertRaisesRegex(setup.SetupError, "active toolchain differs"):
            self.prepare()
        self.assert_no_export()

    def test_unowned_compiler_is_rejected(self):
        self.env["FAKE_COMPILER"] = str(self.bin / "rustc")
        with self.assertRaisesRegex(setup.SetupError, "inside the job's Rust home"):
            self.prepare()
        self.assert_no_export()

    def test_path_compiler_mismatch_is_rejected(self):
        self.env["FAKE_PATH_VERSION"] = "1.99.1"
        with self.assertRaisesRegex(setup.SetupError, "rustc on PATH"):
            self.prepare()
        self.assert_no_export()

    def test_observed_inventory_and_home_changes_reproduce_hash_behavior(self):
        # Exact upstream v2.9.2 getRustVersions/hash sequence from the two job logs.
        pinned = "1.99.0 x86_64-unknown-linux-gnu b940084d7eb6a299eb4bfeb8e34901bc051e7ac4"
        unused = "1.99.1 x86_64-unknown-linux-gnu 58cf8f9c35b355199727627d07ae34a4e2846d67"

        def key(versions, rustup_home=None):
            data = "".join(sorted(set(versions))) + "CARGO_INCREMENTAL=0"
            if rustup_home:
                data += f"RUSTUP_HOME={rustup_home}"
            return hashlib.sha1(data.encode()).hexdigest()[:8]

        self.assertEqual(key([pinned]), "b9e20461")
        self.assertEqual(key([pinned, unused]), "e3a6d410")
        stable_home = "/home/runner/work/_temp/diffz-rustup"
        self.assertNotEqual(key([pinned], stable_home), key([unused], stable_home))
        self.assertNotEqual(key([pinned], stable_home), key([pinned], stable_home + "-random"))


if __name__ == "__main__":
    unittest.main()
