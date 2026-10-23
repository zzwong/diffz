#!/usr/bin/env python3
"""Guard the generated Linux backend's explicit Bazel package boundary."""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


def metadata(*, proc_macro: bool = False) -> dict:
    """Small Cargo-format fixture; real pinned metadata is checked separately."""
    return {
        "packages": [
            {"id": "linux", "name": "gpui-pre-linux", "version": "0.3.9",
             "features": {"x11": ["dep:x11-dep"]},
             "dependencies": [{"name": "x11-dep", "rename": "x11", "optional": True},
                              {"name": "inactive", "optional": True}]},
            {"id": "x11", "name": "x11-dep", "version": "1.2.3",
             "targets": [{"name": "x11_impl", "kind": ["proc-macro" if proc_macro else "lib"]}]},
            {"id": "inactive", "name": "inactive", "version": "1.0.0",
             "targets": [{"name": "inactive", "kind": ["lib"]}]},
            {"id": "dev", "name": "test-only", "version": "2.0.0",
             "targets": [{"name": "test_only", "kind": ["lib"]}]},
        ],
        "resolve": {"nodes": [{"id": "linux", "features": ["x11"], "deps": [
            {"name": "x11", "pkg": "x11", "dep_kinds": [{"kind": None}]},
            {"name": "inactive", "pkg": "inactive", "dep_kinds": [{"kind": None}]},
            {"name": "test_only", "pkg": "dev", "dep_kinds": [{"kind": "dev"}]},
        ]}]},
    }


class GeneratorTests(unittest.TestCase):
    def invoke(self, payload: dict, existing: str = "sentinel\n") -> tuple[subprocess.CompletedProcess, str]:
        with tempfile.TemporaryDirectory(prefix="diffz-linux-boundary-") as temporary:
            root = Path(temporary)
            (root / "scripts").mkdir()
            (root / "bazel").mkdir()
            (root / "bin").mkdir()
            shutil.copy2(ROOT / "scripts/bazel-vendor-linux.py", root / "scripts")
            (root / "metadata.json").write_text(json.dumps(payload))
            cargo = root / "bin/cargo"
            cargo.write_text(
                f"#!{sys.executable}\n"
                "import pathlib, sys\n"
                "assert sys.argv[1:] == ['metadata', '--locked', '--all-features', "
                "'--filter-platform', 'x86_64-unknown-linux-gnu', '--format-version', '1']\n"
                "print(pathlib.Path('metadata.json').read_text())\n"
            )
            cargo.chmod(0o755)
            target = root / "bazel/vendor_linux.bzl"
            target.write_text(existing)
            process = subprocess.run(
                [sys.executable, str(root / "scripts/bazel-vendor-linux.py")],
                cwd=root, env={**os.environ, "PATH": str(root / "bin") + os.pathsep + os.environ.get("PATH", "")},
                capture_output=True, text=True, timeout=10,
            )
            return process, target.read_text()

    def test_generator_uses_owned_source_labels_without_cross_package_globs(self):
        result, output = self.invoke(metadata())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('crate_root = "//vendor/gpui-pre-linux:src/gpui_linux.rs"', output)
        self.assertIn('srcs = ["//vendor/gpui-pre-linux:rust_sources"]', output)
        self.assertIn('["//vendor/gpui-pre-linux:crate_data"]', output)
        self.assertNotIn('native.glob', output)
        self.assertIn("'CARGO_MANIFEST_DIR': 'vendor/gpui-pre-linux'", output)

    def test_dependency_alias_features_and_optional_filter_are_preserved(self):
        result, output = self.invoke(metadata())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("'@@rules_rust++crate+crates__x11-dep-1.2.3//:x11_impl': 'x11'", output)
        self.assertIn("crate_features = ['x11']", output)
        self.assertNotIn('crates__inactive', output)
        self.assertNotIn('crates__test-only', output)
        self.assertIn('rustc_flags = ["-Cdebuginfo=1"]', output)

    def test_unsupported_new_proc_macro_fails_without_overwriting_output(self):
        result, output = self.invoke(metadata(proc_macro=True))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('requires a proc_macro_deps entry', result.stderr)
        self.assertEqual(output, 'sentinel\n')

    def test_render_is_deterministic(self):
        first, a = self.invoke(metadata())
        second, b = self.invoke(metadata())
        self.assertEqual((first.returncode, second.returncode), (0, 0))
        self.assertEqual(a, b)


class PackageContractTests(unittest.TestCase):
    def test_generated_package_keeps_the_annotation_source_groups(self):
        addition = (ROOT / 'bazel/gpui-linux-sources.BUILD.bazel').read_text()
        generated = (ROOT / 'vendor/gpui-pre-linux/BUILD.bazel').read_text()
        self.assertEqual(generated.split('# Additive BUILD file content\n')[1], addition)
        self.assertIn('actual = "@@//:gpui_linux"', generated)
        self.assertIn('package(default_visibility = ["//visibility:public"])', generated)
        self.assertIn('exclude = ["BUILD", "BUILD.bazel"]', addition)

    def test_manifest_is_addressed_in_its_own_package(self):
        module = (ROOT / 'MODULE.bazel').read_text()
        root = (ROOT / 'BUILD.bazel').read_text()
        self.assertIn('"//vendor/gpui-pre-linux:Cargo.toml"', module)
        self.assertNotIn('"//:vendor/gpui-pre-linux/Cargo.toml"', module)
        self.assertNotIn('"vendor/gpui-pre-linux/Cargo.toml"', root)
        self.assertIn('additive_build_file = "//bazel:gpui-linux-sources.BUILD.bazel"', module)
        self.assertIn('override_target_lib = "//:gpui_linux"', module)


if __name__ == '__main__':
    unittest.main()
