#!/usr/bin/env python3
"""Native identity and CI lifecycle regressions; no Docker or Rust is needed."""

import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "scripts/debian-build-environment.py"
WORKFLOW = REPO / ".github/workflows/debian-packages.yml"
IMAGE_ID = "sha256:" + "1" * 64
spec = importlib.util.spec_from_file_location("debian_build_environment", SCRIPT)
identity = importlib.util.module_from_spec(spec)
spec.loader.exec_module(identity)


def executable(path, source):
    path.write_text(f"#!{sys.executable}\n" + source)
    path.chmod(0o755)


class NativeIdentityTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="diffz-native-identity-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.tools = self.root / "tools with spaces"
        self.tools.mkdir()
        self.packages = "libc6\t2.36-9\tamd64\tinstalled\nclang\t14.0.6\tamd64\tinstalled\n"
        self.package_file = self.tools / "packages.txt"
        self.package_file.write_text(self.packages)
        executable(self.tools / "dpkg", 'print("amd64")\n')
        executable(self.tools / "dpkg-query", 'from pathlib import Path\nprint(Path(__file__).with_name("packages.txt").read_text(), end="")\n')
        executable(self.tools / "rustup", 'from pathlib import Path\nimport sys\nassert sys.argv[1] == "which"\nprint(Path(__file__).with_name("real-" + sys.argv[2]))\n')
        for tool in identity.TOOLS:
            name = "real-" + tool if tool in ("rustc", "cargo") else tool
            executable(self.tools / name,
                       'import sys\nprint("x86_64-linux-gnu" if "-dumpmachine" in sys.argv else "compiler fixture 1.0")\n')
        self.env = {**os.environ, "PATH": str(self.tools), "SECRET_TOKEN": "must-not-be-in-the-receipt"}
        self.counter = 0

    def run_identity(self, image_id=IMAGE_ID, output=None, **environment):
        self.counter += 1
        output = output or self.root / f"receipt-{self.counter}.json"
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "--image-id", image_id, "--output", str(output)],
            env={**self.env, **environment}, cwd=self.root, text=True, capture_output=True,
        )
        return result, output

    def receipt(self, **kwargs):
        result, output = self.run_identity(**kwargs)
        self.assertEqual(result.returncode, 0, result.stderr)
        data = json.loads(output.read_text())
        self.assertEqual(result.stdout, data["sha256"] + "\n")
        self.assertEqual(data["sha256"], identity.identity_digest(data["identity"]))
        self.assertNotIn(self.env["SECRET_TOKEN"], output.read_text())
        self.assertNotIn(str(self.root), output.read_text())
        return data

    def test_unchanged_native_environment_is_stable_across_checkout_paths(self):
        first = self.receipt()
        moved = self.root / "another checkout" / "tools"
        shutil.copytree(self.tools, moved)
        second = self.receipt(PATH=str(moved), GITHUB_SHA="different source revision")
        self.assertEqual(first, second)

    def test_package_order_and_removed_package_records_do_not_change_identity(self):
        first = self.receipt()
        self.package_file.write_text("obsolete\t1.0\tamd64\tconfig-files\n" + "\n".join(reversed(self.packages.splitlines())) + "\n")
        self.assertEqual(first, self.receipt())

    def test_package_version_change_invalidates_compiled_cache_namespace(self):
        first = self.receipt()
        self.package_file.write_text(self.packages.replace("2.36-9", "2.36-10"))
        self.assertNotEqual(first["sha256"], self.receipt()["sha256"])

    def test_new_native_package_changes_identity(self):
        first = self.receipt()
        self.package_file.write_text(self.packages + "libssl-dev\t3.0.15\tamd64\tinstalled\n")
        self.assertNotEqual(first["sha256"], self.receipt()["sha256"])

    def test_resolved_base_image_change_invalidates_identity(self):
        first = self.receipt()
        second = self.receipt(image_id="sha256:" + "2" * 64)
        self.assertNotEqual(first["sha256"], second["sha256"])

    def test_real_rust_binary_change_matters_even_when_version_is_unchanged(self):
        first = self.receipt()
        with (self.tools / "real-rustc").open("a") as binary:
            binary.write("# replacement toolchain payload with the same version label\n")
        self.assertNotEqual(first["sha256"], self.receipt()["sha256"])

    def test_hashes_actual_compiler_instead_of_rustup_proxy(self):
        first = self.receipt()
        with (self.tools / "rustup").open("a") as proxy:
            proxy.write("# only the selector changed, not the selected toolchain\n")
        self.assertEqual(first, self.receipt())

    def test_native_linker_change_invalidates_identity(self):
        first = self.receipt()
        with (self.tools / "ld").open("a") as binary:
            binary.write("# native linker replacement\n")
        self.assertNotEqual(first["sha256"], self.receipt()["sha256"])

    def test_mutable_image_tag_is_rejected_without_a_success_digest(self):
        result, output = self.run_identity(image_id="debian:12-slim")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, "")
        self.assertFalse(output.exists())

    def test_missing_native_tool_cannot_enable_cache_restore(self):
        (self.tools / "clang").unlink()
        result, output = self.run_identity()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("clang", result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertFalse(output.exists())

    def test_failed_native_probe_cannot_enable_cache_restore(self):
        executable(self.tools / "dpkg-query", 'import sys\nprint("private diagnostic", file=sys.stderr)\nsys.exit(7)\n')
        result, output = self.run_identity()
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("private diagnostic", result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertFalse(output.exists())

    def test_malformed_or_empty_package_inventory_is_rejected(self):
        for packages in ("", "bad-record\n", "libc6\t\tamd64\tinstalled\n", self.packages * 2):
            with self.subTest(packages=packages):
                self.package_file.write_text(packages)
                result, output = self.run_identity()
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(result.stdout, "")
                self.assertFalse(output.exists())

    def test_existing_receipt_is_preserved_and_no_digest_is_returned(self):
        output = self.root / "existing.json"
        output.write_text("previous evidence")
        result, _ = self.run_identity(output=output)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, "")
        self.assertEqual(output.read_text(), "previous evidence")

    def test_slow_probe_has_a_bounded_failure(self):
        with patch.object(identity.subprocess, "run", side_effect=subprocess.TimeoutExpired("cc", 30)):
            with self.assertRaisesRegex(ValueError, "TimeoutExpired"):
                identity.capture(["cc", "--version"])


def workflow_step(name):
    # Read the checked-in shell block rather than maintaining a second copy of
    # the Docker lifecycle in this test. No third-party YAML module is required.
    document = WORKFLOW.read_text()
    for step in re.split(r"(?=^      - )", document, flags=re.MULTILINE):
        if step.startswith(f"      - name: {name}\n"):
            return step
    raise AssertionError(f"missing workflow step: {name}")


def workflow_shell(name):
    step = workflow_step(name)
    block = step.split("        run: |\n", 1)[1]
    lines = []
    for line in block.splitlines():
        if line and not line.startswith("          "):
            break
        lines.append(line[10:] if line else "")
    return "\n".join(lines) + "\n"


class BuilderWorkflowTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="diffz-builder-workflow-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.trace = self.root / "docker.jsonl"
        self.output = self.root / "outputs"
        self.output.touch()
        executable(self.bin / "docker", '''import json, os, sys
from pathlib import Path
args = sys.argv[1:]
with open(os.environ["DOCKER_TRACE"], "a") as trace:
    trace.write(json.dumps(args) + "\\n")
if args[:2] == ["image", "inspect"]:
    print("sha256:" + "1" * 64)
elif args[0] == "run":
    Path(args[args.index("--cidfile") + 1]).write_text("owned-builder")
    print("owned-builder")
elif args[0] == "exec" and "-i" in args:
    sys.stdin.read()
elif args[0] == "exec":
    if os.environ.get("FAIL_IDENTITY"):
        sys.exit(8)
    print("not-a-digest" if os.environ.get("BAD_IDENTITY") else "a" * 64)
elif args[0] == "cp":
    if os.environ.get("FAIL_COPY"):
        sys.exit(9)
    Path(args[-1]).write_text('{"sha256": "' + "a" * 64 + '"}')
''')
        self.env = {
            **os.environ, "PATH": str(self.bin) + os.pathsep + os.environ["PATH"],
            "RUNNER_TEMP": str(self.root), "GITHUB_OUTPUT": str(self.output),
            "GITHUB_WORKSPACE": str(self.root), "CARGO_HOME": str(self.root / "cargo"),
            "DOCKER_TRACE": str(self.trace), "IMAGE": "debian:12-slim", "TARGET": "debian12",
        }

    def run_step(self, name, **overrides):
        return subprocess.run(["/bin/bash", "-e", "-o", "pipefail", "-c", workflow_shell(name)],
                              env={**self.env, **overrides}, text=True, capture_output=True)

    def outputs(self):
        return dict(line.split("=", 1) for line in self.output.read_text().splitlines())

    def commands(self):
        return [json.loads(line) for line in self.trace.read_text().splitlines()]

    def test_prepares_then_builds_in_the_same_resolved_container_and_cleans_it(self):
        result = self.run_step("Prepare and identify native Debian builder")
        self.assertEqual(result.returncode, 0, result.stderr)
        values = self.outputs()
        self.assertEqual(values["native-key"], "a" * 64)
        self.assertEqual(values["image"], IMAGE_ID)
        self.assertTrue((Path(values["state"]) / "native-environment.json").is_file())
        built = self.run_step("Build and validate native Debian package", BUILDER_ID=values["container"])
        self.assertEqual(built.returncode, 0, built.stderr)
        cleaned = self.run_step("Remove the owned builder container", BUILDER_STATE=values["state"])
        self.assertEqual(cleaned.returncode, 0, cleaned.stderr)
        calls = self.commands()
        start = next(call for call in calls if call[0] == "run")
        self.assertIn(IMAGE_ID, start)
        self.assertNotIn(self.env["IMAGE"], start)
        executions = [call for call in calls if call[0] == "exec"]
        self.assertEqual(len(executions), 3)
        self.assertTrue(all("owned-builder" in call for call in executions))
        self.assertEqual(calls[-1], ["rm", "--force", "owned-builder"])

    def test_failed_or_incomplete_identity_never_exposes_a_cache_key(self):
        for failure in ("FAIL_IDENTITY", "BAD_IDENTITY", "FAIL_COPY"):
            with self.subTest(failure=failure):
                self.output.write_text("")
                result = self.run_step("Prepare and identify native Debian builder", **{failure: "1"})
                self.assertNotEqual(result.returncode, 0)
                values = self.outputs()
                self.assertNotIn("native-key", values)
                cleaned = self.run_step("Remove the owned builder container", BUILDER_STATE=values["state"])
                self.assertEqual(cleaned.returncode, 0, cleaned.stderr)

    def test_cleanup_skips_a_builder_that_never_received_a_container_id(self):
        state = self.root / "failed-start"
        state.mkdir()
        (state / "container.cid").touch()
        result = self.run_step("Remove the owned builder container", BUILDER_STATE=str(state))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(self.trace.exists())

    def test_cache_is_partitioned_and_restored_only_after_preparation(self):
        document = WORKFLOW.read_text()
        self.assertLess(document.index("name: Prepare and identify"), document.index("name: Cache Cargo"))
        self.assertLess(document.index("name: Cache Cargo"), document.index("name: Build and validate"))
        cache = workflow_step("Cache Cargo registry and dependency artifacts")
        self.assertIn("prefix-key: debian-native-v1-${{ steps.builder.outputs.native-key }}", cache)
        self.assertIn("if: ${{ env.USE_CACHE == 'true' }}", cache)
        self.assertNotIn("always()", cache)
        self.assertIn("github.event_name == 'pull_request' || (github.event_name == 'push' && github.ref == 'refs/heads/main')", document)
        build = workflow_shell("Build and validate native Debian package")
        self.assertNotIn("apt-get", build)
        self.assertNotIn("rustup-init", build)
        self.assertIn("cargo build --locked --release -p diffz", build)
        smoke = workflow_shell("Install and smoke-test in a clean runtime container")
        self.assertIn("docker run --rm", smoke)
        self.assertNotIn("docker exec", smoke)
        self.assertIn('"$RUNTIME_IMAGE"', smoke)


if __name__ == "__main__":
    unittest.main()
