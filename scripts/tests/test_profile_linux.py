import importlib.util
from contextlib import redirect_stdout
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock


REPO = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "profile_linux", REPO / "scripts/profile-linux.py"
)
PROFILE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROFILE)


def fdinfo(*fields):
    return "\n".join(fields) + "\n"


class LinuxDrmMemoryTests(unittest.TestCase):
    def test_xe_resident_regions_ignore_overlapping_categories(self):
        total, categories, estimated = PROFILE.parse_drm_memory([
            fdinfo(
                "drm-driver:\txe",
                "drm-pdev:\t0000:00:02.0",
                "drm-client-id:\t42",
                "drm-total-system:\t8 MiB",
                "drm-shared-gtt:\t4 MiB",
                "drm-active-gtt:\t1 MiB",
                "drm-resident-system:\t0",
                "drm-total-gtt:\t82028 KiB",
                "drm-resident-gtt:\t82028 KiB",
                "drm-purgeable-gtt:\t12 MiB",
                "drm-resident-stolen:\t0",
            )
        ])

        self.assertEqual(total, 82028 * 1024)
        self.assertEqual(categories, {
            "drm-resident-gtt": 82028 * 1024,
            "drm-resident-stolen": 0,
            "drm-resident-system": 0,
        })
        self.assertFalse(estimated)

    def test_amd_legacy_alias_and_resident_key_are_not_added(self):
        total, categories, estimated = PROFILE.parse_drm_memory([
            fdinfo(
                "drm-pdev: 0000:03:00.0",
                "drm-client-id: 7",
                "drm-memory-vram: 2048 KiB",
                "drm-resident-vram: 2 MiB",
                "drm-memory-gtt: 128 KiB",
            ),
            fdinfo(
                "drm-pdev: 0000:03:00.0",
                "drm-client-id: 7",
                "drm-memory-vram: 2 MiB",
                "drm-resident-gtt: 128 KiB",
            ),
        ])

        self.assertEqual(total, 2 * 1024 * 1024 + 128 * 1024)
        self.assertEqual(categories, {
            "drm-resident-gtt": 128 * 1024,
            "drm-resident-vram": 2 * 1024 * 1024,
        })
        self.assertFalse(estimated)

    def test_modern_resident_key_overrides_legacy_alias_including_zero(self):
        total, categories, estimated = PROFILE.parse_drm_memory([
            fdinfo(
                "drm-pdev: 0000:03:00.0",
                "drm-client-id: 8",
                "drm-memory-vram: 3 MiB",
                "drm-resident-vram: 2 MiB",
                "drm-memory-gtt: 1 MiB",
                "drm-resident-gtt: 0",
            )
        ])

        self.assertEqual(total, 2 * 1024 * 1024)
        self.assertEqual(categories, {
            "drm-resident-gtt": 0,
            "drm-resident-vram": 2 * 1024 * 1024,
        })
        self.assertFalse(estimated)

    def test_independent_clients_sum_but_duplicate_fds_do_not(self):
        total, categories, estimated = PROFILE.parse_drm_memory([
            fdinfo("drm-pdev: 0000:03:00.0", "drm-client-id: 10",
                   "drm-resident-gtt: 100 B", "drm-resident-vram: 2 B"),
            fdinfo("drm-pdev: 0000:03:00.0", "drm-client-id: 10",
                   "drm-resident-gtt: 100 B", "drm-resident-vram: 2 B"),
            fdinfo("drm-pdev: 0000:03:00.0", "drm-client-id: 11",
                   "drm-resident-gtt: 200 B", "drm-resident-vram: 1 B"),
            # Client IDs can be device-scoped, so the same ID on another
            # device is an independent client.
            fdinfo("drm-pdev: 0000:04:00.0", "drm-client-id: 10",
                   "drm-resident-gtt: 50 B"),
        ])

        self.assertEqual(total, 353)
        self.assertEqual(categories, {
            "drm-resident-gtt": 350,
            "drm-resident-vram": 3,
        })
        self.assertFalse(estimated)

    def test_absent_keys_are_unknown_but_reported_zero_is_zero(self):
        self.assertEqual(
            PROFILE.parse_drm_memory([fdinfo("drm-total-gtt: 1 MiB")]),
            (None, {}, None),
        )
        self.assertEqual(
            PROFILE.parse_drm_memory([
                fdinfo("drm-pdev: 0000:03:00.0", "drm-client-id: 12",
                       "drm-resident-gtt: 0", "drm-active-gtt: 9 MiB")
            ]),
            (0, {"drm-resident-gtt": 0}, False),
        )

    def test_units_and_dynamic_region_names(self):
        total, categories, estimated = PROFILE.parse_drm_memory([
            fdinfo(
                "drm-pdev: 0000:03:00.0",
                "drm-client-id: 13",
                "drm-resident-vram0: 7 B",
                "drm-resident-gtt: 2KiB",
                "drm-resident-system: 1 MiB",
                "drm-resident-stolen: 3",
            )
        ])

        self.assertEqual(total, 1_050_634)
        self.assertEqual(categories, {
            "drm-resident-gtt": 2 * 1024,
            "drm-resident-stolen": 3,
            "drm-resident-system": 1024 * 1024,
            "drm-resident-vram0": 7,
        })
        self.assertFalse(estimated)

    def test_missing_client_ids_use_conservative_per_device_maxima(self):
        total, categories, estimated = PROFILE.parse_drm_memory([
            fdinfo("drm-pdev: 0000:03:00.0", "drm-resident-gtt: 100 B"),
            fdinfo("drm-pdev: 0000:03:00.0", "drm-resident-gtt: 200 B",
                   "drm-resident-vram: 10 B"),
            fdinfo("drm-pdev: 0000:04:00.0", "drm-resident-gtt: 300 B"),
        ])

        self.assertEqual(total, 510)
        self.assertEqual(categories, {
            "drm-resident-gtt": 500,
            "drm-resident-vram": 10,
        })
        self.assertTrue(estimated)

        # Without a PCI identity, unknown-device entries also use one
        # conservative bucket rather than assuming the descriptors are unique.
        self.assertEqual(
            PROFILE.parse_drm_memory([
                fdinfo("drm-resident-gtt: 100 B"),
                fdinfo("drm-resident-gtt: 200 B"),
            ]),
            (200, {"drm-resident-gtt": 200}, True),
        )


class LinuxSettleEvidenceTests(unittest.TestCase):
    """Exercise production orchestration, not a replacement profiling model.

    Process startup/handoff, /proc sampling, settling, sleeping and cleanup are
    mocked at their native boundaries. Actual run_one, JSON persistence,
    settle_evidence and summarize execute against owned temporary files. These
    tests prove evidence retention, not native settling or performance.
    """

    SAMPLE = {
        "pss_bytes": 1_000_000, "private_dirty_bytes": 500_000,
        "gpu_memory_bytes": None, "drm_memory_bytes": {},
        "gpu_memory_estimated": None, "cpu_ticks": 0,
        "voluntary_csw": 0, "nonvoluntary_csw": 0,
    }

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="diffz-settle-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.counter = 0

    def run_profile(self, outcomes, *, handoff=True, handoff_error=None):
        self.counter += 1
        out = self.root / f"out-{self.counter}"
        temp = self.root / f"state-{self.counter}"
        (out / "raw").mkdir(parents=True)
        temp.mkdir()
        name = "handoff-to-f01" if handoff else "f01"
        spec = next(spec for spec in PROFILE.SCENARIOS if spec[0] == name)
        process = mock.Mock(pid=31337)

        def started(*args, **kwargs):
            # The UI step marker is an input to this evidence-only fixture.
            kwargs["stdout"].write(f"diffz-profile stepped {spec[3]}\n")
            kwargs["stdout"].flush()
            return process

        handoff_result = PROFILE.subprocess.CompletedProcess(
            [], 0, '{"status":"handed_off"}\n', ""
        )
        with (mock.patch.object(PROFILE.subprocess, "Popen", side_effect=started) as launch,
              mock.patch.object(PROFILE.subprocess, "run", return_value=handoff_result,
                                side_effect=handoff_error) as send,
              mock.patch.object(PROFILE, "settle", side_effect=outcomes) as settle,
              mock.patch.object(PROFILE, "read_proc", return_value=self.SAMPLE.copy()) as sample,
              mock.patch.object(PROFILE, "heap_sample", return_value={}),
              mock.patch.object(PROFILE, "quiet_failures", return_value=[]),
              mock.patch.object(PROFILE.time, "sleep"),
              mock.patch.object(PROFILE.os, "sysconf", return_value=100),
              mock.patch.object(PROFILE, "stop") as stop,
              mock.patch.object(PROFILE, "stop_state_processes") as cleanup,
              redirect_stdout(io.StringIO())):
            result = PROFILE.run_one(
                self.root / "never-executed-diffz", spec, 1, out, temp, 0, 1, False
            )
        launch.assert_called_once()
        self.assertEqual(send.call_count, int(handoff))
        stop.assert_called_once_with(process)
        cleanup.assert_called_once_with(temp / f"s1-{name}", process.pid)
        self.assertEqual(
            json.loads((out / "raw" / f"{name}-run1" / "meta.json").read_text()),
            result,
        )
        env = {"label": "fixture", "git_sha": "0" * 40, "session_type": "fixture",
               "desktop": "fixture", "gpu": "fixture", "driver": "fixture", "repeat": 1}
        with redirect_stdout(io.StringIO()):
            summary = PROFILE.summarize(out, env)
        self.assertEqual(json.loads((out / "summary.json").read_text()), summary)
        return result, summary["scenarios"][name], settle, sample

    def test_initial_timeout_survives_successful_handoff_and_summary(self):
        result, summary, settle, sample = self.run_profile([(180, True), (14, False)])
        # This is the original regression: baseline reports False here.
        self.assertTrue(result["settle_timeout"])
        self.assertEqual(result["settle_timeout_stages"], ["initial"])
        self.assertEqual(result["settle_stages"], {
            "initial": {"seconds": 180, "timeout": True},
            "handoff": {"seconds": 14, "timeout": False},
        })
        self.assertEqual(result["settle_seconds"], 14)
        self.assertEqual(summary["settle_timeouts"], 1)
        self.assertIn("1 settle timeouts", summary["flags"])
        self.assertEqual(summary["failed_runs"], 0)
        self.assertEqual(summary["short_steps"], 0)
        self.assertEqual([call.args[3] for call in settle.call_args_list], [20, 0])
        self.assertEqual(sample.call_count, 2)

    def test_handoff_timeout_and_both_timeouts_retain_stage_names(self):
        for initial_timeout in (False, True):
            with self.subTest(initial_timeout=initial_timeout):
                result, summary, _, _ = self.run_profile([
                    (180 if initial_timeout else 14, initial_timeout), (180, True)
                ])
                self.assertTrue(result["settle_timeout"])
                self.assertEqual(result["settle_timeout_stages"],
                                 (["initial"] if initial_timeout else []) + ["handoff"])
                self.assertEqual(result["settle_stages"]["initial"]["timeout"], initial_timeout)
                self.assertEqual(result["settle_stages"]["handoff"],
                                 {"seconds": 180, "timeout": True})
                self.assertEqual(result["settle_seconds"], 180)
                # Count timed-out runs, not the number of timed-out stages.
                self.assertEqual(summary["settle_timeouts"], 1)

    def test_successful_handoff_is_not_flagged(self):
        result, summary, _, _ = self.run_profile([(14, False), (16, False)])
        self.assertFalse(result["settle_timeout"])
        self.assertEqual(result["settle_timeout_stages"], [])
        self.assertEqual(result["settle_stages"], {
            "initial": {"seconds": 14, "timeout": False},
            "handoff": {"seconds": 16, "timeout": False},
        })
        self.assertEqual(result["settle_seconds"], 16)
        self.assertEqual(summary["flags"], [])

    def test_nonhandoff_preserves_its_single_settle_stage(self):
        for timeout in (False, True):
            with self.subTest(timeout=timeout):
                elapsed = 180 if timeout else 14
                result, summary, settle, _ = self.run_profile([(elapsed, timeout)], handoff=False)
                self.assertEqual(result["settle_timeout"], timeout)
                self.assertEqual(result["settle_timeout_stages"], ["initial"] if timeout else [])
                self.assertEqual(result["settle_stages"], {
                    "initial": {"seconds": elapsed, "timeout": timeout}
                })
                self.assertEqual(result["settle_seconds"], elapsed)
                self.assertEqual(summary["settle_timeouts"], int(timeout))
                self.assertEqual([call.args[3] for call in settle.call_args_list], [0])

    def test_later_handoff_failure_keeps_completed_initial_timeout(self):
        result, summary, settle, sample = self.run_profile(
            [(180, True)], handoff_error=OSError("handoff fixture unavailable")
        )
        self.assertEqual(result["failed"], "handoff fixture unavailable")
        self.assertTrue(result["settle_timeout"])
        self.assertEqual(result["settle_timeout_stages"], ["initial"])
        self.assertEqual(result["settle_stages"], {
            "initial": {"seconds": 180, "timeout": True}
        })
        self.assertEqual(result["settle_seconds"], 180)
        self.assertEqual(summary["failed_runs"], 1)
        self.assertEqual(summary["settle_timeouts"], 1)
        self.assertEqual(settle.call_count, 1)
        sample.assert_not_called()

    def test_later_settle_failure_keeps_completed_initial_timeout(self):
        result, summary, settle, sample = self.run_profile([
            (180, True), RuntimeError("fixture process exited during handoff settle")
        ])
        self.assertIn("during handoff settle", result["failed"])
        self.assertTrue(result["settle_timeout"])
        self.assertEqual(result["settle_timeout_stages"], ["initial"])
        self.assertEqual(result["settle_stages"], {
            "initial": {"seconds": 180, "timeout": True}
        })
        self.assertEqual(result["settle_seconds"], 180)
        self.assertEqual(summary["failed_runs"], 1)
        self.assertEqual(summary["settle_timeouts"], 1)
        self.assertEqual(settle.call_count, 2)
        sample.assert_not_called()

    def test_failure_before_any_settle_completes_does_not_invent_a_timeout(self):
        result, summary, settle, sample = self.run_profile(
            [RuntimeError("fixture process exited during initial settle")], handoff=False
        )
        self.assertIn("during initial settle", result["failed"])
        self.assertFalse(result["settle_timeout"])
        self.assertEqual(result["settle_timeout_stages"], [])
        self.assertEqual(result["settle_stages"], {})
        self.assertIsNone(result["settle_seconds"])
        self.assertEqual(summary["failed_runs"], 1)
        self.assertEqual(summary["settle_timeouts"], 0)
        self.assertEqual(settle.call_count, 1)
        sample.assert_not_called()


if __name__ == "__main__":
    unittest.main()
