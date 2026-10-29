import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "analyze-cargo-timings.py"
SPEC = importlib.util.spec_from_file_location("timing_analysis", SCRIPT)
analysis = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(analysis)


def unit(identifier, name, start, duration, frontend=None, complete=(), metadata=(), mode="todo"):
    return {
        "i": identifier, "name": name, "start": start, "duration": duration, "mode": mode,
        "unblocked_units": list(complete), "unblocked_rmeta_units": list(metadata),
        "sections": None if frontend is None else [
            ["frontend", {"start": 0, "end": frontend}],
            ["codegen", {"start": frontend, "end": duration}],
        ],
    }


def concurrency(units):
    return [
        {"t": min(row["start"] for row in units), "active": 1, "waiting": 0, "inactive": 0},
        {"t": max(row["start"] + row["duration"] for row in units), "active": 0, "waiting": 0, "inactive": 0},
    ]


def summarize(units):
    return analysis.summarize(units, concurrency(units))


def html(units, samples):
    return "<script>const UNIT_DATA = " + json.dumps(units, ensure_ascii=False) + ";\nconst CONCURRENCY_DATA = " + json.dumps(samples) + ";</script>"


class TimingAnalysisTests(unittest.TestCase):
    def test_overlap_is_not_wall_or_cpu_time(self):
        data = [unit(1, "a", 0, 10, 2, complete=(3,)), unit(2, "b", 2, 5, 1), unit(3, "app", 10, 1, 0.2)]
        result = summarize(data)
        self.assertEqual(result["observed_span_seconds"], 11)
        self.assertEqual(result["summed_overlapping_unit_seconds"], 16)
        self.assertEqual(result["active_union_seconds"], 11)
        rows = {row["name"]: row for row in result["units_by_duration"]}
        self.assertEqual(rows["a"]["exclusive_span_seconds"], 5)
        self.assertEqual(rows["b"]["exclusive_span_seconds"], 0)

    def test_metadata_gate_does_not_wait_for_parent_codegen(self):
        data = [unit(1, "a", 0, 10, 2, metadata=(2,)), unit(2, "b", 2, 10, 1, complete=(3,)), unit(3, "app", 12, 1, 0.2)]
        result = summarize(data)
        self.assertEqual([(row["name"], row["endpoint"]) for row in result["observed_gating_path"]],
                         [("a", "metadata"), ("b", "complete"), ("app", "complete")])

    def test_latest_recorded_predecessor_is_explicit(self):
        data = [unit(1, "a", 0, 5, 1, complete=(3,)), unit(2, "b", 0, 6, 1, complete=(3,)), unit(3, "app", 6, 1, 0.2)]
        final = summarize(data)["observed_gating_path"][-1]
        self.assertEqual(final["selected_predecessor"]["id"], 2)
        self.assertEqual({row["id"] for row in final["recorded_predecessors"]}, {1, 2})

    def test_ready_gap_is_not_an_invented_dependency(self):
        result = summarize([unit(1, "a", 0, 1, 0.5, metadata=(2,)), unit(2, "b", 5, 2, 1)])
        self.assertEqual(result["observed_gating_path"][1]["ready_gap_seconds"], 4.5)
        self.assertEqual(result["active_union_seconds"], 3)
        self.assertEqual(result["observed_span_seconds"], 7)

    def test_native_build_script_has_unknown_rust_phases(self):
        result = summarize([unit(1, "sqlite", 0, 7, mode="run-custom-build")])
        self.assertEqual(result["units_by_duration"][0]["section_seconds"], {})

    def test_ready_wait_integrates_samples_without_calling_it_cpu(self):
        samples = [{"t": t, "active": active, "waiting": wait, "inactive": 0}
                   for t, active, wait in ((0, 18, 2), (3, 1, 0), (7, 1, 1), (10, 0, 0))]
        result = analysis.summarize([unit(1, "a", 0, 10)], samples)
        self.assertEqual(result["concurrency"]["ready_wait_seconds"], 6)
        self.assertEqual(result["concurrency"]["max_active_invocations"], 18)

    def test_embedded_data_never_executes_javascript(self):
        data = [unit(1, "a", 0, 1)]
        text = html(data, concurrency(data)) + '<script>throw new Error("not data");</script>'
        self.assertEqual(analysis.load_units(text), data)

    def test_invalid_or_missing_unit_fields_fail(self):
        invalid = [[], [unit(1, "a", 0, -1)], [unit(1, "a", 0, float("nan"))],
                   [unit(1, "a", 0, 1), unit(1, "duplicate", 1, 2)]]
        for field in ("unblocked_units", "unblocked_rmeta_units"):
            missing = unit(1, "a", 0, 1)
            missing.pop(field)
            invalid.append([missing])
            malformed = unit(1, "a", 0, 1)
            malformed[field] = {}
            invalid.append([malformed])
        for data in invalid:
            with self.subTest(data=data), self.assertRaises(ValueError):
                analysis.load_units("const UNIT_DATA = " + json.dumps(data) + ";")

    def test_missing_and_impossible_edges_fail(self):
        with self.assertRaises(ValueError):
            analysis.load_units("const UNIT_DATA = " + json.dumps([unit(1, "a", 0, 1, metadata=(2,))]) + ";")
        with self.assertRaises(ValueError):
            summarize([unit(1, "a", 0, 2, 1, complete=(2,)), unit(2, "b", 1, 3, 1)])
        with self.assertRaises(ValueError):
            summarize([unit(1, "a", 0, 1, metadata=(2,)), unit(2, "b", 1, 2, 1)])

    def test_malformed_sections_fail(self):
        for value in (False, 0, "", {}, [["frontend", {"start": False, "end": 1}]],
                      [["frontend", {"start": 0, "end": float("inf")}]], [["frontend", {}]], [[]]):
            data = unit(1, "a", 0, 1)
            data["sections"] = value
            with self.subTest(sections=value), self.assertRaises(ValueError):
                analysis.load_units("const UNIT_DATA = " + json.dumps([data]) + ";")

    def test_cycle_outside_final_path_fails(self):
        with self.assertRaises(ValueError):
            summarize([unit(1, "a", 0, 0, complete=(2,)), unit(2, "b", 0, 0, complete=(1,)),
                       unit(3, "app", 1, 2)])

    def test_bad_concurrency_fails(self):
        for data in (None, {}, [], [{"t": -1, "active": 1, "waiting": 0, "inactive": 0}],
                     [{"t": 0, "active": float("nan"), "waiting": 0, "inactive": 0}],
                     [{"t": 0, "active": 1.5, "waiting": 0, "inactive": 0}],
                     [{"t": 2, "active": 1, "waiting": 0, "inactive": 0}, {"t": 1, "active": 0, "waiting": 0, "inactive": 0}]):
            with self.subTest(data=data), self.assertRaises(ValueError):
                analysis.load_concurrency("const CONCURRENCY_DATA = " + json.dumps(data) + ";")
        with self.assertRaises(ValueError):
            analysis.summarize([unit(1, "a", 0, 10)], [{"t": 0, "active": 1, "waiting": 0, "inactive": 0}])

    def test_cli_utf8_output_and_missing_data_exit_status(self):
        data = [unit(1, "crate-λ", 0, 1)]
        with tempfile.TemporaryDirectory(prefix="diffz-timing-tests-", dir=os.environ.get("TMPDIR", "/var/tmp")) as temporary:
            root = Path(temporary)
            source, output = root / "timing.html", root / "analysis.json"
            source.write_text(html(data, concurrency(data)), encoding="utf-8")
            result = subprocess.run([sys.executable, "-B", str(SCRIPT), str(source), "--output", str(output)], capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(output.read_text(encoding="utf-8"))["units_by_duration"][0]["name"], "crate-λ")
            for invalid in ("", "const CONCURRENCY_DATA = [];", "const CONCURRENCY_DATA = null;"):
                source.write_text("const UNIT_DATA = " + json.dumps(data) + ";\n" + invalid, encoding="utf-8")
                output.unlink()
                result = subprocess.run([sys.executable, "-B", str(SCRIPT), str(source), "--output", str(output)], capture_output=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(output.exists())
                output.write_text("old", encoding="utf-8")


if __name__ == "__main__":
    unittest.main()
