"""Regression tests for the benchmark driver, not Rust performance samples.

The integration fixture uses real disposable Git worktrees and deliberately fake
compiler executables. Their timings must never be used as build measurements.
"""
from contextlib import redirect_stderr, redirect_stdout
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

REPO = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("benchmark_cold", REPO / "scripts/benchmark-cold-builds.py")
BENCH = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BENCH)

FAKE_TOOL = r'''#!/usr/bin/env python3
import json
import os
from pathlib import Path
import shutil
import sys

tool = Path(sys.argv[0]).name
args = sys.argv[1:]
if tool == "rustup":
    if os.environ.get("FIXTURE_MISSING_TOOLCHAIN"):
        print("fixture toolchain is not installed")
        sys.exit(1)
    if args[0] == "which":
        print(Path(__file__).with_name(args[-1]))
        sys.exit(0)
    assert args[0] == "run", args
    os.environ["FIXTURE_SELECTED_TOOLCHAIN"] = args[1]
    tool, args = args[2], args[3:]

selected = os.environ.get("FIXTURE_SELECTED_TOOLCHAIN", "")
if any(arg in ("-V", "-Vv", "--version") for arg in args):
    suffix = "-nightly" if selected.startswith("nightly-") else ""
    if tool == "cargo" and os.environ.get("FIXTURE_STABLE_CARGO"):
        suffix = ""
    print(tool + " 1.100.0" + suffix + " (fixture only)")
    sys.exit(0)

operation = next(arg for arg in args if arg in ("fetch", "clean", "build", "test", "shutdown"))
cwd = Path.cwd()
target = Path(os.environ["CARGO_TARGET_DIR"])
sources = {str(path): (cwd / path).read_text() for path in (
    "crates/core/src/patch.rs", "crates/ui/src/magnify.rs", "crates/app/src/main.rs")}
row = dict(tool=tool, operation=operation, args=args, selected_toolchain=selected,
           cwd=str(cwd), target=str(target), build_dir=os.environ["CARGO_BUILD_BUILD_DIR"],
           rustc=os.environ.get("RUSTC"), wrapper=os.environ.get("RUSTC_WRAPPER"),
           fingerprint=os.environ.get("CARGO_BUILD_FINGERPRINT"), sources=sources,
           source_mtimes={path: (cwd / path).stat().st_mtime_ns for path in sources})
marker = target / "fixture-seed"
if marker.exists():
    row["seed_mtime"] = marker.stat().st_mtime_ns
    row["seed_inode"] = marker.stat().st_ino
with Path(os.environ["FIXTURE_TRACE"]).open("a") as trace:
    trace.write(json.dumps(row) + "\n")
if tool == "bazel":
    if operation == "shutdown" and os.environ.get("FIXTURE_SHUTDOWN_FAILURE"):
        sys.exit(17)
    if operation == "build" and "--nobuild" not in args and os.environ.get("FIXTURE_BAZEL_FAILURE"):
        sys.exit(23)
    sys.exit(0)
if operation == "clean":
    assert args[args.index("--target-dir") + 1] == str(target), args
    shutil.rmtree(target, ignore_errors=True)
elif operation == "build":
    assert args[args.index("--target-dir") + 1] == str(target), args
    core = cwd / "crates/core/src/patch.rs"
    if "41_720" in core.read_text():
        if os.environ.get("FIXTURE_CONCURRENT_EDIT"):
            core.write_text("concurrent human edit\n")
        if os.environ.get("FIXTURE_FAIL_EDIT"):
            sys.exit(23)
    timing = target / "cargo-timings/cargo-timing.html"
    timing.parent.mkdir(parents=True, exist_ok=True)
    timing.write_text("<title>Compiler fixture; not a performance sample</title>")
    if not marker.exists():
        marker.write_text("seeded fixture artifact")
    (target / "fixture-last-sources").write_text(json.dumps(sources))
'''


@unittest.skipUnless(shutil.which("git"), "Git is required for worktree safety regressions")
class HarnessTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="diffz-benchmark-test-")
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.root = self.base / "repo"
        self.root.mkdir()
        self.bin = self.base / "bin"
        self.bin.mkdir()
        self.scratch = self.base / "scratch"
        self.scratch.mkdir()
        self.trace = self.base / "fixture-trace.jsonl"
        self.evidence = self.base / "evidence"
        self.write(".gitignore", "target/\n")
        self.write("Cargo.toml", '[workspace]\nmembers=[]\n')
        self.write("Cargo.lock", "# fixture only\n")
        self.write("rust-toolchain.toml", '[toolchain]\nchannel="1.99.0"\n')
        self.write(".cargo/config.toml", '[profile.fast-dev]\ninherits="dev"\n')
        self.write("crates/core/src/patch.rs", "max_files: 10_000,\n")
        self.write("crates/ui/src/magnify.rs", "MAX_SCALE: f32 = 5.0;\nassert_eq!(window.magnification().scale, 4.0);\n")
        self.write("crates/app/src/main.rs", 'format!("diffz {}", version);\nformat!("diffz {}", version);\n')
        self.write("scripts/bazel-trial.sh", '#!/usr/bin/env bash\nexec "$BAZEL" "$@"\n')
        for tool in ("cargo", "rustc", "rustup", "cc", "cxx", "bazel"):
            path = self.bin / tool
            path.write_text(FAKE_TOOL.replace("#!/usr/bin/env python3", "#!" + sys.executable, 1))
            path.chmod(0o755)
        self.env = {
            **os.environ, "PATH": str(self.bin) + os.pathsep + os.environ.get("PATH", ""),
            "TMPDIR": str(self.scratch), "CARGO_HOME": str(self.base / "cargo-home"),
            "BAZEL": str(self.bin / "bazel"), "FIXTURE_TRACE": str(self.trace),
            "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
            "CARGO_TARGET_DIR": str(self.root / "target/ordinary"),
            "CARGO_BUILD_BUILD_DIR": str(self.root / "target/ordinary-build"),
            "RUSTC_WRAPPER": "should-be-disabled",
        }
        for key in ("RUSTC", "RUSTC_BOOTSTRAP", "CARGO_BUILD_FINGERPRINT", "RUSTUP_TOOLCHAIN"):
            self.env.pop(key, None)
        self.git("init", "-q")
        self.git("config", "user.name", "Benchmark fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        self.git("add", ".")
        self.git("-c", "commit.gpgsign=false", "commit", "-qm", "fixture")
        self.write("target/ordinary/keep", "ordinary target must survive")
        self.write("target/ordinary-build/keep", "ordinary build dir must survive")
        self.original = {relative: ((self.root / relative).read_bytes(), (self.root / relative).stat().st_mtime_ns)
                         for _, relative, _, _ in BENCH.PROBES}
        self.addCleanup(mock.patch.stopall)
        mock.patch.object(BENCH, "ROOT", self.root).start()
        mock.patch.dict(os.environ, self.env, clear=True).start()

    def write(self, relative, content):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)

    def git(self, *args):
        result = subprocess.run(["git", *args], cwd=self.root, env=self.env, check=True,
                                text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        return result.stdout.strip()

    def invoke(self, *args):
        argv = ["--tool", "cargo", "--iterations", "1", "--jobs", "2",
                "--cc", str(self.bin / "cc"), "--cxx", str(self.bin / "cxx")]
        if "--preflight" not in args:
            argv += ["--output", str(self.evidence)]
        with redirect_stdout(io.StringIO()):
            BENCH.main(argv + list(args))

    def read_rows(self):
        return [json.loads(line) for line in (self.evidence / "measurements.jsonl").read_text().splitlines()]

    def compiler_calls(self):
        return [json.loads(line) for line in self.trace.read_text().splitlines()] if self.trace.exists() else []

    def assert_original_checkout(self):
        for relative, expected in self.original.items():
            path = self.root / relative
            self.assertEqual((path.read_bytes(), path.stat().st_mtime_ns), expected)
        self.assertEqual((self.root / "target/ordinary/keep").read_text(), "ordinary target must survive")
        self.assertEqual((self.root / "target/ordinary-build/keep").read_text(), "ordinary build dir must survive")

    def test_legacy_cold_workload_and_tool_order_are_preserved(self):
        self.invoke("--tool", "both", "--iterations", "2")
        rows = self.read_rows()
        cold = [row["name"] for row in rows if row["phase"] == "build"]
        self.assertEqual(cold, ["cargo-cold-1", "bazel-cold-1", "bazel-cold-2", "cargo-cold-2"])
        builds = [call for call in self.compiler_calls() if call["tool"] == "cargo" and call["operation"] == "build"]
        self.assertTrue(all("--workspace" in call["args"] and "--all-features" in call["args"]
                            and "--all-targets" in call["args"] for call in builds))
        self.assertTrue((self.evidence / "cargo-timing-1.html").is_file())
        self.assertTrue(json.loads((self.evidence / "completion.json").read_text())["completed"])
        self.assert_original_checkout()

    def test_paired_nightly_desktop_and_reuse_have_identical_inputs(self):
        self.invoke("--cargo-workload", "desktop", "--cargo-profile", "fast-dev",
                    "--cargo-toolchain", "nightly-2026-10-19", "--cargo-fingerprint", "both",
                    "--iterations", "2", "--cargo-scenario", "noop", "--cargo-scenario", "edit",
                    "--cargo-scenario", "fresh-worktree")
        rows = self.read_rows()
        cold = [row["name"] for row in rows if "-cold-" in row["name"]]
        self.assertEqual(cold, ["cargo-mtime-cold-1", "cargo-content-cold-1",
                                "cargo-content-cold-2", "cargo-mtime-cold-2"])
        self.assertTrue(all(row["schema_version"] == BENCH.SCHEMA_VERSION for row in rows))
        builds = [call for call in self.compiler_calls() if call["operation"] == "build"]
        for call in builds:
            self.assertEqual(call["selected_toolchain"], "nightly-2026-10-19")
            self.assertEqual(call["rustc"], str(self.bin / "rustc"))
            self.assertEqual(call["wrapper"], "")
            self.assertIn('build.fingerprint="' + call["fingerprint"] + '"', call["args"])
            self.assertIn("-Zchecksum-freshness", call["args"])
            self.assertIn("--profile=fast-dev", call["args"])
            self.assertEqual(call["args"][call["args"].index("-p") + 1], "diffz")
            self.assertEqual(call["args"][call["args"].index("--bin") + 1], "diffz")
            self.assertNotIn("--all-features", call["args"])
            self.assertNotIn("--workspace", call["args"])
            self.assertEqual(call["target"], call["build_dir"])
            self.assertNotEqual(call["cwd"], str(self.root))
        fresh = [call for call in builds if "-fresh-" in call["cwd"]]
        self.assertEqual(len(fresh), 4)
        for call in fresh:
            self.assertIn("seed_mtime", call, "fresh builds must receive actual copied artifacts")
            self.assertEqual(call["sources"], {path: value[0].decode() for path, value in self.original.items()})
        seed_modes = {mode: {call["target"] for call in builds if call["fingerprint"] == mode and "-fresh-" not in call["cwd"]}
                      for mode in ("mtime", "content")}
        self.assertTrue(seed_modes["mtime"].isdisjoint(seed_modes["content"]))
        edits = [row for row in rows if "-edit-" in row["name"]]
        self.assertEqual(len(edits), 12)
        self.assertEqual(len([row for row in rows if row["phase"] == "restore"]), 4)
        self.assertEqual(len([row for row in rows if row["phase"] == "verification"]), 2)
        self.assertEqual(len(self.git("worktree", "list", "--porcelain").split("worktree ")) - 1, 1)
        self.assertEqual(list(self.scratch.iterdir()), [])
        self.assert_original_checkout()

    def test_failed_edit_restores_source_and_propagates_compile_failure(self):
        with mock.patch.dict(os.environ, {"FIXTURE_FAIL_EDIT": "1"}):
            with self.assertRaisesRegex(RuntimeError, "cargo-edit-core-1 failed"):
                self.invoke("--cargo-scenario", "edit")
        self.assertEqual(next(row for row in self.read_rows() if row["name"] == "cargo-edit-core-1")["exit_code"], 23)
        completion = json.loads((self.evidence / "completion.json").read_text())
        self.assertFalse(completion["completed"])
        self.assertEqual(completion["retained_worktrees"], [])
        self.assertEqual(list(self.scratch.iterdir()), [])
        self.assert_original_checkout()

    def test_concurrent_edit_is_preserved_in_retained_worktree(self):
        with mock.patch.dict(os.environ, {"FIXTURE_FAIL_EDIT": "1", "FIXTURE_CONCURRENT_EDIT": "1"}):
            with self.assertRaisesRegex(RuntimeError, "Source changed concurrently"):
                self.invoke("--cargo-scenario", "edit")
        completion = json.loads((self.evidence / "completion.json").read_text())
        self.assertFalse(completion["completed"])
        self.assertTrue(completion["cleanup_errors"])
        retained = [Path(path) for path in completion["retained_worktrees"]]
        self.assertEqual(len(retained), 1)
        self.assertEqual((retained[0] / "crates/core/src/patch.rs").read_text(), "concurrent human edit\n")
        self.assert_original_checkout()

    def test_bazel_shutdown_failure_invalidates_success(self):
        with mock.patch.dict(os.environ, {"FIXTURE_SHUTDOWN_FAILURE": "1"}):
            with self.assertRaisesRegex(RuntimeError, "Cleanup failed"):
                self.invoke("--tool", "bazel")
        completion = json.loads((self.evidence / "completion.json").read_text())
        self.assertFalse(completion["completed"])
        self.assertIn("bazel-shutdown exited 17", completion["cleanup_errors"])

    def test_partial_worktree_creation_keeps_hook_changes(self):
        hook = self.root / ".git/hooks/post-checkout"
        hook.write_text("#!/bin/sh\nprintf 'hook output\\n' > hook-created.txt\nexit 9\n")
        hook.chmod(0o755)
        with self.assertRaisesRegex(RuntimeError, "seed-checkout failed"):
            self.invoke("--cargo-scenario", "noop")
        completion = json.loads((self.evidence / "completion.json").read_text())
        self.assertFalse(completion["completed"])
        self.assertEqual(len(completion["retained_worktrees"]), 1)
        retained = Path(completion["retained_worktrees"][0])
        self.assertEqual((retained / "hook-created.txt").read_text(), "hook output\n")
        self.assertEqual(self.compiler_calls(), [])
        self.assert_original_checkout()

    def test_cleanup_failure_does_not_mask_bazel_build_failure(self):
        with mock.patch.dict(os.environ, {"FIXTURE_SHUTDOWN_FAILURE": "1", "FIXTURE_BAZEL_FAILURE": "1"}):
            with self.assertRaisesRegex(RuntimeError, "bazel-cold-1 failed"):
                self.invoke("--tool", "bazel")
        self.assertFalse(json.loads((self.evidence / "completion.json").read_text())["completed"])

    def test_preflight_creates_no_evidence_worktrees_or_builds(self):
        self.invoke("--preflight")
        self.assertFalse(self.evidence.exists())
        self.assertEqual(self.compiler_calls(), [])
        self.assertEqual(list(self.scratch.iterdir()), [])

    def test_missing_default_toolchain_does_not_invoke_rustup_proxy(self):
        with mock.patch.dict(os.environ, {"FIXTURE_MISSING_TOOLCHAIN": "1"}):
            with self.assertRaisesRegex(SystemExit, "not installed"):
                self.invoke("--preflight")
        self.assertFalse(self.evidence.exists())
        self.assertEqual(self.compiler_calls(), [])

    def test_mismatched_nightly_cargo_fails_before_build(self):
        with mock.patch.dict(os.environ, {"FIXTURE_STABLE_CARGO": "1"}):
            with self.assertRaisesRegex(SystemExit, "Both Cargo and rustc must report nightly"):
                self.invoke("--cargo-toolchain", "nightly-2026-10-19", "--cargo-fingerprint", "both")
        self.assertFalse(self.evidence.exists())
        self.assertEqual(self.compiler_calls(), [])

    def test_missing_tools_are_reported_before_evidence(self):
        real_which = shutil.which
        with mock.patch.object(BENCH.shutil, "which", side_effect=lambda tool: None if tool == "cargo" else real_which(tool)):
            with self.assertRaisesRegex(SystemExit, "Missing required executable.*cargo"):
                self.invoke()
        self.assertFalse(self.evidence.exists())

    def test_dirty_checkout_and_different_ancestor_config_fail_before_build(self):
        self.write("crates/core/src/patch.rs", "uncommitted change\n")
        with self.assertRaisesRegex(SystemExit, "clean, committed checkout"):
            self.invoke("--cargo-scenario", "fresh-worktree")
        self.git("checkout", "--", "crates/core/src/patch.rs")
        config = self.scratch / ".cargo/config.toml"
        config.parent.mkdir()
        config.write_text('[build]\nrustflags=["--cfg", "different"]\n')
        with self.assertRaisesRegex(SystemExit, "inherit different Cargo configuration"):
            self.invoke("--cargo-scenario", "fresh-worktree")
        self.assertFalse(self.evidence.exists())

    def test_fingerprint_arguments_require_one_recent_dated_nightly(self):
        for toolchain in ("nightly", "stable", "1.99.0", "nightly-2026-09-01", "nightly-2026-09-31"):
            with self.subTest(toolchain=toolchain), redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                BENCH.parse_args(["--preflight", "--tool", "cargo", "--cargo-fingerprint", "both",
                                  "--cargo-toolchain", toolchain])
        for extra in (["--cargo-arg=--config=build.fingerprint=\"mtime\""], ["--tool", "both"]):
            with self.subTest(extra=extra), redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                BENCH.parse_args(["--preflight", "--tool", "cargo", "--cargo-fingerprint", "both",
                                  "--cargo-toolchain", "nightly-2026-10-19", *extra])


if __name__ == "__main__":
    unittest.main()
