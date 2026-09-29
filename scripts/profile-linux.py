#!/usr/bin/env python3
"""Profile diffz in a hardware-backed Linux desktop session. See docs/performance.md."""
import argparse
import json
import os
from pathlib import Path
import platform
import random
import re
import signal
import statistics
import subprocess
import sys
import tempfile
import time


SCENARIOS = [
    ("open-panel", False, None, 0, None),
    ("f01", False, ("--fixture", "F01"), 0, None),
    ("large-patch", False, "PATCH", 0, None),
    ("large-patch-stepped", False, "PATCH", 20, None),
    ("small-compare", True, ("https://github.com/dtolnay/anyhow/compare/1.0.70...1.0.81",), 0, None),
    ("large-compare-fresh", True, ("https://github.com/rust-lang/cargo/compare/0.80.0...0.81.0",), 0, None),
    ("large-compare-stepped", True, ("https://github.com/rust-lang/cargo/compare/0.80.0...0.81.0",), 20, None),
    ("idle", True, ("https://github.com/rust-lang/cargo/compare/0.80.0...0.81.0",), 20, "idle"),
    ("handoff-to-f01", True, ("https://github.com/rust-lang/cargo/compare/0.80.0...0.81.0",), 20, "handoff"),
]
METRICS = ("pss_bytes", "private_dirty_bytes", "gpu_memory_bytes", "heap_in_use_bytes",
           "heap_arena_bytes", "cpu_percent", "voluntary_csw_per_s", "nonvoluntary_csw_per_s")
ROOT = Path(__file__).resolve().parent.parent


def command(*args):
    return subprocess.run(args, cwd=ROOT, text=True, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, check=True).stdout.strip()


def scenario_line(spec):
    name, network, source, steps, after = spec
    token = ("-" if source is None else "PATCH" if source == "PATCH" else
             "--fixture=F01" if source == ("--fixture", "F01") else
             "ANYHOW" if name == "small-compare" else "CARGO")
    return f"{name} {int(network)} {token} {steps} {after or '-'}"


def make_patch(path):
    rng = random.Random(68)
    words = "snapshot review anchor thread hunk layout palette registry release blame cursor frame buffer".split()
    kinds = ["rs"] * 8 + ["md", "toml"]
    out = []
    for i in range(480):
        kind = rng.choice(kinds)
        name = f"crates/{words[i % len(words)]}/src/{words[(i * 7) % len(words)]}_{i}.{kind}"
        out += [f"diff --git a/{name} b/{name}", f"index {i:07x}..{i+1:07x} 100644",
                f"--- a/{name}", f"+++ b/{name}"]
        line, shift = 1, 0
        for _ in range(rng.randint(1, 6)):
            line += rng.randint(5, 80)
            ctx = [f"    let {rng.choice(words)}_{rng.randint(0,999)} = {rng.choice(words)}(&{rng.choice(words)});" for _ in range(6)]
            removed = [f"    {rng.choice(words)}.{rng.choice(words)}({rng.randint(0,99)});" for _ in range(rng.randint(0, 12))]
            added = [f"    {rng.choice(words)}.{rng.choice(words)}_{rng.choice(words)}({rng.randint(0,99)}, \"{rng.choice(words)}\");" for _ in range(rng.randint(1, 15))]
            out.append(f"@@ -{line},{6+len(removed)} +{line+shift},{6+len(added)} @@ fn {rng.choice(words)}_{i}()")
            out += [" " + c for c in ctx[:3]] + ["-" + r for r in removed] + ["+" + a for a in added] + [" " + c for c in ctx[3:]]
            line += 6 + len(removed)
            shift += len(added) - len(removed)
    path.write_text("\n".join(out) + "\n")


def read_proc(pid):
    root = Path(f"/proc/{pid}")
    stat = (root / "stat").read_text().rsplit(")", 1)[1].split()
    status = (root / "status").read_text()
    rollup = (root / "smaps_rollup").read_text()
    fields = {k: int(v) * 1024 for k, v in re.findall(r"^(Pss|Private_Dirty|Rss):\s+(\d+) kB", rollup, re.M)}
    switches = {k: int(v) for k, v in re.findall(r"^(voluntary_ctxt_switches|nonvoluntary_ctxt_switches):\s+(\d+)", status, re.M)}
    gpu = {}
    for fd in (root / "fdinfo").iterdir():
        try:
            info = fd.read_text()
        except (OSError, PermissionError):
            continue
        for key, value, unit in re.findall(r"^(drm-memory-[\w-]+):\s+(\d+)\s*(KiB|MiB|B)?", info, re.M):
            # Several handles can report the same DRM client. Keep each category's maximum.
            amount = int(value) * {"KiB": 1024, "MiB": 1024 * 1024, "B": 1, "": 1}[unit]
            gpu[key] = max(gpu.get(key, 0), amount)
    gpu_total = sum(gpu[k] for k in ("drm-memory-vram", "drm-memory-gtt") if k in gpu)
    return {"pss_bytes": fields.get("Pss"), "private_dirty_bytes": fields.get("Private_Dirty"),
            "rss_bytes": fields.get("Rss"), "gpu_memory_bytes": gpu_total if any(k in gpu for k in ("drm-memory-vram", "drm-memory-gtt")) else None,
            "drm_memory_bytes": gpu, "cpu_ticks": int(stat[11]) + int(stat[12]),
            "voluntary_csw": switches.get("voluntary_ctxt_switches"),
            "nonvoluntary_csw": switches.get("nonvoluntary_ctxt_switches")}


def heap_sample(path):
    try:
        rows = [json.loads(x) for x in path.read_text().replace("\0", "").splitlines()]
        return rows[-1] if rows else {}
    except (OSError, ValueError):
        return {}


def quiet_failures():
    failures = []
    load = os.getloadavg()[0]
    max_load = float(os.getenv("PROFILE_MAX_LOAD", os.cpu_count() / 3))
    if load > max_load:
        failures.append(f"load {load:.1f} > {max_load:.1f}")
    mem = {k: int(v) for k, v in re.findall(r"^(MemTotal|MemAvailable):\s+(\d+)", Path("/proc/meminfo").read_text(), re.M)}
    free = 100 * mem["MemAvailable"] / mem["MemTotal"]
    minimum = float(os.getenv("PROFILE_MIN_FREE_PERCENT", "25"))
    if free < minimum:
        failures.append(f"available memory {free:.1f}% < {minimum:.1f}%")
    for name in ("cargo", "rustc", "clippy-driver"):
        if subprocess.run(["pgrep", "-x", name], stdout=subprocess.DEVNULL).returncode == 0:
            failures.append(f"{name} running")
    for proc in Path("/proc").glob("[0-9]*/cmdline"):
        try:
            argv = [x.decode(errors="replace") for x in proc.read_bytes().split(b"\0") if x]
            if (argv and Path(argv[0]).name.startswith("python") and
                    any(Path(arg).name == "profile-linux.py" for arg in argv[1:]) and
                    int(proc.parent.name) != os.getpid()):
                failures.append("another Linux profile running")
                break
        except (OSError, ValueError):
            pass
    return failures


def settle(proc, state, log, steps):
    start = time.monotonic()
    minimum = float(os.getenv("PROFILE_SETTLE_MIN", "8"))
    stable_for = float(os.getenv("PROFILE_STABLE_SECONDS", "6"))
    timeout = float(os.getenv("PROFILE_SETTLE_TIMEOUT", "180"))
    reference, since = None, None
    while time.monotonic() - start < timeout:
        if proc.poll() is not None:
            raise RuntimeError(f"diffz exited with {proc.returncode}; see {log}")
        if time.monotonic() - start < minimum:
            time.sleep(1)
            continue
        stepped = re.search(r"diffz-profile stepped (\d+)", log.read_text(errors="replace"))
        if steps and not stepped:
            time.sleep(1)
            continue
        wal = tuple(sorted((p.name, p.stat().st_size, p.stat().st_mtime_ns) for p in state.glob("*-wal")))
        pss = read_proc(proc.pid)["pss_bytes"]
        if reference and wal == reference[0] and pss is not None and abs(pss - reference[1]) < 1_000_000:
            if time.monotonic() - since >= stable_for:
                return round(time.monotonic() - start), False
        else:
            reference, since = (wal, pss), time.monotonic()
        time.sleep(2)
    return round(time.monotonic() - start), True


def stop(proc):
    if proc.poll() is None:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()


def stop_state_processes(state, keep_pid):
    # A failed hand-off can start another diffz. The state path is unique to this invocation.
    for path in Path("/proc").glob("[0-9]*/cmdline"):
        try:
            pid = int(path.parent.name)
            args = path.read_bytes().split(b"\0")
            if pid != keep_pid and b"--state-dir" in args and os.fsencode(state) in args:
                os.kill(pid, signal.SIGTERM)
        except (OSError, ValueError):
            pass


def run_one(bin_path, spec, run, out, temp, idle_seconds, sample_seconds, malloc_trim):
    name, _, source, steps, after = spec
    directory = out / "raw" / f"{name}-run{run}"
    directory.mkdir(parents=True)
    state = temp / f"s{run}-{name}"
    state.mkdir()
    args = (str(temp / "large.patch"),) if source == "PATCH" else source or ()
    log = directory / "app.log"
    heap = directory / "heap.jsonl"
    env = dict(os.environ, DIFFZ_PROFILE_STEPS=str(steps), DIFFZ_PROFILE_HEAP=str(heap))
    if malloc_trim:
        env["DIFFZ_PROFILE_MALLOC_TRIM"] = "1"
    with log.open("w") as writer:
        proc = subprocess.Popen([str(bin_path), "--foreground", "--state-dir", str(state), *args],
                                cwd=ROOT, env=env, stdout=writer, stderr=subprocess.STDOUT)
        try:
            elapsed, timeout = settle(proc, state, log, steps)
            handoff_status = None
            if after == "idle":
                time.sleep(idle_seconds)
            elif after == "handoff":
                handoff_env = env.copy()
                handoff_env.pop("DIFFZ_PROFILE_HEAP", None)
                handoff = subprocess.run([str(bin_path), "--state-dir", str(state), "--json", "--fixture", "F01"],
                                         cwd=ROOT, env=handoff_env, capture_output=True, text=True, timeout=30)
                (directory / "handoff.json").write_text(handoff.stdout + handoff.stderr)
                try:
                    handoff_status = json.loads(handoff.stdout.splitlines()[-1])["status"]
                except (ValueError, IndexError, KeyError):
                    handoff_status = "failed"
                elapsed, timeout = settle(proc, state, log, 0)
            start = read_proc(proc.pid)
            samples = [{"seconds": 0, **start}]
            for second in range(sample_seconds):
                time.sleep(1)
                samples.append({"seconds": second + 1, **read_proc(proc.pid)})
            (directory / "proc.json").write_text(json.dumps(samples, indent=2) + "\n")
            end = samples[-1]
            ticks = os.sysconf("SC_CLK_TCK")
            h = heap_sample(heap)
            result = {
                "pss_bytes": end["pss_bytes"], "private_dirty_bytes": end["private_dirty_bytes"],
                "gpu_memory_bytes": end["gpu_memory_bytes"], "drm_memory_bytes": end["drm_memory_bytes"],
                "heap_in_use_bytes": h.get("uordblks"), "heap_arena_bytes": h.get("arena", 0) + h.get("hblkhd", 0) if h else None,
                "cpu_percent": round(100 * (end["cpu_ticks"] - start["cpu_ticks"]) / ticks / sample_seconds, 3),
                "voluntary_csw_per_s": round((end["voluntary_csw"] - start["voluntary_csw"]) / sample_seconds, 2),
                "nonvoluntary_csw_per_s": round((end["nonvoluntary_csw"] - start["nonvoluntary_csw"]) / sample_seconds, 2),
                "settle_seconds": elapsed, "settle_timeout": timeout,
                "handoff_status": handoff_status, "steps": steps,
                "stepped": int(re.search(r"diffz-profile stepped (\d+)", log.read_text(errors="replace")).group(1)) if steps and re.search(r"diffz-profile stepped (\d+)", log.read_text(errors="replace")) else 0,
                "busy": bool(quiet_failures()),
            }
            (directory / "meta.json").write_text(json.dumps(result, indent=2) + "\n")
            print(f"  {name}: PSS {result['pss_bytes']/1e6:.1f} MB, CPU {result['cpu_percent']:.2f}%", flush=True)
            return result
        except (OSError, RuntimeError, subprocess.TimeoutExpired) as error:
            result = {"failed": str(error)}
            (directory / "meta.json").write_text(json.dumps(result, indent=2) + "\n")
            print(f"  {name}: {error}", file=sys.stderr, flush=True)
            return result
        finally:
            stop(proc)
            stop_state_processes(state, proc.pid)


def spread(values):
    values = sorted(x for x in values if x is not None)
    return {"median": statistics.median(values), "min": values[0], "max": values[-1], "n": len(values)} if values else None


def summarize(out, env):
    scenarios = {}
    for spec in SCENARIOS:
        name = spec[0]
        runs = [json.loads(p.read_text()) for p in sorted((out / "raw").glob(f"{name}-run*/meta.json"))]
        if not runs:
            continue
        scenarios[name] = {
            "metrics": {key: spread([r.get(key) for r in runs]) for key in METRICS},
            "runs": runs,
            "failed_runs": sum("failed" in r for r in runs),
            "settle_timeouts": sum(bool(r.get("settle_timeout")) for r in runs),
            "busy_samples": sum(bool(r.get("busy")) for r in runs),
            "short_steps": sum(r.get("stepped", 0) < r.get("steps", 0) for r in runs),
            "failed_handoffs": sum(r.get("handoff_status") not in (None, "handed_off") for r in runs),
        }
        scenarios[name]["flags"] = [f"{scenarios[name][key]} {label}{'s' if key == 'busy_samples' and scenarios[name][key] != 1 else ''}" for key, label in (
            ("failed_runs", "runs failed"), ("settle_timeouts", "settle timeouts"),
            ("busy_samples", "busy sample"), ("short_steps", "runs stepped short"),
            ("failed_handoffs", "hand-offs failed")) if scenarios[name][key]]
    summary = {"env": env, "scenarios": scenarios}
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    lines = [f"# diffz Linux profile: {env['label']}", "", f"Commit `{env['git_sha'][:12]}`, {env['session_type']} on {env['desktop']}, {env['gpu']}, {env['driver']}; {env['repeat']} runs.", "", "MB are decimal; medians with min–max.", "", "| Scenario | PSS MB | Private dirty MB | DRM memory MB | Heap in use MB | Heap arena MB | CPU % | voluntary csw/s | runs |", "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"]
    for name, row in scenarios.items():
        cells = []
        for key in ("pss_bytes", "private_dirty_bytes", "gpu_memory_bytes", "heap_in_use_bytes", "heap_arena_bytes", "cpu_percent", "voluntary_csw_per_s"):
            s = row["metrics"][key]
            scale = 1 if key in ("cpu_percent", "voluntary_csw_per_s") else 1e6
            cells.append("–" if not s else f"{s['median']/scale:.2f} ({s['min']/scale:.2f}–{s['max']/scale:.2f})")
        flags = ", ".join(row["flags"])
        lines.append(f"| {name}{' ⚠ ' + flags if flags else ''} | " + " | ".join(cells) + f" | {len(row['runs']) - row['failed_runs']} |")
    (out / "summary.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))
    return summary


def compare(old, new):
    print(f"Compare: {old['env']['label']} → {new['env']['label']}")
    labels = ("PSS MB", "private dirty MB", "DRM memory MB", "heap in use MB",
              "heap arena MB", "CPU %", "voluntary csw/s", "nonvoluntary csw/s")
    print("Δ is new − old medians; ≈ marks a change inside the old min–max range.\n")
    print("| Scenario | " + " | ".join(labels) + " | runs (old → new) | flags |")
    print("| --- | " + " | ".join("---:" for _ in METRICS) + " | ---: | --- |")
    for name, row in new["scenarios"].items():
        prior = old["scenarios"].get(name, {})
        cells = []
        for key in METRICS:
            a, b = prior.get("metrics", {}).get(key), row["metrics"].get(key)
            if not a or not b:
                cells.append("–")
            else:
                scale = 1 if key in ("cpu_percent", "voluntary_csw_per_s", "nonvoluntary_csw_per_s") else 1e6
                delta = (b["median"] - a["median"]) / scale
                cells.append(f"{delta:+.2f}{' ≈' if a['min'] <= b['median'] <= a['max'] else ''}")
        flags = [f"old: {flag}" for flag in prior.get("flags", [])] + [f"new: {flag}" for flag in row.get("flags", [])]
        print(f"| {name} | " + " | ".join(cells) +
              f" | {len(prior.get('runs', []))} → {len(row['runs'])} | {'; '.join(flags) or '–'} |")


def main():
    def interrupt(_signum, _frame):
        raise KeyboardInterrupt

    signal.signal(signal.SIGTERM, interrupt)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin", type=Path)
    parser.add_argument("--label")
    parser.add_argument("--repeat", type=int, default=3)
    parser.add_argument("--scenarios")
    parser.add_argument("--list", action="store_true")
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--idle-seconds", type=int, default=240)
    parser.add_argument("--sample-seconds", type=int, default=30)
    parser.add_argument("--compare", type=Path)
    parser.add_argument("--to", type=Path)
    parser.add_argument("--wait", type=int, default=0)
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--malloc-trim", action="store_true", help="log the return value of the scheduled glibc trim (Linux/glibc production behavior)")
    args = parser.parse_args()
    if args.list:
        print("\n".join(s[0] + (" (network)" if s[1] else "") for s in SCENARIOS))
        return
    if args.compare and args.to:
        compare(json.loads(args.compare.read_text()), json.loads(args.to.read_text()))
        return
    if args.to or args.repeat < 1 or args.idle_seconds < 0 or args.sample_seconds < 1 or args.wait < 0:
        parser.error("invalid option combination or duration")
    if platform.system() != "Linux" or not (os.getenv("WAYLAND_DISPLAY") or os.getenv("DISPLAY")):
        parser.error("run in a Linux desktop session")
    selected = set(args.scenarios.split(",")) if args.scenarios else {s[0] for s in SCENARIOS}
    unknown = selected - {s[0] for s in SCENARIOS}
    if unknown:
        parser.error(f"unknown scenarios: {', '.join(sorted(unknown))}")
    specs = [s for s in SCENARIOS if s[0] in selected and not (args.offline and s[1])]
    if not specs:
        parser.error("no scenarios selected")
    bin_path = args.bin.expanduser().resolve() if args.bin else ROOT / "target/release/diffz"
    if args.bin is None:
        subprocess.run(["cargo", "build", "--locked", "--release", "-p", "diffz"], cwd=ROOT, check=True)
    if not os.access(bin_path, os.X_OK):
        parser.error(f"binary is not executable: {bin_path}")
    deadline = time.monotonic() + args.wait
    failures = quiet_failures()
    while failures and time.monotonic() < deadline and not args.force:
        print("waiting for quiet machine: " + "; ".join(failures), file=sys.stderr)
        time.sleep(min(30, max(0, deadline - time.monotonic())))
        failures = quiet_failures()
    if failures and not args.force:
        parser.error("machine is not quiet: " + "; ".join(failures))
    sha = command("git", "rev-parse", "HEAD")
    dirty = bool(command("git", "status", "--porcelain", "--untracked-files=no"))
    label = args.label or sha[:7] + ("-dirty" if dirty else "")
    out = ROOT / os.getenv("CARGO_TARGET_DIR", "target") / "profile" / label
    if out.exists():
        parser.error(f"output exists: {out}; choose another label")
    out.mkdir(parents=True)
    (out / "raw").mkdir()
    try:
        gpu = subprocess.run(["lspci"], text=True, capture_output=True).stdout
    except OSError:
        gpu = ""
    gpu = next((x.split(": ", 1)[-1] for x in gpu.splitlines() if "Display controller:" in x or "VGA compatible controller:" in x), "unknown")
    try:
        driver = subprocess.run(["glxinfo", "-B"], text=True, capture_output=True).stdout
    except OSError:
        driver = ""
    driver_output = driver
    driver = next((x.split(":", 1)[-1].strip() for x in driver_output.splitlines() if x.startswith("OpenGL renderer string:")), "unknown")
    driver_version = next((x.split(":", 1)[-1].strip() for x in driver_output.splitlines() if x.startswith("OpenGL core profile version string:")), "unknown")
    env = {"label": label, "git_sha": sha, "git_dirty": dirty, "binary": str(bin_path),
           "binary_bytes": bin_path.stat().st_size, "binary_version": command(str(bin_path), "--version"),
           "linux": platform.platform(), "session_type": os.getenv("XDG_SESSION_TYPE", "unknown"),
           "desktop": os.getenv("XDG_CURRENT_DESKTOP", "unknown"), "gpu": gpu, "driver": driver,
           "driver_version": driver_version,
           "wayland_display": os.getenv("WAYLAND_DISPLAY"), "x11_display": os.getenv("DISPLAY"),
           "client_backend": "wayland" if os.getenv("WAYLAND_DISPLAY") else "x11",
           "window_points": "1360x900", "repeat": args.repeat, "idle_seconds": args.idle_seconds,
           "sample_seconds": args.sample_seconds, "offline": args.offline, "forced": bool(failures),
           "malloc_trim": args.malloc_trim,
           "max_load": float(os.getenv("PROFILE_MAX_LOAD", os.cpu_count() / 3)),
           "min_free_percent": float(os.getenv("PROFILE_MIN_FREE_PERCENT", "25")),
           "gate_failures": failures, "scenario_table": "\n".join(scenario_line(s) for s in specs)}
    (out / "raw" / "env.json").write_text(json.dumps(env, indent=2) + "\n")
    with tempfile.TemporaryDirectory(prefix="dzp.", dir="/tmp") as root:
        temp = Path(root)
        make_patch(temp / "large.patch")
        for run in range(1, args.repeat + 1):
            print(f"run {run} of {args.repeat}", flush=True)
            for spec in specs:
                run_one(bin_path, spec, run, out, temp, args.idle_seconds, args.sample_seconds, args.malloc_trim)
    summary = summarize(out, env)
    if args.compare:
        compare(json.loads(args.compare.read_text()), summary)
    print(f"Results are in {out}")


if __name__ == "__main__":
    main()
