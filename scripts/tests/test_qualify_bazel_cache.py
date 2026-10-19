"""Qualification policy tests with synthetic BEP and a subprocess Bazel fixture.

These do not claim a Rust build or a real cross-host cache result.
"""

import argparse
import copy
import importlib.util
import json
import os
from pathlib import Path
import shutil
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
    raise SystemExit(0)
def option(name):
    return next(x.split("=", 1)[1] for x in args if x.startswith(name + "="))
output = Path(option("--output_user_root"))
bep = Path(option("--build_event_json_file"))
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
    runners = [{"name": "internal", "count": 2},
               {"name": "processwrapper-sandbox" if local else "remote cache hit", "count": 12}]
    events = [{"buildMetrics": {"actionSummary": {"runnerCount": runners}}},
              {"namedSetOfFiles": {"files": [{"name": "diffz", "uri": binary.as_uri()}]}}]
else:
    events = [{"id": {"testResult": {"label": "//:core_tests"}}, "testResult": {"status": "PASSED"}},
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
            self.assertIn("--disk_cache=", row["argv"])
            self.assertIn("--remote_upload_local_results=false", row["argv"])
            self.assertIn("--remote_executor=", row["argv"])
            self.assertIn("--build_event_json_file_path_conversion=false", row["argv"])
        self.assertIn("--cache_test_results=no", commands[1]["argv"])
        self.assertIn("--test_strategy=local", commands[1]["argv"])
        self.assertIn("--state-dir", commands[-1]["argv"])
        state = Path(commands[-1]["argv"][-1])
        self.assertTrue(state.is_relative_to(self.root / "target/consumer/runtime"))
        self.assertTrue((self.root / "target/consumer/comparison.json").is_file())

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
        self.assertIn("shutdown", report)
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

    def test_shutdown_failure_prevents_success_publication_and_preserves_primary_error(self):
        self.execute(self.args("producer"))
        original_capture = qualify.capture

        def fail_shutdown(argv, **kwargs):
            if "shutdown" in argv:
                return {"exit_code": 9, "stdout": "", "stderr": "fixture shutdown failed"}
            return original_capture(argv, **kwargs)

        with patch.object(qualify, "capture", side_effect=fail_shutdown):
            success_args = self.args("consumer", "-shutdown-failed")
            report = self.execute(success_args)
            self.assertEqual(report["status"], "failed")
            self.assertIn("cleanup_error", report)
            self.assertFalse((success_args.output / "comparison.json").exists())
            failure_args = self.args("consumer", "-build-and-shutdown-failed")
            with patch.dict(os.environ, {"QUALIFICATION_FIXTURE_LOCAL_BUILD": "1"}):
                with self.assertRaisesRegex(qualify.QualificationError, "uncached build"):
                    self.execute(failure_args)
            report = json.loads((failure_args.output / "result.json").read_text())
            self.assertIn("uncached build", report["error"])
            self.assertIn("shutdown failed", report["cleanup_error"])
            self.assertFalse((failure_args.output / "comparison.json").exists())

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


if __name__ == "__main__":
    unittest.main()
