#!/usr/bin/env python3
"""Guard trial measurements against missing tests, unsafe snapshots and timeouts."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import sys

SCRIPT = Path(__file__).resolve().parents[1] / "benchmark-ci-builds.py"
spec = importlib.util.spec_from_file_location("benchmark_ci", SCRIPT)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class Controls(unittest.TestCase):
    def test_libtest_membership_and_duplicate_names(self):
        result = module.rust_tests("test foo ... ok\ntest perf ... ignored, benchmark\ntest foo ... ok\n")
        self.assertEqual(result, {"foo: ok": 2, "perf: ignored": 1})

    def test_missing_and_failed_execution_cannot_pass(self):
        for text in ("cached summary only", "test foo ... FAILED\n", "running 0 tests"):
            with self.subTest(text=text), self.assertRaises(RuntimeError):
                module.rust_tests(text)

    def test_snapshot_paths_are_constrained(self):
        module.safe_members(["target/debug/", "target/debug/deps/lib.so"], ["target/debug"])
        for name in ("/etc/passwd", "target/debug/../../etc/passwd", "target/other", "repositories/x"):
            with self.subTest(name=name), self.assertRaises(RuntimeError):
                module.safe_members([name], ["target/debug"])

    def trial(self, root):
        return patch.object(module, "ROOT", root)

    def test_nonzero_exit_is_recorded_and_fails(self):
        with tempfile.TemporaryDirectory(prefix="diffz-ci-guards-") as tmp, self.trial(Path(tmp)):
            with patch.object(module, "read_command", return_value="fake-revision"):
                trial = module.Trial("cargo", "seed", 1)
            with self.assertRaises(RuntimeError):
                trial.run("failure", [sys.executable, "-c", "raise SystemExit(17)"])
            self.assertEqual(trial.result["commands"][-1]["exit_code"], 17)
            self.assertFalse(trial.result["complete"])

    def test_timeout_is_not_a_successful_sample(self):
        with tempfile.TemporaryDirectory(prefix="diffz-ci-guards-") as tmp, self.trial(Path(tmp)):
            with patch.object(module, "read_command", return_value="fake-revision"):
                trial = module.Trial("bazel", "seed", 1)
            with self.assertRaises(RuntimeError):
                trial.run("timeout", [sys.executable, "-c", "import time; time.sleep(10)"], timeout=0.05)
            self.assertTrue(trial.result["commands"][-1]["timed_out"])
            self.assertFalse(trial.result["complete"])

    def test_kache_and_cargo_use_distinct_snapshot_contents(self):
        with tempfile.TemporaryDirectory(prefix="diffz-ci-guards-") as tmp, self.trial(Path(tmp)):
            with patch.object(module, "read_command", return_value="fake-revision"):
                cargo = module.Trial("cargo", "seed", 1)
                kache = module.Trial("cargo-kache", "consumer", 1)
            self.assertNotIn("target/ci-cache/kache-store", cargo.workspace_paths())
            self.assertIn("target/ci-cache/kache-store", kache.workspace_paths())


if __name__ == "__main__":
    unittest.main()
