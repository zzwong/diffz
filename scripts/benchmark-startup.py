#!/usr/bin/env python3
"""Measure a Linux desktop launch through the first painted content frame.

Run in an unlocked graphical session, without other builds or test launches:
  scripts/benchmark-startup.py --binary target/release/diffz --output target/perf -- --fixture F01

Each run uses fresh SQLite state. Filesystem/driver caches are not cleared. Reports
include startup marks, settled process RSS/PSS and the kernel's peak RSS, not GPU
memory or time until the compositor presents the frame on the physical display.
Compare release binaries on the same host, viewport and input, alternating runs.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--settle", type=float, default=2.0, help="seconds after first content paint")
    parser.add_argument("--timeout", type=float, default=30.0)
    parser.add_argument("--idle-seconds", type=float, default=1.0)
    parser.add_argument("args", nargs=argparse.REMAINDER, help="diffz arguments after --")
    options = parser.parse_args()
    if options.runs < 1 or options.settle < 0 or options.idle_seconds <= 0 or options.timeout <= options.settle:
        parser.error("runs and idle-seconds must be positive; timeout must exceed nonnegative settle time")
    args = options.args[1:] if options.args[:1] == ["--"] else options.args
    if any(arg == "--state-dir" for arg in args):
        parser.error("the benchmark manages its own fresh state directories")
    output = options.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    results = []
    for run in range(options.runs):
        log_path = output / f"run-{run}.log"
        command = [str(options.binary.resolve()), "--state-dir", str(output / f"state-{run}"), *args]
        env = dict(os.environ, DIFFZ_TIMING="1")
        with log_path.open("w") as log:
            start_ms = time.time_ns() / 1_000_000
            started = time.monotonic()
            process = subprocess.Popen(command, stdout=log, stderr=log, env=env)
            try:
                marks = {}
                while time.monotonic() - started < options.timeout:
                    marks = {name: int(ms) - start_ms for name, ms in re.findall(
                        r"^diffz-timing (.+) (\d+)$", log_path.read_text(), re.MULTILINE
                    )}
                    if process.poll() is not None:
                        raise RuntimeError(f"app exited with {process.returncode}; see {log_path}")
                    paint_ms = marks.get("first content paint")
                    if paint_ms is not None and time.time_ns() / 1_000_000 >= start_ms + paint_ms + options.settle * 1000:
                        break
                    time.sleep(0.02)
                else:
                    raise RuntimeError(f"no settled content frame before timeout; see {log_path}")
                proc = Path(f"/proc/{process.pid}")
                def cpu_seconds():
                    fields = (proc / "stat").read_text().rsplit(")", 1)[1].split()
                    return sum(int(value) for value in fields[11:13]) / os.sysconf("SC_CLK_TCK")

                initial_cpu = cpu_seconds()
                idle_start = time.monotonic()
                time.sleep(options.idle_seconds)
                idle_cpu_percent = (cpu_seconds() - initial_cpu) / (time.monotonic() - idle_start) * 100
                memory = {name: int(value) for name, value in re.findall(
                    r"^(Rss|Pss|Private_Dirty|Anonymous):\s+(\d+)",
                    (proc / "smaps_rollup").read_text(), re.MULTILINE
                )}
                peak = re.search(r"^VmHWM:\s+(\d+)", (proc / "status").read_text(), re.MULTILINE)
                result = {"run": run, "marks_ms": marks, "memory_kib": memory,
                          "peak_rss_kib": int(peak[1]) if peak else None,
                          "cpu_seconds_at_settle": initial_cpu,
                          "idle_cpu_percent": idle_cpu_percent}
                results.append(result)
                print(json.dumps(result), flush=True)
                (output / "results.json").write_text(json.dumps({
                    "binary": str(options.binary.resolve()), "args": args,
                    "settle_seconds": options.settle, "idle_seconds": options.idle_seconds,
                    "runs": results,
                }, indent=2) + "\n")
            finally:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
        time.sleep(0.2)


if __name__ == "__main__":
    main()
