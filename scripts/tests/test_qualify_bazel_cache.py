"""Qualification policy tests with synthetic BEP and a subprocess Bazel fixture.

These do not claim a Rust build or a real cross-host cache result.
"""

import argparse
import copy
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().parents[1] / "qualify-bazel-cache.py"
SPEC = importlib.util.spec_from_file_location("qualify_bazel_cache", SCRIPT)
qualify = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(qualify)


def identity(host="producer"):
    native = {"declared_environment_id": "image@sha256:abc", "tools": {"gcc": "gcc-bytes"},
              "configured_environment": {"cc": "/usr/bin/gcc", "cxx": "/usr/bin/g++"},
              "selected_linker": {"search_path": "bazel-out/k8-fastbuild/bin/linker/gcc-ld"}}
    return {"schema": 1, "source": {"commit": "a" * 40, "clean": True},
            "native": native, "native_sha256": qualify.digest(native),
            "host": {"label": host, "machine_id_sha256": host, "kernel": "fixture"},
            "bazel": {"version": "8.4.2", "sha256": "bazel-bytes"},
            "graph": {"MODULE.bazel": "graph-bytes"}}


def completed(mode):
    return {"schema": 1, "status": "complete", "mode": mode, "identity": identity(mode),
            "cache_endpoint": "grpc://cache.example:9092",
            "instance_name": "diffz-qualification/" + identity()["native_sha256"],
            "service_instance_isolation_acknowledged": True,
            "build": {"remote_cache_hits": 12, "other_execution": {}, "runners": {"remote cache hit": 12}},
            "tests": {"labels": ["//:core_tests"], "executed_results": 1},
            "materialized_linker": {"ld_lld_sha256": "linker-bytes"},
            "runtime": {"binary_sha256": "binary-bytes", "version": "diffz 0.3.1", "inspect": {"fixture": "F01"}}}


FAKE_BAZEL = r'''#!/usr/bin/env python3
import json
import os
from pathlib import Path
import sys

args = sys.argv[1:]
if "--version" in args:
    print("bazel 8.4.2")
    raise SystemExit(0)
if "shutdown" in args:
    raise SystemExit("batch qualification must not start a shutdown client")
if "--batch" not in args:
    raise SystemExit("qualification must opt into batch mode")
for required in ("--spawn_strategy=linux-sandbox", "--strategy_regexp=.*=linux-sandbox",
                 "--strategy=TestRunner=linux-sandbox", "--noincompatible_legacy_local_fallback"):
    if required not in args:
        raise SystemExit("missing required sandbox strategy flag: " + required)
def option(name):
    return next(x.split("=", 1)[1] for x in args if x.startswith(name + "="))
output = Path(option("--output_user_root"))
bep = Path(option("--build_event_json_file"))
if os.environ.get("QUALIFICATION_FIXTURE_NO_SANDBOX"):
    bep.write_text(json.dumps({"finished": {"exitCode": {"code": 2},
                  "failureDetail": {"executionOptions": {"code": "STRATEGY_NOT_FOUND"}}}}) + "\n")
    print("linux-sandbox was requested but no strategy with that identifier was registered")
    raise SystemExit(2)
if os.environ.get("QUALIFICATION_FIXTURE_HANG"):
    import time
    time.sleep(60)
if "build" in args:
    linker_dir = output / "fixture-hash/execroot/diffz/bazel-out/k8-fastbuild/bin/linker/gcc-ld"
    linker_dir.mkdir(parents=True)
    for linker in (linker_dir / "ld.lld", linker_dir.parent / "rust-lld"):
        linker.write_text("#!/usr/bin/env python3\nprint('LLD fixture')\n")
        linker.chmod(0o755)
    binary = output / "artifacts" / "diffz"
    binary.parent.mkdir(parents=True, exist_ok=True)
    binary.write_text("#!/usr/bin/env python3\nimport sys\nprint('diffz 0.3.1' if '--version' in sys.argv else '{\"fixture\": \"F01\"}')\n")
    binary.chmod(0o755)
    producer = "--remote_accept_cached=false" in args
    local = producer or bool(os.environ.get("QUALIFICATION_FIXTURE_LOCAL_BUILD"))
    runner = os.environ.get("QUALIFICATION_FIXTURE_RUNNER", "linux-sandbox" if local else "remote cache hit")
    runners = [{"name": "internal", "count": 2}, {"name": runner, "count": 12}]
    events = [{"buildMetrics": {"actionSummary": {"runnerCount": runners}}},
              {"namedSetOfFiles": {"files": [{"name": "diffz", "uri": binary.as_uri()}]}}]
else:
    if "--test_strategy=standalone" not in args:
        raise SystemExit("test context must be standalone; local is not registered in Bazel 8.4.2")
    runner = os.environ.get("QUALIFICATION_FIXTURE_TEST_RUNNER", "linux-sandbox")
    events = [{"buildMetrics": {"actionSummary": {"runnerCount": [{"name": runner, "count": 1}]}}},
              {"id": {"testResult": {"label": "//:core_tests"}}, "testResult": {"status": "PASSED"}},
              {"id": {"testSummary": {"label": "//:core_tests"}},
               "testSummary": {"overallStatus": "PASSED", "totalNumCached": 0}}]
events.append({"finished": {"overallSuccess": True, "exitCode": {"code": 0}}})
bep.write_text("".join(json.dumps(event) + "\n" for event in events))
'''


class EvidencePolicyTests(unittest.TestCase):
    def test_accepts_matching_observations_on_different_hosts(self):
        result = qualify.compare_runs(completed("producer"), completed("consumer"))
        self.assertEqual(result["status"], "observed_two_host_cache_reuse")
        self.assertTrue(result["limitations"])

    def test_native_changes_are_rejected(self):
        producer, consumer = completed("producer"), completed("consumer")
        consumer["identity"]["native"]["tools"]["gcc"] = "different-compiler"
        consumer["identity"]["native_sha256"] = qualify.digest(consumer["identity"]["native"])
        with self.assertRaisesRegex(qualify.QualificationError, "native_sha256"):
            qualify.compare_runs(producer, consumer)

    def test_same_observed_host_cannot_qualify(self):
        producer, consumer = completed("producer"), completed("consumer")
        consumer["identity"]["host"]["machine_id_sha256"] = "producer"
        with self.assertRaisesRegex(qualify.QualificationError, "same observed machine"):
            qualify.compare_runs(producer, consumer)

    def test_commit_and_bazel_and_graph_mismatches_fail(self):
        for key in ("source", "bazel", "graph"):
            with self.subTest(key=key):
                producer, consumer = completed("producer"), completed("consumer")
                consumer["identity"][key] = {"different": True}
                with self.assertRaisesRegex(qualify.QualificationError, key):
                    qualify.compare_runs(producer, consumer)

    def test_partial_remote_hit_does_not_claim_full_restore(self):
        producer, consumer = completed("producer"), completed("consumer")
        consumer["build"]["other_execution"] = {"linux-sandbox": 1}
        with self.assertRaisesRegex(qualify.QualificationError, "full build"):
            qualify.compare_runs(producer, consumer)

    def test_no_hits_do_not_claim_reuse(self):
        producer, consumer = completed("producer"), completed("consumer")
        consumer["build"]["remote_cache_hits"] = 0
        with self.assertRaises(qualify.QualificationError):
            qualify.compare_runs(producer, consumer)

    def test_runtime_and_test_differences_fail(self):
        for key in ("runtime", "tests"):
            with self.subTest(key=key):
                producer, consumer = completed("producer"), completed("consumer")
                consumer[key] = {"different": True}
                with self.assertRaisesRegex(qualify.QualificationError, key):
                    qualify.compare_runs(producer, consumer)

    def test_service_isolation_acknowledgement_is_required(self):
        producer, consumer = completed("producer"), completed("consumer")
        producer["service_instance_isolation_acknowledged"] = False
        with self.assertRaisesRegex(qualify.QualificationError, "service_instance"):
            qualify.compare_runs(producer, consumer)

    def test_credential_bearing_or_non_grpc_endpoints_are_rejected(self):
        for endpoint in ("grpc://user:token@cache", "grpcs://cache?token=secret", "http://cache", "grpc://cache/path"):
            with self.subTest(endpoint=endpoint), self.assertRaises(qualify.QualificationError):
                qualify.validate_endpoint(endpoint)

    def test_missing_or_failed_bep_cannot_pass(self):
        for events in ([], [{"finished": {"overallSuccess": False}}],
                       [{"finished": {"overallSuccess": True}}, {"buildMetrics": {"actionSummary": {}}}]):
            with self.subTest(events=events), self.assertRaises(qualify.QualificationError):
                qualify.build_evidence(events)

    def test_disk_or_unknown_execution_remains_visible(self):
        events = [{"finished": {"exitCode": {"code": 0}}},
                  {"buildMetrics": {"actionSummary": {"runnerCount": [
                      {"name": "remote cache hit", "count": 10}, {"name": "disk cache hit", "count": 2},
                      {"name": "internal", "count": 2}, {"name": "new-runner", "count": 1}]}}}]
        result = qualify.build_evidence(events)
        self.assertEqual(result["other_execution"], {"disk cache hit": 2, "new-runner": 1})

    def test_cached_test_results_are_rejected(self):
        base = [{"finished": {"exitCode": {"code": 0}}},
                {"id": {"testSummary": {"label": "//:test"}}, "testSummary": {"overallStatus": "PASSED"}},
                {"testResult": {"status": "PASSED"}}]
        for cached in ({"cachedLocally": True}, {"executionInfo": {"cachedRemotely": True}}):
            events = copy.deepcopy(base)
            events[2]["testResult"].update(cached)
            with self.assertRaisesRegex(qualify.QualificationError, "cached result"):
                qualify.test_evidence(events)

    def test_invalid_resource_limits_fail(self):
        for name, values in (("phase_timeout_seconds", (0, -1, float("inf"), float("nan"))),
                             ("runtime_timeout_seconds", (0, -1, float("inf"), float("nan"))),
                             ("disk_reserve_gib", (0, 1.99, float("inf"), float("nan"), 1e308))):
            for value in values:
                with self.subTest(name=name, value=value), self.assertRaises(qualify.QualificationError):
                    qualify.resource_policy(argparse.Namespace(**{name: value}))


@unittest.skipUnless(sys.platform == "linux", "qualification process ownership uses Linux waitid and /proc")
class RunnerFixtureTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="diffz-qualification-tests-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name) / "checkout with spaces"
        (self.root / "scripts").mkdir(parents=True)
        shutil.copy(SCRIPT.with_name("bazel-trial.sh"), self.root / "scripts/bazel-trial.sh")
        self.binary = self.root / "fixture bazel"
        self.binary.write_text(FAKE_BAZEL)
        self.binary.chmod(0o755)
        test_root = patch.object(qualify, "TEST_ROOT", str(Path(self.tmp.name) / "tests"))
        test_root.start()
        self.addCleanup(test_root.stop)
        # These fixtures write kilobytes; simulate sufficient capacity so a
        # developer can run the controls' tests on a nearly full filesystem.
        capacity = patch.object(qualify, "free_bytes", return_value=8 * qualify.GIB)
        capacity.start()
        self.addCleanup(capacity.stop)

    def args(self, mode, suffix=""):
        return argparse.Namespace(command=mode, output=self.root / "target" / (mode + suffix),
                                  native_environment_id="fixture", native_file=[], host_label=mode,
                                  cache_endpoint="grpc://cache.example:9092", confirm_instance_isolation=True,
                                  repo_contents_cache=None, credential_helper=None,
                                  producer_report=self.root / "target/producer/result.json")

    def execute(self, args):
        with patch.object(qualify, "collect_identity", return_value=(identity(args.command), self.binary)), \
             patch.object(qualify, "source_identity", return_value=identity()["source"]):
            return qualify.execute(args, self.root)

    def test_complete_driver_uses_read_only_fresh_consumer_and_forced_tests(self):
        producer = self.execute(self.args("producer"))
        consumer = self.execute(self.args("consumer"))
        result = qualify.compare_runs(producer, consumer)
        self.assertEqual(result["status"], "observed_two_host_cache_reuse")
        commands = [json.loads(line) for line in (self.root / "target/consumer/commands.jsonl").read_text().splitlines()]
        for row in commands[:2]:
            self.assertEqual(row["status"], "complete")
            self.assertEqual(row["cleanup"]["live_members_after"], [])
            self.assertIn("--disk_cache=", row["argv"])
            self.assertIn("--remote_upload_local_results=false", row["argv"])
            self.assertIn("--remote_executor=", row["argv"])
            self.assertIn("--build_event_json_file_path_conversion=false", row["argv"])
            self.assertIn("--spawn_strategy=linux-sandbox", row["argv"])
            self.assertIn("--strategy_regexp=.*=linux-sandbox", row["argv"])
            self.assertIn("--strategy=TestRunner=linux-sandbox", row["argv"])
            self.assertIn("--noincompatible_legacy_local_fallback", row["argv"])
        self.assertIn("--cache_test_results=no", commands[1]["argv"])
        self.assertIn("--test_strategy=standalone", commands[1]["argv"])
        self.assertIn("--state-dir", commands[-1]["argv"])
        state = Path(commands[-1]["argv"][-1])
        self.assertTrue(state.is_relative_to(self.root / "target/consumer/runtime"))
        self.assertTrue((self.root / "target/consumer/comparison.json").is_file())
        self.assertEqual(consumer["resource_policy"]["execution_mode"], "batch")
        self.assertEqual(consumer["schema"], 1)
        self.assertEqual(consumer["test_execution"]["runners"], {"linux-sandbox": 1})

    def test_existing_output_is_never_reused_or_deleted(self):
        args = self.args("producer")
        args.output.mkdir(parents=True)
        sentinel = args.output / "keep"
        sentinel.write_text("existing data")
        with self.assertRaises(FileExistsError):
            self.execute(args)
        self.assertEqual(sentinel.read_text(), "existing data")

    def test_build_miss_retains_failure_evidence_and_skips_tests(self):
        self.execute(self.args("producer"))
        args = self.args("consumer")
        with patch.dict(os.environ, {"QUALIFICATION_FIXTURE_LOCAL_BUILD": "1"}):
            with self.assertRaisesRegex(qualify.QualificationError, "uncached build"):
                self.execute(args)
        report = json.loads((args.output / "result.json").read_text())
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["resource_policy"]["execution_mode"], "batch")
        self.assertFalse((args.output / "test.log").exists())

    def test_unacknowledged_service_cannot_start_bazel(self):
        args = self.args("producer")
        args.confirm_instance_isolation = False
        with self.assertRaisesRegex(qualify.QualificationError, "isolates instance"):
            self.execute(args)
        self.assertFalse((args.output / "bazel-output").exists())

    def test_credential_helper_is_scoped_and_passed_as_one_argument(self):
        args = self.args("producer")
        args.credential_helper = self.binary
        self.execute(args)
        commands = [json.loads(line) for line in (args.output / "commands.jsonl").read_text().splitlines()]
        self.assertIn("--credential_helper=cache.example=" + str(self.binary), commands[0]["argv"])

    def test_source_change_during_run_invalidates_completion(self):
        args = self.args("producer")
        with patch.object(qualify, "collect_identity", return_value=(identity("producer"), self.binary)), \
             patch.object(qualify, "source_identity", return_value={"commit": "different", "clean": True}):
            with self.assertRaisesRegex(qualify.QualificationError, "Source changed"):
                qualify.execute(args, self.root)
        report = json.loads((args.output / "result.json").read_text())
        self.assertEqual(report["status"], "failed")

    def test_timeout_retains_failure_receipt_and_cannot_publish_comparison(self):
        self.execute(self.args("producer"))
        args = self.args("consumer")
        args.phase_timeout_seconds = 0.3
        with patch.dict(os.environ, {"QUALIFICATION_FIXTURE_HANG": "1"}):
            with self.assertRaisesRegex(qualify.PhaseError, "phase_timeout"):
                self.execute(args)
        report = json.loads((args.output / "result.json").read_text())
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["failed_phase"]["stop_reason"], "phase_timeout")
        self.assertEqual(report["failed_phase"]["cleanup"]["live_members_after"], [])
        self.assertFalse((args.output / "comparison.json").exists())
        self.assertFalse((args.output / "test.log").exists())

    def test_required_sandbox_unavailable_stops_without_weaker_retry(self):
        args = self.args("producer")
        with patch.dict(os.environ, {"QUALIFICATION_FIXTURE_NO_SANDBOX": "1"}):
            with self.assertRaises(qualify.PhaseError):
                self.execute(args)
        report = json.loads((args.output / "result.json").read_text())
        commands = (args.output / "commands.jsonl").read_text().splitlines()
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["failed_phase"]["exit_code"], 2)
        self.assertEqual(len(commands), 1)
        self.assertIn("--spawn_strategy=linux-sandbox", report["resource_policy"]["bazel_strategy_flags"])
        self.assertIn("STRATEGY_NOT_FOUND", (args.output / "build.bep.jsonl").read_text())
        self.assertFalse((args.output / "test.log").exists())
        self.assertFalse((args.output / "comparison.json").exists())

    def test_observed_weaker_build_runner_cannot_qualify(self):
        args = self.args("producer")
        with patch.dict(os.environ, {"QUALIFICATION_FIXTURE_RUNNER": "processwrapper-sandbox"}):
            with self.assertRaisesRegex(qualify.QualificationError, "required linux-sandbox"):
                self.execute(args)
        report = json.loads((args.output / "result.json").read_text())
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["build"]["runners"]["processwrapper-sandbox"], 12)
        self.assertFalse((args.output / "test.log").exists())

    def test_observed_weaker_test_runner_cannot_publish_comparison(self):
        self.execute(self.args("producer"))
        args = self.args("consumer")
        with patch.dict(os.environ, {"QUALIFICATION_FIXTURE_TEST_RUNNER": "local"}):
            with self.assertRaisesRegex(qualify.QualificationError, "required linux-sandbox"):
                self.execute(args)
        report = json.loads((args.output / "result.json").read_text())
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["test_execution"]["runners"], {"local": 1})
        self.assertFalse((args.output / "comparison.json").exists())

    def test_preflight_reports_insufficient_capacity_without_a_build(self):
        args = self.args("preflight")
        with patch.object(qualify, "free_bytes", return_value=qualify.GIB):
            report = self.execute(args)
        self.assertEqual(report["status"], "complete")
        self.assertFalse(report["resource_preflight"]["startup_ready"])
        self.assertFalse((args.output / "commands.jsonl").exists())
        self.assertEqual(json.loads((args.output / "preflight.json").read_text()), identity("preflight"))

    def test_wrapper_batch_opt_in_preserves_normal_startup(self):
        recorded = self.root / "wrapper-argv.json"
        self.binary.write_text("#!/usr/bin/env python3\nimport json, os, sys\n"
                               "open(os.environ['FIXTURE_ARGV'], 'w').write(json.dumps(sys.argv[1:]))\n")
        env = {**os.environ, "BAZEL": str(self.binary), "FIXTURE_ARGV": str(recorded)}
        argv = ["bash", str(self.root / "scripts/bazel-trial.sh"), "info"]
        for value in (None, "0", "1"):
            with self.subTest(value=value):
                env.pop("DIFFZ_BAZEL_BATCH", None)
                if value is not None:
                    env["DIFFZ_BAZEL_BATCH"] = value
                result = subprocess.run(argv, env=env, capture_output=True, text=True, timeout=10)
                self.assertEqual(result.returncode, 0, result.stderr)
                passed = json.loads(recorded.read_text())
                self.assertEqual("--batch" in passed, value == "1")
                if value == "1":
                    self.assertLess(passed.index("--batch"), passed.index("info"))
        recorded.unlink()
        env["DIFFZ_BAZEL_BATCH"] = "invalid"
        result = subprocess.run(argv, env=env, capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 2)
        self.assertIn("DIFFZ_BAZEL_BATCH", result.stderr)
        self.assertFalse(recorded.exists())

    def test_native_mismatch_rejected_before_consumer_cache_access(self):
        self.execute(self.args("producer"))
        args = self.args("consumer")
        changed = identity("consumer")
        changed["native"]["tools"]["gcc"] = "other"
        changed["native_sha256"] = qualify.digest(changed["native"])
        with patch.object(qualify, "collect_identity", return_value=(changed, self.binary)):
            with self.assertRaisesRegex(qualify.QualificationError, "native_sha256"):
                qualify.execute(args, self.root)
        self.assertFalse((args.output / "bazel-output").exists())

    def test_bep_cannot_select_binary_outside_owned_output(self):
        candidate = self.root / "outside"
        candidate.write_text("not an executable from this run")
        events = [{"namedSetOfFiles": {"files": [{"name": "diffz", "uri": candidate.as_uri()}]}}]
        with self.assertRaisesRegex(qualify.QualificationError, "within this run"):
            qualify.binary_from_events(events, self.root / "target/output")


@unittest.skipUnless(sys.platform == "linux", "qualification process ownership uses Linux waitid and /proc")
class ResourceGuardTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="diffz-guard-tests-")
        self.addCleanup(self.tmp.cleanup)
        self.output = Path(self.tmp.name)
        self.policy = qualify.resource_policy(argparse.Namespace(phase_timeout_seconds=0.5,
                                                                runtime_timeout_seconds=0.5))
        for name, value in (("free_bytes", lambda _path: 8 * qualify.GIB),
                            ("INTERRUPT_GRACE_SECONDS", 0.15),
                            ("TERMINATE_GRACE_SECONDS", 0.15), ("POLL_SECONDS", 0.02)):
            replacement = patch.object(qualify, name, value)
            replacement.start()
            self.addCleanup(replacement.stop)

    def phase(self, code, name="build"):
        return qualify.run_logged(name, [sys.executable, "-u", "-c", code], self.output,
                                  dict(os.environ), self.output, self.policy)

    def failure(self, code, reason, name="build"):
        with self.assertRaises(qualify.PhaseError) as caught:
            self.phase(code, name)
        row = caught.exception.result
        self.assertEqual(row["stop_reason"], reason)
        self.assertNotEqual(row["status"], "complete")
        receipt = json.loads((self.output / "commands.jsonl").read_text().splitlines()[-1])
        self.assertEqual(receipt, row)
        if "process_group" in row:
            self.assertEqual(qualify.group_members(row["process_group"]), [])
        return row

    def test_low_disk_refuses_to_launch_and_retains_receipt(self):
        with patch.object(qualify, "free_bytes", return_value=qualify.GIB), \
             patch.object(qualify.subprocess, "Popen") as launch:
            row = self.failure("raise SystemExit('must not execute')", "insufficient_disk_before_start")
        launch.assert_not_called()
        self.assertIsNone(row["exit_code"])
        self.assertEqual(row["minimum_free_bytes"], qualify.GIB)

    def test_incoherent_proc_view_refuses_to_launch(self):
        with patch.object(qualify.os, "getpgrp", return_value=-1), \
             patch.object(qualify.subprocess, "Popen") as launch:
            row = self.failure("raise SystemExit('must not execute')", "monitor_or_launch_error")
        launch.assert_not_called()
        self.assertIn("same PID namespace", row["error"])
        self.assertIsNone(row["exit_code"])

    def test_asynchronous_child_reaping_is_rejected_before_launch(self):
        with patch.object(qualify.signal, "getsignal", return_value=signal.SIG_IGN), \
             patch.object(qualify.subprocess, "Popen") as launch:
            row = self.failure("raise SystemExit('must not execute')", "monitor_or_launch_error")
        launch.assert_not_called()
        self.assertIn("default SIGCHLD", row["error"])
        self.assertIsNone(row["exit_code"])

    def test_nonzero_child_exit_and_logs_are_not_replaced_by_guard_status(self):
        row = self.failure("import sys; print('compiler diagnostic'); sys.exit(7)", None)
        self.assertEqual(row["exit_code"], 7)
        self.assertIn("compiler diagnostic", (self.output / "build.log").read_text())
        self.assertEqual(row["cleanup"]["signals"], [])

    def test_timeout_escalates_only_the_owned_group(self):
        unrelated = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"],
                                     start_new_session=True)
        try:
            row = self.failure("import signal, time\n"
                               "signal.signal(signal.SIGINT, signal.SIG_IGN)\n"
                               "signal.signal(signal.SIGTERM, signal.SIG_IGN)\n"
                               "print('ready', flush=True)\ntime.sleep(60)\n", "phase_timeout")
            self.assertIn("ready", (self.output / "build.log").read_text())
            self.assertEqual(row["exit_code"], -signal.SIGKILL)
            self.assertEqual(row["cleanup"]["signals"], [signal.SIGINT, signal.SIGTERM, signal.SIGKILL])
            self.assertTrue(row["cleanup"]["forced_kill"])
            self.assertIsNone(unrelated.poll())
        finally:
            unrelated.kill()
            unrelated.wait(timeout=5)

    def test_interrupted_zero_exit_cannot_pass(self):
        row = self.failure("import signal, sys, time\n"
                           "signal.signal(signal.SIGINT, lambda *_: sys.exit(0))\n"
                           "print('ready', flush=True)\ntime.sleep(60)\n", "phase_timeout")
        self.assertIn("ready", (self.output / "build.log").read_text())
        self.assertEqual(row["exit_code"], 0)
        self.assertEqual(row["status"], "failed")

    def test_timeout_stops_descendants_in_the_owned_group(self):
        row = self.failure("import signal, subprocess, sys, time\n"
                           "child = subprocess.Popen([sys.executable, '-u', '-c', "
                           "\"import time; print('child ready', flush=True); time.sleep(60)\"])\n"
                           "def interrupted(*_):\n    child.wait(timeout=2)\n    sys.exit(0)\n"
                           "signal.signal(signal.SIGINT, interrupted)\n"
                           "time.sleep(60)\n", "phase_timeout")
        self.assertIn("child ready", (self.output / "build.log").read_text())
        self.assertEqual(row["exit_code"], 0)
        self.assertEqual(row["cleanup"]["signals"], [signal.SIGINT])

    def test_zero_exit_leader_with_live_descendant_cannot_pass(self):
        row = self.failure("import subprocess, sys, time\n"
                           "subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(3)'])\n"
                           "time.sleep(0.1)\n", "live_descendants_after_phase_exit")
        self.assertEqual(row["exit_code"], 0)
        self.assertEqual(row["status"], "failed")
        self.assertEqual(row["cleanup"]["signals"], [signal.SIGINT])

    def test_runtime_timeout_uses_its_own_budget_and_preserves_stdout(self):
        self.policy["phase_timeout_seconds"] = 10
        self.policy["runtime_timeout_seconds"] = 0.2
        row = self.failure("import time; print('runtime output', flush=True); time.sleep(3)",
                           "phase_timeout", name="version")
        self.assertEqual(row["timeout_seconds"], 0.2)
        self.assertLess(row["seconds"], 2)
        self.assertIn("runtime output", (self.output / "version.stdout").read_text())

    def test_disk_reserve_sends_interrupt_before_the_hard_floor(self):
        samples = 0

        def capacity(_path):
            nonlocal samples
            samples += 1
            return 8 * qualify.GIB if samples == 1 else self.policy["interrupt_free_bytes"] - 1

        with patch.object(qualify, "free_bytes", side_effect=capacity):
            row = self.failure("import time; time.sleep(3)", "disk_reserve_guard")
        self.assertEqual(row["cleanup"]["signals"], [signal.SIGINT])
        self.assertFalse(row["cleanup"]["forced_kill"])

    def test_disk_drop_below_hard_floor_kills_without_grace(self):
        samples = 0

        def capacity(_path):
            nonlocal samples
            samples += 1
            return 8 * qualify.GIB if samples == 1 else qualify.GIB

        with patch.object(qualify, "free_bytes", side_effect=capacity):
            row = self.failure("import time; time.sleep(60)", "disk_reserve_guard")
        self.assertEqual(row["exit_code"], -signal.SIGKILL)
        self.assertEqual(row["cleanup"]["signals"], [signal.SIGKILL])
        self.assertEqual(row["minimum_free_bytes"], qualify.GIB)
        self.assertTrue((self.output / "build.resources.jsonl").is_file())

    def test_monitor_failure_still_stops_the_child(self):
        samples = 0

        def broken_probe(_path):
            nonlocal samples
            samples += 1
            if samples > 1:
                raise OSError("fixture statvfs failure")
            return 8 * qualify.GIB

        with patch.object(qualify, "free_bytes", side_effect=broken_probe):
            row = self.failure("import time; time.sleep(60)", "monitor_or_launch_error")
        self.assertIn("statvfs failure", row["error"])
        self.assertIsNotNone(row["exit_code"])

    def test_cleanup_observation_failure_still_kills_and_keeps_raw_exit(self):
        with patch.object(qualify, "finish_group", side_effect=OSError("fixture proc failure")):
            row = self.failure("raise SystemExit(7)", None)
        self.assertEqual(row["exit_code"], 7)
        self.assertIn("proc failure", row["cleanup_error"])
        self.assertEqual(row["cleanup"]["fallback_signal"], signal.SIGKILL)

    def test_empty_observation_cannot_leave_the_owned_leader_running(self):
        observe = qualify.group_members
        with patch.object(qualify, "group_members", return_value=[]), \
             patch.object(qualify, "KILL_WAIT_SECONDS", 0.2):
            row = self.failure("import time; time.sleep(60)", "phase_timeout")
        self.assertEqual(row["exit_code"], -signal.SIGKILL)
        self.assertIn("observation may be incomplete", row["cleanup_error"])
        self.assertEqual(row["cleanup"]["fallback_signal"], signal.SIGKILL)
        self.assertTrue(row["cleanup"]["forced_kill"])
        self.assertEqual(observe(row["process_group"]), [])

    def test_unreadable_owned_stat_is_not_treated_as_an_absent_process(self):
        started = []
        popen, read_text = subprocess.Popen, Path.read_text

        def start(*args, **kwargs):
            process = popen(*args, **kwargs)
            started.append(process.pid)
            return process

        def unreadable(path, *args, **kwargs):
            if started and path == Path(f"/proc/{started[0]}/stat"):
                raise PermissionError("fixture owned stat is unreadable")
            return read_text(path, *args, **kwargs)

        with patch.object(qualify.subprocess, "Popen", side_effect=start), \
             patch.object(Path, "read_text", unreadable):
            row = self.failure("import time; time.sleep(60)", "phase_timeout")
        self.assertIn("owned stat is unreadable", row["cleanup_error"])
        self.assertEqual(row["exit_code"], -signal.SIGKILL)
        self.assertEqual(row["cleanup"]["fallback_signal"], signal.SIGKILL)

    def test_sigterm_cancellation_is_recorded_and_handler_is_restored(self):
        previous = signal.getsignal(signal.SIGTERM)
        samples = 0

        def interrupt(_path):
            nonlocal samples
            samples += 1
            if samples == 2:
                os.kill(os.getpid(), signal.SIGTERM)
            return 8 * qualify.GIB

        with patch.object(qualify, "free_bytes", side_effect=interrupt):
            row = self.failure("import time; time.sleep(60)", "cancelled")
        self.assertEqual(row["cancellation_signal"], signal.SIGTERM)
        self.assertEqual(signal.getsignal(signal.SIGTERM), previous)

    def test_first_cancellation_during_cleanup_is_not_discarded(self):
        original = qualify.finish_group

        def interrupt_during_cleanup(*args):
            os.kill(os.getpid(), signal.SIGTERM)
            return original(*args)

        with patch.object(qualify, "finish_group", side_effect=interrupt_during_cleanup):
            row = self.failure("raise SystemExit(0)", "cancelled")
        self.assertEqual(row["cancellation_signal"], signal.SIGTERM)
        self.assertEqual(row["exit_code"], 0)
        self.assertEqual(row["status"], "failed")

    def test_test_lock_wait_is_bounded_without_unlinking_shared_lock(self):
        test_root = str(self.output / "tests")
        lock = Path(test_root + ".lock")
        with patch.object(qualify, "TEST_ROOT", test_root), lock.open("a") as owner:
            fcntl.flock(owner.fileno(), fcntl.LOCK_EX)
            with self.assertRaisesRegex(qualify.QualificationError, "test lock"):
                with qualify.local_test_lock(0.1):
                    self.fail("must not acquire another open description's lock")
        self.assertTrue(lock.exists())


if __name__ == "__main__":
    unittest.main()
