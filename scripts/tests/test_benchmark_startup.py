#!/usr/bin/env python3
"""Startup harness process-lifetime fixtures; no desktop or native app required."""
import json
import os
import signal
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "benchmark-startup.py"


@unittest.skipUnless(sys.platform == "linux", "startup harness reads Linux /proc")
class StartupProcessTests(unittest.TestCase):
    def invoke(self, paint):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "fake-diffz"
            stopped = root / "stopped"
            arguments = root / "arguments.json"
            binary.write_text(f'''#!{sys.executable}
import json, pathlib, signal, sys, time
pathlib.Path({str(arguments)!r}).write_text(json.dumps(sys.argv[1:]))
# The real desktop launcher exits after handing off without --foreground.
if "--foreground" not in sys.argv:
    sys.exit(0)
def stop(*args):
    pathlib.Path({str(stopped)!r}).write_text("terminated")
    sys.exit(0)
signal.signal(signal.SIGTERM, stop)
if {paint!r}:
    print("diffz-timing first content paint", time.time_ns() // 1_000_000, flush=True)
while True:
    time.sleep(0.01)
''')
            binary.chmod(0o755)
            output = root / "report"
            command = [sys.executable, str(SCRIPT), "--binary", str(binary),
                       "--output", str(output), "--runs", "1", "--settle", "0",
                       "--timeout", "2", "--idle-seconds", "0.01", "--", "--fixture", "F01"]
            # Bound the whole fixture process group, including a fake app left
            # behind if the harness itself regresses and stops cleaning up.
            with subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                  text=True, start_new_session=True) as process:
                try:
                    stdout, stderr = process.communicate(timeout=10)
                finally:
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                result = subprocess.CompletedProcess(command, process.returncode, stdout, stderr)
            self.assertEqual(stopped.read_text(), "terminated")
            self.assertEqual(json.loads(arguments.read_text()),
                             ["--foreground", "--state-dir", str(output / "state-0"),
                              "--fixture", "F01"])
            receipt = output / "results.json"
            return result, json.loads(receipt.read_text()) if receipt.exists() else None

    def test_foreground_process_is_measured_and_stopped(self):
        result, receipt = self.invoke(paint=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(receipt["runs"]), 1)
        self.assertIn("first content paint", receipt["runs"][0]["marks_ms"])
        self.assertGreater(receipt["runs"][0]["memory_kib"]["Rss"], 0)

    def test_no_paint_timeout_stops_foreground_process(self):
        result, receipt = self.invoke(paint=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no settled content frame before timeout", result.stderr)
        self.assertIsNone(receipt)


if __name__ == "__main__":
    unittest.main()
