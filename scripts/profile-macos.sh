#!/usr/bin/env bash
# Measures diffz's memory and CPU on macOS over a fixed set of scenarios, each in a fresh process,
# and writes raw samples plus summary.json and summary.md under <target>/profile/<label>/.
# docs/performance.md explains the method, the metrics and the A/B protocol for pull requests.
set -euo pipefail
cd "$(dirname "$0")/.."

usage() {
  cat >&2 <<'USAGE'
Usage: bash scripts/profile-macos.sh [options]
       bash scripts/profile-macos.sh --compare OLD/summary.json --to NEW/summary.json

  --bin FILE         profile this binary instead of building the release binary
  --label NAME       output directory name (default: the short commit, plus -dirty)
  --repeat N         runs of each scenario, interleaved (default 3)
  --scenarios A,B    run only these scenarios (default: all; see --list)
  --list             print the scenarios and exit
  --offline          skip the scenarios that need the network
  --idle-seconds N   how long the idle scenario waits (default 240)
  --attribute        after the runs, one MallocStackLogging=lite run per scenario, with
                     malloc_history and leaks --groupByType output (not in the medians)
  --compare FILE     print per-scenario deltas of this run (or of --to) against FILE
  --to FILE          with --compare and no run: the newer summary.json
  --wait SECONDS     wait up to this long for the machine to become quiet (default 0)
  --force            run even when the machine is not quiet; the summary records it
USAGE
  exit 2
}

# Scenario name, whether it needs the network, the source argument, profile steps, and what
# happens after the source settles. The window opens at 1360x900 points.
anyhow=https://github.com/dtolnay/anyhow/compare/1.0.70...1.0.81
cargo_range=https://github.com/rust-lang/cargo/compare/0.80.0...0.81.0
large_patch=/tmp/dzp/fixtures/large.patch
SCENARIOS='open-panel 0 - 0 -
f01 0 --fixture=F01 0 -
large-patch 0 PATCH 0 -
large-patch-stepped 0 PATCH 20 -
small-compare 1 ANYHOW 0 -
large-compare-fresh 1 CARGO 0 -
large-compare-stepped 1 CARGO 20 -
idle 1 CARGO 20 idle
handoff-to-f01 1 CARGO 20 handoff'

bin="" label="" repeat=3 only="" offline=0 idle_seconds=240 attribute=0 compare="" compare_to=""
wait_quiet=0 force=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --bin) bin="${2:?}"; shift 2 ;;
    --label) label="${2:?}"; shift 2 ;;
    --repeat) repeat="${2:?}"; shift 2 ;;
    --scenarios) only=",${2:?},"; shift 2 ;;
    --list) echo "$SCENARIOS" | awk '{print $1 ($2 == 1 ? " (network)" : "")}'; exit 0 ;;
    --offline) offline=1; shift ;;
    --idle-seconds) idle_seconds="${2:?}"; shift 2 ;;
    --attribute) attribute=1; shift ;;
    --compare) compare="${2:?}"; shift 2 ;;
    --to) compare_to="${2:?}"; shift 2 ;;
    --wait) wait_quiet="${2:?}"; shift 2 ;;
    --force) force=1; shift ;;
    -h|--help) usage ;;
    *) echo "unknown option: $1" >&2; usage ;;
  esac
done
[[ "$repeat" =~ ^[1-9][0-9]*$ && "$idle_seconds" =~ ^[0-9]+$ && "$wait_quiet" =~ ^[0-9]+$ ]] || usage
[[ "$(uname -s)" == Darwin ]] || { echo 'The profile runs on macOS only.' >&2; exit 2; }
for tool in footprint vmmap heap top python3; do
  command -v "$tool" >/dev/null || { echo "$tool is required; nothing was measured." >&2; exit 127; }
done

# Summaries, comparisons and the synthetic patch are computed in Python.
py() {
  python3 - "$@" <<'PYTHON'
import json, os, pathlib, random, re, statistics, sys

MB = 1_000_000
# metric, label, unit, whether larger is worse
METRICS = [
    ("phys_footprint", "phys_footprint", "MB", True),
    ("footprint_minus_iosurface", "footprint − IOSurface", "MB", True),
    ("iosurface", "IOSurface", "MB", True),
    ("phys_footprint_peak", "peak", "MB", True),
    ("malloc_dirty", "MALLOC dirty", "MB", True),
    ("heap_live", "live heap", "MB", True),
    ("ioaccelerator", "IOAccelerator", "MB", True),
    ("cpu_percent", "CPU %", "%", True),
    ("csw_per_s", "csw/s", "/s", True),
]

def parse_sample(d):
    d = pathlib.Path(d)
    m = {}
    fp = json.loads((d / "footprint.json").read_text())
    proc = fp["processes"][0]
    cats = proc["categories"]
    dirty = lambda pred: sum(v["dirty"] for k, v in cats.items() if pred(k))
    m["phys_footprint"] = proc["auxiliary"]["phys_footprint"]
    m["phys_footprint_peak"] = proc["auxiliary"]["phys_footprint_peak"]
    m["iosurface"] = dirty(lambda k: k == "IOSurface")
    m["footprint_minus_iosurface"] = m["phys_footprint"] - m["iosurface"]
    m["malloc_dirty"] = dirty(lambda k: k.startswith("MALLOC"))
    m["ioaccelerator"] = dirty(lambda k: k.startswith("IOAccelerator"))
    heap = (d / "heap.txt").read_text(errors="replace")
    h = re.search(r"All zones: (\d+) nodes \((\d+) bytes\)", heap)
    if h:
        m["heap_nodes"], m["heap_live"] = int(h[1]), int(h[2])
    top = (d / "top.txt").read_text(errors="replace")
    rows = [l.split() for l in top.splitlines() if re.match(r"^\d+\s", l)]
    # The first top sample has no interval to measure; CSW is cumulative.
    cpu = [float(r[1]) for r in rows[1:]]
    csw = [int(r[2].rstrip("+")) for r in rows]
    if cpu:
        m["cpu_percent"] = round(statistics.mean(cpu), 2)
        m["cpu_max_percent"] = max(cpu)
    if len(csw) > 1:
        m["csw_per_s"] = round((csw[-1] - csw[0]) / (len(csw) - 1), 2)
    meta = json.loads((d / "meta.json").read_text())
    m.update(meta)
    return m

def spread(values):
    values = sorted(v for v in values if v is not None)
    if not values:
        return None
    return {"median": statistics.median(values), "min": values[0], "max": values[-1], "n": len(values)}

def fmt(key, v):
    if v is None:
        return "–"
    unit = next((u for k, _, u, _ in METRICS if k == key), "")
    return f"{v / MB:.1f}" if unit == "MB" else f"{v:.2f}" if unit == "%" else f"{v:.1f}"

def summarise(out):
    out = pathlib.Path(out)
    env = json.loads((out / "raw" / "env.json").read_text())
    scenarios = {}
    for d in sorted((out / "raw").glob("*-run*")):
        if not (d / "meta.json").exists():
            continue
        name = d.name.rsplit("-run", 1)[0]
        try:
            scenarios.setdefault(name, []).append(parse_sample(d))
        except (OSError, ValueError, KeyError, IndexError) as e:
            print(f"skipped {d}: {e}", file=sys.stderr)
    order = [l.split()[0] for l in env["scenario_table"].splitlines()]
    summary = {"env": env, "scenarios": {}}
    for name in sorted(scenarios, key=lambda n: order.index(n) if n in order else 99):
        runs = scenarios[name]
        keys = [k for k, *_ in METRICS] + ["heap_nodes", "drawables", "settle_seconds", "cpu_max_percent"]
        summary["scenarios"][name] = {
            "metrics": {k: spread([r.get(k) for r in runs]) for k in keys},
            "iosurface_ok": all(r.get("iosurface_ok") for r in runs),
            "settle_timeouts": sum(1 for r in runs if r.get("settle_timeout")),
            "busy_samples": sum(1 for r in runs if r.get("busy")),
            "short_steps": sum(1 for r in runs if r.get("stepped", 0) < r.get("steps", 0)),
            "runs": runs,
        }
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    lines = [f"# diffz profile: {env['label']}", ""]
    lines.append(
        f"Commit `{env['git_sha'][:12]}`{' (dirty)' if env['git_dirty'] else ''}, binary "
        f"{env['binary_bytes'] / MB:.1f} MB, macOS {env['macos']}, display scale {env['display_scale']}, "
        f"Low Power Mode {env['low_power_mode']}, thermal state {env['thermal_state']}, "
        f"{env['repeat']} runs per scenario, idle {env['idle_seconds']} s."
    )
    if env.get("forced"):
        lines.append(f"\n**Forced on a busy machine:** {'; '.join(env['gate_failures'])}. Treat CPU numbers as noise.")
    lines += ["", "Medians in MB unless noted, with min–max over the runs.", ""]
    heads = [label for _, label, *_ in METRICS] + ["frame buffers"]
    lines.append("| Scenario | " + " | ".join(heads) + " |")
    lines.append("|---" * (len(heads) + 1) + "|")
    for name, s in summary["scenarios"].items():
        cells = []
        for k, *_ in METRICS:
            v = s["metrics"][k]
            cells.append("–" if v is None else f"{fmt(k, v['median'])} ({fmt(k, v['min'])}–{fmt(k, v['max'])})")
        dr = s["metrics"]["drawables"]
        cells.append("–" if dr is None else f"{dr['min']:g}–{dr['max']:g}" if dr["min"] != dr["max"] else f"{dr['median']:g}")
        flags = []
        if dr and dr["min"] < env.get("frame_buffers", 3):
            flags.append("frame buffers short")
        if not s["iosurface_ok"]:
            flags.append("IOSurface size unexpected")
        if s["settle_timeouts"]:
            flags.append(f"{s['settle_timeouts']} settle timeouts")
        if s["short_steps"]:
            flags.append(f"{s['short_steps']} runs stepped short")
        if s["busy_samples"]:
            flags.append(f"{s['busy_samples']} busy samples")
        lines.append(f"| {name}{' ⚠ ' + ', '.join(flags) if flags else ''} | " + " | ".join(cells) + " |")
    attr = out / "raw" / "attribute"
    if attr.is_dir():
        lines += ["", "## Attribution (MallocStackLogging=lite, not in the medians)", ""]
        for f in sorted(attr.glob("*/malloc_history.txt")):
            top = [l for l in f.read_text(errors="replace").splitlines() if re.match(r"^\s*\d+ calls? for", l)][:8]
            lines += [f"### {f.parent.name}", "", "```", *top, "```", ""]
    (out / "summary.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))

def compare(old, new):
    old, new = (json.loads(pathlib.Path(p).read_text()) for p in (old, new))
    print(f"Compare: {old['env']['label']} ({old['env']['git_sha'][:10]}) → {new['env']['label']} ({new['env']['git_sha'][:10]})")
    print("Δ is new − old medians; ≈ marks a change inside the old run's min–max spread.\n")
    keys = [(k, label) for k, label, *_ in METRICS]
    print("| Scenario | " + " | ".join(label for _, label in keys) + " |")
    print("|---" * (len(keys) + 1) + "|")
    for name, s in new["scenarios"].items():
        o = old["scenarios"].get(name)
        if not o:
            continue
        cells = []
        for k, _ in keys:
            a, b = o["metrics"].get(k), s["metrics"].get(k)
            if not a or not b:
                cells.append("–")
                continue
            delta = b["median"] - a["median"]
            pct = f" ({delta / a['median'] * 100:+.0f}%)".replace("-", "−") if a["median"] else ""
            noise = " ≈" if a["min"] <= b["median"] <= a["max"] else ""
            sign = "+" if delta >= 0 else "−"
            cells.append(f"{fmt(k, b['median'])}, {sign}{fmt(k, abs(delta))}{pct}{noise}")
        print(f"| {name} | " + " | ".join(cells) + " |")

def large_patch(path):
    # A deterministic 480-file patch, about the size of the cargo compare, for offline runs.
    rng = random.Random(68)
    words = "snapshot review anchor thread hunk layout palette registry release blame cursor frame buffer".split()
    kinds = ["rs"] * 8 + ["md", "toml"]
    out = []
    for i in range(480):
        kind = rng.choice(kinds)
        name = f"crates/{words[i % len(words)]}/src/{words[(i * 7) % len(words)]}_{i}.{kind}"
        out += [f"diff --git a/{name} b/{name}", f"index {i:07x}..{i + 1:07x} 100644", f"--- a/{name}", f"+++ b/{name}"]
        line, shift = 1, 0
        for _ in range(rng.randint(1, 6)):
            line += rng.randint(5, 80)
            ctx = [f"    let {rng.choice(words)}_{rng.randint(0, 999)} = {rng.choice(words)}(&{rng.choice(words)});" for _ in range(6)]
            removed = [f"    {rng.choice(words)}.{rng.choice(words)}({rng.randint(0, 99)});" for _ in range(rng.randint(0, 12))]
            added = [f"    {rng.choice(words)}.{rng.choice(words)}_{rng.choice(words)}({rng.randint(0, 99)}, \"{rng.choice(words)}\");" for _ in range(rng.randint(1, 15))]
            out.append(f"@@ -{line},{6 + len(removed)} +{line + shift},{6 + len(added)} @@ fn {rng.choice(words)}_{i}()")
            out += [" " + c for c in ctx[:3]] + ["-" + r for r in removed] + ["+" + a for a in added] + [" " + c for c in ctx[3:]]
            line += 6 + len(removed)
            shift += len(added) - len(removed)
    pathlib.Path(path).write_text("\n".join(out) + "\n")

cmd = sys.argv[1]
if cmd == "summarise":
    summarise(sys.argv[2])
elif cmd == "compare":
    compare(sys.argv[2], sys.argv[3])
elif cmd == "large-patch":
    large_patch(sys.argv[2])
elif cmd == "footprint":
    d = json.loads(pathlib.Path(sys.argv[2]).read_text())["processes"][0]
    ios = sum(v["dirty"] for k, v in d["categories"].items() if k == "IOSurface")
    print(d["auxiliary"]["phys_footprint"], ios)
PYTHON
}

if [[ -n "$compare" && -n "$compare_to" ]]; then
  py compare "$compare" "$compare_to"
  exit 0
fi
[[ -z "$compare_to" ]] || usage

git_sha="$(git rev-parse HEAD)"
git_dirty=false
[[ -z "$(git status --porcelain --untracked-files=no)" ]] || git_dirty=true
label="${label:-$(git rev-parse --short HEAD)$([[ "$git_dirty" == true ]] && echo -dirty)}"
target_dir="${CARGO_TARGET_DIR:-target}"
out="$target_dir/profile/$label"
if [[ -z "$bin" ]]; then
  cargo build --locked --release -p diffz
  bin="$target_dir/release/diffz"
fi
[[ -x "$bin" ]] || { echo "not an executable: $bin" >&2; exit 2; }
bin="$(cd "$(dirname "$bin")" && pwd)/$(basename "$bin")"

pids=()
cleanup() {
  local pid
  for pid in "${pids[@]+"${pids[@]}"}"; do kill "$pid" 2>/dev/null || true; done
  sleep 1
  for pid in "${pids[@]+"${pids[@]}"}"; do kill -9 "$pid" 2>/dev/null || true; done
  rm -rf /tmp/dzp
}
trap cleanup EXIT
trap 'echo "interrupted" >&2; exit 130' INT TERM

# The quiet-machine gate: CPU numbers from a busy machine are noise, and memory pressure
# makes macOS compress pages, which moves phys_footprint.
ncpu="$(sysctl -n hw.ncpu)"
max_load="${PROFILE_MAX_LOAD:-$((ncpu / 3))}"
min_free="${PROFILE_MIN_FREE_PERCENT:-25}"
load1() { sysctl -n vm.loadavg | awk '{print $2}'; }
builds_running() { pgrep -x cargo >/dev/null || pgrep -x rustc >/dev/null || pgrep -x clippy-driver >/dev/null; }
gate_failures() {
  local pressure free load
  pressure="$(sysctl -n kern.memorystatus_vm_pressure_level)"
  free="$(memory_pressure -Q | awk -F': ' '/free percentage/ {print $2+0}')"
  load="$(load1)"
  [[ "$pressure" == 1 ]] || echo "memory pressure level $pressure (want 1)"
  [[ "$free" -ge "$min_free" ]] || echo "free memory $free% (want >= $min_free%)"
  awk -v l="$load" -v m="$max_load" 'BEGIN { exit !(l > m) }' && echo "load $load (want <= $max_load)"
  builds_running && echo "cargo or rustc is running"
  return 0
}
failures="$(gate_failures)"
waited=0
while [[ -n "$failures" && "$waited" -lt "$wait_quiet" && "$force" == 0 ]]; do
  echo "waiting for a quiet machine: ${failures//$'\n'/; }" >&2
  sleep 30
  waited=$((waited + 30))
  failures="$(gate_failures)"
done
if [[ -n "$failures" ]]; then
  if [[ "$force" == 0 ]]; then
    printf 'The machine is not quiet; nothing was measured (use --force to override):\n%s\n' "$failures" >&2
    exit 3
  fi
  echo "warning: running on a busy machine: ${failures//$'\n'/; }" >&2
fi

rm -rf "$out" /tmp/dzp
mkdir -p "$out/raw" /tmp/dzp/fixtures
py large-patch "$large_patch"
scale="$(osascript -l JavaScript -e 'ObjC.import("AppKit"); $.NSScreen.mainScreen.backingScaleFactor' 2>/dev/null || echo 2)"
thermal="$(osascript -l JavaScript -e 'ObjC.import("Foundation"); ["nominal","fair","serious","critical"][$.NSProcessInfo.processInfo.thermalState]' 2>/dev/null || echo unknown)"
low_power="$(pmset -g | awk '/lowpowermode/ {print $2}')"
powermetrics=0
sudo -n true 2>/dev/null && command -v powermetrics >/dev/null && powermetrics=1
# One frame buffer: the window's pixels at 4 bytes each. IOSurface should be a whole number of them.
drawable_bytes=$((1360 * 900 * scale * scale * 4))
# GPUI's Metal layer keeps at most three; see docs/performance.md.
frame_buffers="${PROFILE_FRAME_BUFFERS:-3}"

scenario_table=""
while read -r name net source steps after; do
  [[ -z "$only" || "$only" == *",$name,"* ]] || continue
  [[ "$offline" == 1 && "$net" == 1 ]] && continue
  scenario_table+="$name $net $source $steps $after"$'\n'
done <<< "$SCENARIOS"
[[ -n "$scenario_table" ]] || { echo 'no scenario selected; see --list' >&2; exit 2; }

python3 - "$out/raw/env.json" <<PYTHON
import json, sys
json.dump({
    "label": "$label", "git_sha": "$git_sha", "git_dirty": json.loads("$git_dirty"), "binary": "$bin",
    "binary_bytes": $(stat -f %z "$bin"), "binary_version": "$("$bin" --version 2>/dev/null | head -1)",
    "macos": "$(sw_vers -productVersion) ($(sw_vers -buildVersion))", "cpus": $ncpu,
    "memory_bytes": $(sysctl -n hw.memsize), "display_scale": $scale, "window_points": "1360x900",
    "drawable_bytes": $drawable_bytes, "frame_buffers": $frame_buffers, "low_power_mode": "${low_power:-unknown}", "thermal_state": "$thermal",
    "repeat": $repeat, "idle_seconds": $idle_seconds, "offline": $offline == 1,
    "forced": $([[ -n "$failures" ]] && echo True || echo False),
    "gate_failures": [l for l in """$failures""".splitlines() if l],
    "powermetrics": $powermetrics == 1, "scenario_table": """$scenario_table""",
}, open(sys.argv[1], "w"), indent=2)
PYTHON

source_args() {
  case "$1" in
    -) ;;
    --fixture=F01) echo --fixture; echo F01 ;;
    PATCH) echo "$large_patch" ;;
    ANYHOW) echo "$anyhow" ;;
    CARGO) echo "$cargo_range" ;;
  esac
}

footprint_now() { footprint -j "$1" -f bytes "$2" >/dev/null 2>&1 && py footprint "$1"; }

# Settled means: a minimum wait, the profile steps done, then the SQLite WAL unchanged
# (releases and blame arrive after the source shows) and phys_footprint within 1 MB, both
# for 6 s. Gives up after 180 s and records the timeout.
settle() {
  local pid="$1" state="$2" log="$3" steps="$4" start wal last_wal="" fp last_fp=0 quiet=0
  start=$SECONDS
  sleep "${PROFILE_SETTLE_MIN:-8}"
  if [[ "$steps" -gt 0 ]]; then
    while ! grep -q 'diffz-profile stepped' "$log" && ((SECONDS - start < 180)); do sleep 1; done
  fi
  while ((SECONDS - start < 180)); do
    kill -0 "$pid" 2>/dev/null || return 1
    wal="$(stat -f '%z %m' "$state"/*-wal 2>/dev/null || true)"
    fp="$(footprint_now /tmp/dzp/settle.json "$pid" | awk '{print $1}')"
    if [[ "$wal" == "$last_wal" && "${fp:-0}" -gt 0 ]] && ((fp - last_fp < 1000000 && last_fp - fp < 1000000)); then
      quiet=$((quiet + 2))
      [[ "$quiet" -ge 6 ]] && { echo $((SECONDS - start)); return 0; }
    else
      quiet=0
    fi
    last_wal="$wal" last_fp="${fp:-0}"
    sleep 2
  done
  echo timeout
}

run_scenario() {
  local name="$1" source="$2" steps="$3" after="$4" dir="$5" msl="$6" last_try="$7" pid state log settled
  state="/tmp/dzp/$name"
  rm -rf "$state"
  mkdir -p "$state" "$dir"
  log="$dir/app.log"
  local args=()
  while IFS= read -r arg; do args+=("$arg"); done < <(source_args "$source")
  if [[ "$msl" == 1 ]]; then
    env DIFFZ_PROFILE_STEPS="$steps" MallocStackLogging=lite \
      "$bin" --foreground --state-dir "$state" "${args[@]+"${args[@]}"}" >"$log" 2>&1 &
  else
    env DIFFZ_PROFILE_STEPS="$steps" "$bin" --foreground --state-dir "$state" "${args[@]+"${args[@]}"}" >"$log" 2>&1 &
  fi
  pid=$!
  pids+=("$pid")
  settled="$(settle "$pid" "$state" "$log" "$steps")" || { echo "  $name: diffz exited; see $log" >&2; return 1; }
  # A window that was covered while it opened never drew enough frames to fill the pool.
  # The last attempt is sampled anyway, and the summary shows the frame-buffer count.
  local buffers
  buffers="$(footprint_now /tmp/dzp/settle.json "$pid" | awk -v f="$drawable_bytes" '{printf "%d", $2 / f + 0.5}')"
  if [[ "$buffers" -lt "$frame_buffers" && "$last_try" == 0 ]]; then
    echo "  $name: $buffers of $frame_buffers frame buffers; was the window covered? retrying" >&2
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
    return 2
  fi
  case "$after" in
    idle) sleep "$idle_seconds" ;;
    handoff)
      "$bin" --state-dir "$state" --json --fixture F01 >"$dir/handoff.json" 2>&1 || true
      settled="$(settle "$pid" "$state" "$log" 0)" || return 1
      ;;
  esac
  footprint -j "$dir/footprint.json" -f bytes "$pid" >"$dir/footprint.txt" 2>&1
  if [[ "$msl" == 1 ]]; then
    malloc_history "$pid" -allBySize >"$dir/malloc_history.txt" 2>&1 || true
    leaks --groupByType "$pid" >"$dir/leaks.txt" 2>&1 || true
  else
    local pm_pid=""
    if [[ "$powermetrics" == 1 ]]; then
      # shellcheck disable=SC2024 # the output file belongs to the user on purpose
      sudo -n powermetrics --samplers tasks --show-process-wakeups -i 1000 -n 30 >"$dir/powermetrics.txt" 2>&1 &
      pm_pid=$!
      pids+=("$pm_pid")
    fi
    top -l 31 -s 1 -pid "$pid" -stats pid,cpu,csw,idlew,power >"$dir/top.txt" 2>&1
    [[ -z "$pm_pid" ]] || wait "$pm_pid" || true
    ps -o pid=,rss=,vsz=,%cpu=,time= -p "$pid" >"$dir/ps.txt"
    vmmap --summary "$pid" >"$dir/vmmap.txt" 2>&1 || true
    heap -s "$pid" >"$dir/heap.txt" 2>&1 || true
  fi
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  local ios fp busy=False
  read -r fp ios < <(py footprint "$dir/footprint.json")
  builds_running && busy=True
  python3 - "$dir/meta.json" <<PYTHON
import json, sys
ios, frame = $ios, $drawable_bytes
n = round(ios / frame)
json.dump({
    "settle_seconds": None if "$settled" == "timeout" else int("$settled"),
    "settle_timeout": "$settled" == "timeout", "busy": $busy, "load1": float("$(load1)"),
    "memory_pressure_level": $(sysctl -n kern.memorystatus_vm_pressure_level),
    "drawables": n, "iosurface_ok": n >= 1 and abs(ios - n * frame) <= frame * 0.1,
    "steps": $steps, "stepped": int("$(sed -n 's/^diffz-profile stepped //p' "$log")" or 0),
}, open(sys.argv[1], "w"), indent=2)
PYTHON
  printf '  %-22s %6.1f MB, IOSurface %5.1f MB, settled %s\n' "$name" \
    "$(echo "$fp / 1000000" | bc -l)" "$(echo "$ios / 1000000" | bc -l)" "$settled"
}

started=$SECONDS
for run in $(seq 1 "$repeat"); do
  echo "run $run of $repeat"
  while read -r -u 3 name net source steps after; do
    [[ -n "$name" ]] || continue
    for try in 1 2 3; do
      status=0
      run_scenario "$name" "$source" "$steps" "$after" "$out/raw/$name-run$run" 0 "$((try == 3))" || status=$?
      [[ "$status" == 2 ]] || break
      rm -rf "$out/raw/$name-run$run"
    done
  done 3<<< "$scenario_table"
done
if [[ "$attribute" == 1 ]]; then
  echo "attribution"
  while read -r -u 3 name net source steps after; do
    [[ -n "$name" ]] || continue
    run_scenario "$name" "$source" "$steps" "$after" "$out/raw/attribute/$name" 1 1 || true
  done 3<<< "$scenario_table"
fi
echo "measured in $(((SECONDS - started) / 60)) min $(((SECONDS - started) % 60)) s"
py summarise "$out"
if [[ -n "$compare" ]]; then
  echo
  py compare "$compare" "$out/summary.json"
fi
echo "Results are in $out"
