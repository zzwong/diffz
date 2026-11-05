#!/usr/bin/env python3
"""Offline guard tests; real Rust generators run in verify-cranelift-generators.py."""
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest import mock

SCRIPT = Path(__file__).resolve().parents[1] / "verify-cranelift-generators.py"
spec = importlib.util.spec_from_file_location("generators", SCRIPT)
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)


class GuardTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="diffz-generator-guard-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.name = "cranelift-isle"
        self.version = "0.135.8"
        self.prefix = f"{self.name}-{self.version}"

    def archive(self, entries):
        path = self.root / "source.crate"
        with tarfile.open(path, "w:gz") as tar:
            for name, kind in entries:
                info = tarfile.TarInfo(name)
                if kind == "file":
                    data = b"source\n"
                    info.size = len(data)
                    tar.addfile(info, io.BytesIO(data))
                else:
                    info.type = tarfile.SYMTYPE
                    info.linkname = "/outside"
                    tar.addfile(info)
        return path

    def unpack(self, path):
        return m.unpack_verified(path, self.root / "out", self.name, self.version, m.sha256(path.read_bytes()))

    def lock(self):
        return {"package": [{"name": n, "version": version, "checksum": "1" * 64,
                 "source": "registry+https://github.com/rust-lang/crates.io-index"}
                for version in m.SOURCE_BLOBS for n in m.CRATES]}

    def test_all_locked_versions_are_selected_independent_of_order(self):
        lock = self.lock()
        expected = m.locked_crates(lock)
        self.assertEqual(set(expected), set(m.SOURCE_BLOBS))
        for packages in expected.values():
            self.assertEqual(set(packages), set(m.CRATES))
        self.assertEqual(m.locked_crates({"package": list(reversed(lock["package"]))}), expected)

    def test_unknown_missing_duplicate_and_invalid_packages_fail(self):
        cases = []
        lock = self.lock(); lock["package"].append(dict(lock["package"][0], version="0.999.0")); cases.append(lock)
        lock = self.lock(); lock["package"].pop(); cases.append(lock)
        lock = self.lock(); lock["package"].append(dict(lock["package"][0])); cases.append(lock)
        for key, value in (("source", "git+https://example.invalid"), ("checksum", "bad")):
            lock = self.lock(); lock["package"][-1][key] = value; cases.append(lock)
        cases.append({"package": []})
        for lock in cases:
            with self.subTest(lock=lock), self.assertRaises(m.VerificationError):
                m.locked_crates(lock)

    def test_bazel_annotations_cover_every_selected_version(self):
        versions = m.locked_crates(self.lock())
        module = (SCRIPT.parents[1] / "MODULE.bazel").read_text()
        m.verify_annotations(module, versions)
        for version in versions:
            for name in m.SOURCE_BLOBS[version]:
                line = next(line for line in module.splitlines()
                            if line.startswith(f'crate.annotation(crate = "{name}", version = "={version}",'))
                with self.subTest(version=version, name=name), self.assertRaises(m.VerificationError):
                    m.verify_annotations(module.replace(line, ""), versions)
                with self.subTest(version=version, name=name, bad_patch=True), self.assertRaises(m.VerificationError):
                    m.verify_annotations(module.replace(line, line.replace("-deterministic.patch", "-wrong.patch")), versions)

    def test_experiment_runs_each_version_and_propagates_failure(self):
        (self.root / "Cargo.lock").write_text("")
        (self.root / "MODULE.bazel").write_text("ignored")
        (self.root / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.99.0"\n')
        versions = m.locked_crates(self.lock())
        for failing in (None, "0.137.1"):
            with self.subTest(failing=failing):
                out = self.root / ("output-" + str(failing)); out.mkdir()
                work = self.root / ("work-" + str(failing)); work.mkdir()
                experiment = m.Experiment(self.root, out, "rustc")
                visited = []

                def check_version(_experiment, _cargo_home, _scratch, version, _packages):
                    visited.append(version)
                    if version == failing:
                        raise m.VerificationError("failed generator")

                with mock.patch.object(m, "locked_crates", return_value=versions), \
                     mock.patch.object(m, "verify_annotations"), \
                     mock.patch.object(m.Experiment, "run", return_value=mock.Mock(stdout=b"rustc 1.99.0 test")), \
                     mock.patch.object(m.Experiment, "verify_version", new=check_version):
                    if failing:
                        with self.assertRaises(m.VerificationError):
                            experiment.verify(self.root, work)
                    else:
                        experiment.verify(self.root, work)
                self.assertEqual(visited, ["0.135.8", "0.137.1"])
                self.assertTrue(experiment.report["versions"]["0.135.8"]["completed"])
                self.assertEqual(experiment.report["versions"]["0.137.1"]["completed"], failing is None)
                self.assertFalse(experiment.report["completed"])

    def test_unpack_verified_regular_source(self):
        path = self.archive([(f"{self.prefix}/src/lib.rs", "file")])
        root = self.unpack(path)
        self.assertEqual((root / "src/lib.rs").read_bytes(), b"source\n")

    def test_invalid_archive_hash_writes_nothing(self):
        path = self.archive([(f"{self.prefix}/build.rs", "file")])
        with self.assertRaises(m.VerificationError):
            m.unpack_verified(path, self.root / "out", self.name, self.version, "0" * 64)
        self.assertFalse((self.root / "out").exists())

    def test_traversal_absolute_and_foreign_root_rejected_before_writes(self):
        for name in (f"{self.prefix}/../../escape", "/escape", "foreign/src/lib.rs",
                     f"{self.prefix}/dir\\escape"):
            with self.subTest(name=name):
                path = self.archive([(f"{self.prefix}/valid.rs", "file"), (name, "file")])
                with self.assertRaises(m.VerificationError):
                    self.unpack(path)
                self.assertFalse((self.root / "out").exists())

    def test_links_and_duplicate_archive_entries_rejected(self):
        for entries in ([(f"{self.prefix}/link", "link")],
                        [(f"{self.prefix}/build.rs", "file")] * 2):
            with self.subTest(entries=entries):
                with self.assertRaises(m.VerificationError):
                    self.unpack(self.archive(entries))
                self.assertFalse((self.root / "out").exists())

    def test_generated_records_preserve_name_runner_and_path(self):
        data = b'#[test]\nfn test_run_pass_alpha() {\n    run_pass("isle_examples/pass/alpha.isle");\n}\n'
        self.assertEqual(m.test_records(data), [("test_run_pass_alpha", "run_pass",
                                               "isle_examples/pass/alpha.isle")])
        for bad in (b"", data + data, data + b"unexpected source\n"):
            with self.assertRaises(m.VerificationError):
                m.test_records(bad)

    def test_compiler_failure_cannot_be_success(self):
        out = self.root / "evidence"
        out.mkdir()
        e = m.Experiment(self.root, out, "rustc")
        with self.assertRaises(m.VerificationError):
            e.run("bad", [sys.executable, "-c", "raise SystemExit(7)"], self.root)
        self.assertEqual(e.report["commands"][0]["exit_code"], 7)
        self.assertFalse(e.report["completed"])
        e.run("expected-failure", [sys.executable, "-c", "raise SystemExit(7)"],
              self.root, expect_success=False)
        self.assertFalse(e.report["completed"])

    def test_existing_evidence_is_not_overwritten(self):
        out = self.root / "evidence"
        out.mkdir()
        sentinel = out / "result.json"
        sentinel.write_text('{"earlier":true}\n')
        result = subprocess.run([sys.executable, str(SCRIPT), "--output", str(out)],
                                capture_output=True, timeout=10)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(json.loads(sentinel.read_text()), {"earlier": True})


if __name__ == "__main__":
    unittest.main()
