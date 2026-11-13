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

    def test_packages_are_uploaded_only_for_releases_and_manual_runs(self):
        upload = workflow_step("Upload release packages")
        self.assertIn("if: ${{ github.ref_type == 'tag' || github.event_name == 'workflow_dispatch' }}", upload)
        self.assertIn("retention-days: 1", upload)


class ReceiptComparisonTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.before = self.root / "before.json"
        self.after = self.root / "after.json"
        self.value = dict(schema_version=2, runner_image={"ImageOS": "ubuntu24", "ImageVersion": "old"},
                          architecture="amd64", cc_target="x86_64-linux-gnu",
                          packages=[dict(name="libc6", architecture="amd64", version="1")],
                          tools={name: {"sha256": "a" * 64, "version": name + " 1"} for name in identity.TOOLS})
        self.save(self.before, self.value)

    def save(self, path, value):
        path.write_text(json.dumps(dict(identity=value, sha256=identity.identity_digest(value))))

    def test_identical_receipts_need_no_tools(self):
        self.save(self.after, self.value)
        result = subprocess.run([sys.executable, str(SCRIPT), "--compare", str(self.before), str(self.after)],
                                capture_output=True, text=True, env={**os.environ, "PATH": ""})
        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(result.stdout)
        self.assertTrue(report["same_identity"])
        self.assertEqual([report[k] for k in ("packages", "tools", "fields")], [[], [], []])

    def test_native_package_and_image_changes_explain_new_identity(self):
        self.value["packages"][0]["version"] = "2"
        self.value["packages"].append(dict(name="clang", architecture="amd64", version="3"))
        self.value["runner_image"]["ImageVersion"] = "new"
        self.save(self.after, self.value)
        report = identity.compare_receipts(self.before, self.after)
        self.assertFalse(report["same_identity"])
        self.assertEqual(report["tools"], [])
        self.assertEqual(report["packages"], [dict(name="clang", architecture="amd64", before=None, after="3"),
                                             dict(name="libc6", architecture="amd64", before="1", after="2")])
        self.assertEqual(report["fields"][0]["name"], "runner_image")

    def test_removed_package_and_changed_compiler_are_visible(self):
        self.value["packages"] = [dict(name="clang", architecture="amd64", version="1")]
        self.value["tools"]["cc"]["sha256"] = "b" * 64
        self.save(self.after, self.value)
        report = identity.compare_receipts(self.before, self.after)
        self.assertIsNone(next(p for p in report["packages"] if p["name"] == "libc6")["after"])
        self.assertEqual(report["tools"][0]["name"], "cc")

    def test_tampered_receipt_is_rejected(self):
        self.save(self.after, self.value)
        receipt = json.loads(self.after.read_text())
        receipt["identity"]["cc_target"] = "different"
        self.after.write_text(json.dumps(receipt))
        with self.assertRaisesRegex(ValueError, "digest does not match"):
            identity.compare_receipts(self.before, self.after)

    def test_duplicate_packages_are_not_silently_overwritten(self):
        self.value["packages"] *= 2
        self.save(self.after, self.value)
        with self.assertRaisesRegex(ValueError, "duplicate receipt package"):
            identity.compare_receipts(self.before, self.after)

    def test_v1_container_receipts_compare_and_report_image_change(self):
        self.value["schema_version"] = 1
        del self.value["runner_image"]
        self.value["base_image_id"] = "sha256:" + "1" * 64
        self.save(self.before, self.value)
        self.value["base_image_id"] = "sha256:" + "2" * 64
        self.save(self.after, self.value)
        report = identity.compare_receipts(self.before, self.after)
        self.assertFalse(report["same_identity"])
        self.assertEqual(report["fields"], [dict(name="base_image_id", before="sha256:" + "1" * 64,
                                               after="sha256:" + "2" * 64)])

    def test_structurally_incomplete_receipts_fail_even_with_matching_digest(self):
        mutations = [
            ("schema bool", lambda v: v.update(schema_version=True)),
            ("missing target", lambda v: v.pop("cc_target")),
            ("null target", lambda v: v.update(cc_target=None)),
            ("missing architecture", lambda v: v.pop("architecture")),
            ("null image", lambda v: v.update(runner_image=None)),
            ("missing image version", lambda v: v["runner_image"].pop("ImageVersion")),
            ("null tools", lambda v: v.update(tools=None)),
            ("missing tool", lambda v: v["tools"].pop("cc")),
            ("null tool", lambda v: v["tools"].update(cc=None)),
            ("missing version", lambda v: v["tools"]["cc"].pop("version")),
            ("invalid hash", lambda v: v["tools"]["cc"].update(sha256="invalid")),
            ("empty packages", lambda v: v.update(packages=[])),
            ("extra null field", lambda v: v.update(extra=None)),
            ("extra package field", lambda v: v["packages"][0].update(extra=None)),
        ]
        for name, mutate in mutations:
            with self.subTest(name=name):
                value = json.loads(json.dumps(self.value))
                mutate(value)
                self.save(self.after, value)
                with self.assertRaises(ValueError):
                    identity.compare_receipts(self.before, self.after)

    def test_package_reordering_explains_digest_difference(self):
        self.value["packages"].append(dict(name="clang", architecture="amd64", version="1"))
        self.save(self.before, self.value)
        self.value["packages"].reverse()
        self.save(self.after, self.value)
        report = identity.compare_receipts(self.before, self.after)
        self.assertFalse(report["same_identity"])
        self.assertEqual(report["packages"], [])
        self.assertTrue(report["package_order_changed"])

    def test_invalid_json_returns_failure_without_traceback(self):
        self.after.write_text("{")
        result = subprocess.run([sys.executable, str(SCRIPT), "--compare", str(self.before), str(self.after)],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, "")
        self.assertNotIn("Traceback", result.stderr)


if __name__ == "__main__":
    unittest.main()
