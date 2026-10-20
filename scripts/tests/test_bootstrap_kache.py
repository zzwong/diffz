#!/usr/bin/env python3
"""Exercise cross-process tool publication and failure boundaries without builds."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest


REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "scripts" / "bootstrap-kache.py"
HOST = "x86_64-unknown-linux-gnu"
VERSION = "0.21.0"


class BootstrapTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="diffz-kache-test-")
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.fake_bin = self.base / "commands"
        self.fake_bin.mkdir()
        self.log = self.base / "cargo.jsonl"
        self.root = self.base / "shared tools" / VERSION / HOST
        self.env = {
            "PATH": str(self.fake_bin) + os.pathsep + os.environ["PATH"],
            "HOME": str(self.base / "home"),
            "XDG_CACHE_HOME": str(self.base / "shared cache"),
            "FAKE_LOG": str(self.log),
            "RUSTC_WRAPPER": "/unrelated/wrapper",
            "RUSTC_WORKSPACE_WRAPPER": "/unrelated/workspace-wrapper",
            "CARGO_BUILD_TARGET": "aarch64-unknown-linux-gnu",
            "CARGO_TARGET_DIR": str(self.base / "unrelated target"),
            "CARGO_BUILD_BUILD_DIR": str(self.base / "unrelated build"),
        }
        self.write_executable(
            self.fake_bin / "rustc",
            f"print('rustc 1.99.0\\nhost: {HOST}')\n",
        )
        self.write_executable(self.fake_bin / "cargo", '''
import json, os
from pathlib import Path
import sys, time
args = sys.argv[1:]
record = {"args": args, "cwd": os.getcwd(), "env": {key: os.environ.get(key) for key in (
    "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "CARGO_TARGET_DIR",
    "CARGO_BUILD_BUILD_DIR", "CARGO_BUILD_TARGET")}}
with open(os.environ["FAKE_LOG"], "a") as log:
    log.write(json.dumps(record) + "\\n")
if args[0] != "install":
    raise SystemExit(0)
root = Path(args[args.index("--root") + 1])
binary = root / "bin" / "kache"
binary.parent.mkdir(parents=True)
version = os.environ.get("FAKE_VERSION", args[args.index("--version") + 1])
binary.write_text("#!" + sys.executable + "\\nprint(" + repr("kache " + version) + ")\\n")
binary.chmod(0o755)
if marker := os.environ.get("FAKE_STARTED"):
    Path(marker).touch()
if gate := os.environ.get("FAKE_GATE"):
    deadline = time.monotonic() + 15
    while not Path(gate).exists():
        if time.monotonic() > deadline:
            raise SystemExit(99)
        time.sleep(0.01)
raise SystemExit(int(os.environ.get("FAKE_EXIT", "0")))
''')

    def write_executable(self, path, body):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(f"#!{sys.executable}\n" + body)
        path.chmod(0o755)

    def command(self, *, root=None, binary=None, host=HOST):
        root = root or self.root
        return [sys.executable, str(SCRIPT), "--version", VERSION,
                "--host", host, "--root", str(root), "--bin",
                str(binary or root / "bin" / "kache")]

    def run_bootstrap(self, *, environment=None, cwd=None, **kwargs):
        env = self.env | (environment or {})
        return subprocess.run(self.command(**kwargs), env=env, cwd=cwd,
                              capture_output=True, text=True, timeout=20)

    def records(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []

    def assert_success(self, result):
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def wait_for(self, path):
        deadline = time.monotonic() + 10
        while not path.exists():
            if time.monotonic() > deadline:
                self.fail(f"timed out waiting for {path}")
            time.sleep(0.01)

    def test_install_is_pinned_host_targeted_and_isolates_compiler_outputs(self):
        self.assert_success(self.run_bootstrap())
        record, = self.records()
        args = record["args"]
        self.assertEqual(args[:4], ["install", "--locked", "--version", VERSION])
        self.assertEqual(args[args.index("--target") + 1], HOST)
        self.assertEqual(record["env"]["RUSTC_WRAPPER"], "")
        self.assertEqual(record["env"]["RUSTC_WORKSPACE_WRAPPER"], "")
        stage = Path(args[args.index("--root") + 1]).parent
        self.assertEqual(stage.parent, self.root.parent.resolve())
        self.assertEqual(Path(record["env"]["CARGO_TARGET_DIR"]), stage / "target")
        self.assertEqual(Path(record["env"]["CARGO_BUILD_BUILD_DIR"]), stage / "build")
        self.assertFalse(stage.exists())
        self.assertTrue((self.root / "bin" / "kache").is_file())
        self.assertFalse(Path(self.env["CARGO_TARGET_DIR"]).exists())

    def test_two_worktrees_reuse_tool_after_checkout_target_removal(self):
        first, second = self.base / "checkout-one", self.base / "checkout-two"
        for checkout in (first, second):
            (checkout / "target").mkdir(parents=True)
        self.assert_success(self.run_bootstrap(cwd=first))
        original = (self.root / "bin" / "kache").stat().st_ino
        shutil.rmtree(first / "target")
        self.assert_success(self.run_bootstrap(cwd=second))
        self.assertEqual(len(self.records()), 1)
        self.assertEqual((self.root / "bin" / "kache").stat().st_ino, original)
        self.assertTrue((second / "target").is_dir())

    def test_concurrent_aliases_install_once_and_never_publish_partial_tool(self):
        self.root.parent.mkdir(parents=True)
        alias = self.base / "alias"
        alias.symlink_to(self.root.parent, target_is_directory=True)
        started, gate = self.base / "started", self.base / "continue"
        env = self.env | {"FAKE_STARTED": str(started), "FAKE_GATE": str(gate)}
        processes = []
        try:
            processes.append(subprocess.Popen(self.command(), env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True))
            self.wait_for(started)
            self.assertFalse(self.root.exists())
            processes.append(subprocess.Popen(self.command(root=alias / HOST), env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True))
            time.sleep(0.15)
            self.assertIsNone(processes[1].poll())
            self.assertEqual(len(self.records()), 1)
            self.assertFalse(self.root.exists())
            gate.touch()
            for process in processes:
                output = process.communicate(timeout=20)
                self.assertEqual(process.returncode, 0, output)
            self.assertEqual(len(self.records()), 1)
        finally:
            gate.touch()
            for process in processes:
                if process.poll() is None:
                    process.kill()
                process.communicate(timeout=20)

    def test_failed_partial_install_propagates_status_and_retry_succeeds(self):
        result = self.run_bootstrap(environment={"FAKE_EXIT": "42"})
        self.assertEqual(result.returncode, 42, result.stderr)
        self.assertFalse(self.root.exists())
        self.assertEqual(list(self.root.parent.glob(f".{HOST}.install-*")), [])
        self.assert_success(self.run_bootstrap())
        self.assertEqual(len(self.records()), 2)

    def test_wrong_candidate_is_not_published(self):
        result = self.run_bootstrap(environment={"FAKE_VERSION": "1.2.2"})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("expected kache", result.stderr)
        self.assertFalse(self.root.exists())
        self.assert_success(self.run_bootstrap())

    def test_existing_wrong_version_is_preserved(self):
        binary = self.root / "bin" / "kache"
        self.write_executable(binary, "print('kache 1.2.2')\n")
        before = binary.read_bytes()
        result = self.run_bootstrap()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(binary.read_bytes(), before)
        self.assertEqual(self.records(), [])

    def test_external_override_is_read_only_and_missing_override_never_installs(self):
        binary = self.base / "external tools" / "kache"
        self.write_executable(binary, f"print('kache {VERSION}')\n")
        self.assert_success(self.run_bootstrap(binary=binary))
        result = self.run_bootstrap(binary=binary.with_name("missing"))
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.root.parent.exists())
        self.assertEqual(self.records(), [])

    def test_symlink_root_binary_directory_and_lock_are_not_followed(self):
        unrelated = self.base / "unrelated"
        self.write_executable(unrelated / "bin" / "kache", f"print('kache {VERSION}')\n")
        self.root.parent.mkdir(parents=True)
        self.root.symlink_to(unrelated, target_is_directory=True)
        self.assertNotEqual(self.run_bootstrap().returncode, 0)
        self.root.unlink()
        self.root.mkdir()
        (self.root / "bin").symlink_to(unrelated / "bin", target_is_directory=True)
        self.assertNotEqual(self.run_bootstrap().returncode, 0)
        (self.root / "bin").unlink()
        self.root.rmdir()
        lock = self.root.parent / f".{HOST}.install.lock"
        lock.unlink()
        sentinel = unrelated / "keep"
        sentinel.write_text("preserve me")
        lock.symlink_to(sentinel)
        self.assertNotEqual(self.run_bootstrap().returncode, 0)
        self.assertEqual(sentinel.read_text(), "preserve me")
        self.assertTrue((unrelated / "bin" / "kache").is_file())
        self.assertEqual(self.records(), [])

    def test_host_mismatch_does_not_install(self):
        result = self.run_bootstrap(host="aarch64-unknown-linux-gnu")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("host changed", result.stderr)
        self.assertEqual(self.records(), [])
        self.assertFalse(self.root.exists())

    def make_checkout(self, name):
        checkout = self.base / name
        (checkout / "scripts").mkdir(parents=True)
        shutil.copy2(REPO / "Makefile", checkout / "Makefile")
        for name in (SCRIPT.name, "run-kache.py", "kache.toml"):
            shutil.copy2(REPO / "scripts" / name, checkout / "scripts" / name)
        return checkout

    def run_make(self, checkout, *args, environment=None):
        return subprocess.run(["make", "--no-print-directory", *args], cwd=checkout,
                              env=self.env | (environment or {}), capture_output=True,
                              text=True, timeout=20)

    def test_make_preserves_spaces_and_uses_same_absolute_xdg_root(self):
        first = self.make_checkout("first checkout")
        second = self.make_checkout("second checkout")
        for checkout in (first, second):
            self.assert_success(self.run_make(checkout, "kache"))
        expected = Path(self.env["XDG_CACHE_HOME"]) / "diffz" / "tools" / "kache" / VERSION / HOST
        self.assertTrue((expected / "bin" / "kache").is_file())
        self.assertEqual(len(self.records()), 1)
        self.assert_success(self.run_make(second, "build"))
        build = self.records()[-1]
        self.assertEqual(build["args"], ["build", "--locked", "-p", "diffz"])
        self.assertEqual(Path(build["env"]["RUSTC_WRAPPER"]), expected / "bin" / "kache")

    def test_make_relative_xdg_falls_back_and_relative_root_override_works(self):
        checkout = self.make_checkout("checkout")
        self.assert_success(self.run_make(checkout, "kache", environment={"XDG_CACHE_HOME": "relative cache"}))
        expected = Path(self.env["HOME"]) / ".cache" / "diffz" / "tools" / "kache" / VERSION / HOST
        self.assertTrue((expected / "bin" / "kache").is_file())
        self.assert_success(self.run_make(checkout, "kache", "KACHE_ROOT=local tool"))
        self.assertTrue((checkout / "local tool" / "bin" / "kache").is_file())

    def test_plain_make_routes_clear_wrappers_without_bootstrap(self):
        checkout = self.make_checkout("plain")
        for args in (("build-plain",), ("dev-build", "KACHE=0")):
            self.assert_success(self.run_make(checkout, *args))
        self.assertEqual(len(self.records()), 2)
        for record in self.records():
            self.assertEqual(record["args"][0], "build")
            self.assertEqual(record["env"]["RUSTC_WRAPPER"], "")
            self.assertEqual(record["env"]["RUSTC_WORKSPACE_WRAPPER"], "")
        self.assertFalse(Path(self.env["XDG_CACHE_HOME"]).exists())


if __name__ == "__main__":
    unittest.main()
