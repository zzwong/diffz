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
        self.prefix = f"{self.name}-{m.VERSION}"

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
        return m.unpack_verified(path, self.root / "out", self.name, m.sha256(path.read_bytes()))

    def lock(self):
        return {"package": [{"name": n, "version": m.VERSION, "checksum": "1" * 64,
                 "source": "registry+https://github.com/rust-lang/crates.io-index"} for n in m.CRATES]}

    def test_exact_locked_packages(self):
        self.assertEqual(set(m.locked_crates(self.lock())), set(m.CRATES))

    def test_version_source_and_checksum_changes_fail(self):
        for key, value in (("version", "0.135.8"), ("source", "git+https://example.invalid"),
                           ("checksum", "bad")):
            with self.subTest(key=key):
                lock = self.lock()
                lock["package"][0][key] = value
                with self.assertRaises(m.VerificationError):
                    m.locked_crates(lock)

    def test_duplicate_locked_version_fails(self):
        lock = self.lock()
        lock["package"].append(dict(lock["package"][0]))
        with self.assertRaises(m.VerificationError):
            m.locked_crates(lock)

    def test_unpack_verified_regular_source(self):
        path = self.archive([(f"{self.prefix}/src/lib.rs", "file")])
        root = self.unpack(path)
        self.assertEqual((root / "src/lib.rs").read_bytes(), b"source\n")

    def test_invalid_archive_hash_writes_nothing(self):
        path = self.archive([(f"{self.prefix}/build.rs", "file")])
        with self.assertRaises(m.VerificationError):
            m.unpack_verified(path, self.root / "out", self.name, "0" * 64)
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
