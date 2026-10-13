# Performance

## Linux profiling

`scripts/profile-linux.sh` runs the same nine scenarios and writes the same
top-level `summary.json` layout (`env`, `scenarios`, per-scenario `metrics` and
`runs`) as the macOS harness. Linux metric names differ because `/proc` and DRM
account for memory differently. Run it in an unlocked, hardware-backed Wayland
or Xorg session. Record separate baselines for each display server on the same
GPU, driver, viewport and binary. Xwayland under a Wayland compositor tests an
X11 client path but is not an Xorg session baseline.

```sh
bash scripts/profile-linux.sh --bin /path/to/diffz --label wayland-main
bash scripts/profile-linux.sh --bin /path/to/diffz --label xorg-main
bash scripts/profile-linux.sh --offline --repeat 1 --scenarios f01,large-patch
bash scripts/profile-linux.sh --compare target/profile/wayland-main/summary.json --to target/profile/wayland-fix/summary.json
```

Omit `--bin` to build the normal release binary first. The script checks one
minute load, available memory, other builds and other profiles before any
measurement; `--wait SECONDS` waits for a quiet machine and `--force` records
the override in `env.gate_failures`. Set `PROFILE_MAX_LOAD` or
`PROFILE_MIN_FREE_PERCENT` only when the host's normal idle level requires it,
and disclose the setting. A forced run is not a reliable CPU baseline.

Each scenario gets a fresh process and empty `--state-dir` under the script's
private `/tmp/dzp.*` directory. `DIFFZ_PROFILE_STEPS` saturates the frame pool
and steps files through the same hook as the macOS run. The script waits at
least eight seconds, then for an unchanged SQLite WAL and PSS within 1 MB for
six seconds, with a 180-second timeout. It records 30 one-second `/proc`
samples after settling; `--sample-seconds` can shorten a diagnostic run. The
`idle` scenario first waits 240 seconds by default. Interrupted runs stop the
processes started against the private state directories.

| Metric | Source | Meaning |
| --- | --- | --- |
| `pss_bytes` | `/proc/PID/smaps_rollup` | Proportional resident memory. Shared pages are split across processes. |
| `private_dirty_bytes` | `smaps_rollup` | Private dirty CPU-mapped pages, including glibc arenas and other mutable regions. |
| `gpu_memory_bytes` | Resident DRM categories in `/proc/PID/fdinfo` | Sum of per-client resident bytes by region, deduplicating matching `(drm-pdev, drm-client-id)` descriptors. Legacy `drm-memory-*` keys normalize to `drm-resident-*`; this client-accounting total is not unique physical GPU bytes because shared buffers may be charged to multiple clients. |
| `heap_in_use_bytes` | glibc `mallinfo2().uordblks` | glibc allocator in-use counter. It includes freed chunks retained in per-thread tcache and excludes mmap-backed bytes in `hblkhd`, so it is not exact application-live memory. |
| `heap_arena_bytes` | glibc `mallinfo2().arena + hblkhd` | Legacy combined arena-plus-mmap counter. `arena`, `fordblks`, and `hblkhd` are separate raw fields in `heap.jsonl`; the difference between this combined value and `uordblks` does not isolate allocator retention or fragmentation. |
| `cpu_percent` | `/proc/PID/stat` | Process user plus system CPU time over the sample period, where 100% is one core. |
| `voluntary_csw_per_s` | `/proc/PID/status` | Voluntary context switches per second, a wakeup proxy. Nonvoluntary switches are also saved. |

The DRM parser reads `drm-resident-<region>` and the deprecated amdgpu
`drm-memory-<region>` alias. It emits per-region totals in `drm_memory_bytes`
under canonical `drm-resident-<region>` keys and uses resident values only; it
does not add `drm-total-*`, `drm-shared-*`, `drm-active-*`, or
`drm-purgeable-*`, which are separate or overlapping views and are not additive
to resident accounting. No resident keys means
`gpu_memory_bytes` is null; present keys whose values are all zero produce
zero. The kernel format accepts bytes with optional `KiB` or `MiB` units.

The parser deduplicates descriptors by device and client ID, then adds distinct
clients. The resulting client-accounting total is not unique physical GPU bytes:
buffers shared between clients may be charged to each client, and aggregate
fdinfo values cannot identify and deduplicate those shared buffers. When a
resident-bearing descriptor lacks `drm-client-id`, it uses a conservative
per-device, per-region maximum; descriptors without `drm-pdev` but with a
client ID remain distinct by the globally unique client ID. If both device and
client identity are missing, descriptors use one shared unknown-device bucket.
This fallback avoids double-counting aliased file descriptors but can
undercount independent clients; the sample and run record
`gpu_memory_estimated: true` and should not be called an exact total. See the
kernel's [DRM client usage statistics
format](https://docs.kernel.org/gpu/drm-usage-stats.html#memory).
Older profile summaries used per-category maxima and counted only VRAM and GTT;
the corrected parser sums distinct clients and all resident regions. Keep the
older GPU figures as historical observations, and rerun both sides with the
corrected parser before making a GPU-memory A/B comparison.

The heap sampler is compiled only on glibc Linux and starts only with
`DIFFZ_PROFILE_HEAP` set. `heap.jsonl` and per-second `proc.json` are kept under
each run's `raw/` directory. Missing DRM fdinfo or heap fields stay null in the
summary; do not interpret them as zero. The glibc counters are distinct:
`arena` is system bytes held by allocator arenas, `fordblks` is free arena
chunks reported by the allocator, `uordblks` is its in-use counter, and
`hblkhd` is the separate mmap-backed byte total. Tcache-held frees are omitted from
`fordblks` and therefore remain in `uordblks`; direct mmap bytes are reported
separately in `hblkhd`. Do not infer retained/free arena bytes by subtracting
`uordblks` from the combined `arena + hblkhd` summary field.

These figures overlap, so do not sum PSS, private dirty, GPU resident and heap
values. `/proc` cannot directly count swapchain images; check the wgpu surface
configuration and driver traces before claiming a specific image count. GPUI's
Wayland window requests Mailbox and falls back to FIFO if unsupported; its X11
window uses FIFO. The wgpu surface configuration requests maximum frame latency
2; its Vulkan backend asks for at least three swapchain images
(`maximum_frame_latency + 1`). The actual compositor allocation needs a separate
observation, as recorded in the Wayland baseline below.

On Linux/glibc, diffz schedules glibc `malloc_trim(0)` on a background thread
three seconds after an active snapshot is replaced. A later replacement cancels
the pending trim. Starting another accepted source load also cancels it to avoid
allocator contention; a failed load or an offered revision reschedules against
the active snapshot. The delay lets snapshot readers finish and release old data
first. The profile script's `--malloc-trim` option sets
`DIFFZ_PROFILE_MALLOC_TRIM`, which logs the trim return value to stderr for the
handoff diagnostic; it does not enable or disable the production behavior.

For an A/B run, build both binaries before profiling, run main then branch in
the same session, and use `--compare`. The table marks changes inside the old
run's min–max spread with `≈`. Keep Wayland and Xorg comparisons separate.

### Linux baselines

On 2026-09-29, a release build at `7e2dc6f` plus this Linux profile hook ran
all nine scenarios three times on Fedora 44, GNOME Wayland, an AMD Strix Halo
Radeon 8060S, Mesa 26.1.6, kernel 7.1.8, and a 1360×900 requested window.
The run used Vulkan and a real GPU. Numbers below are median decimal MB or
percent of one CPU core; the complete min–max ranges and raw samples are in
`target/profile/wayland-amd-7e2dc6f/` on the measurement host.

| Scenario | PSS | Private dirty | GPU resident | allocator in-use counter | glibc arena + mmap | CPU % | voluntary csw/s |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Open panel | 77.9 | 17.3 | 58.5 | 8.7 | 16.7 | 0.93 | 6.0 |
| F01 | 87.6 | 26.1 | 58.5 | 14.0 | 26.5 | 0.07 | 0 |
| Large patch | 99.2 | 34.7 | 58.5 | 18.8 | 36.7 | 0.07 | 0 |
| Large patch, stepped | 102.2 | 37.4 | 58.5 | 20.0 | 37.4 | 0.07 | 0 |
| Small compare | 91.8 | 30.0 | 58.5 | 10.8 | 32.2 | 0.07 | 0 |
| Large compare, fresh | 122.0 | 59.2 | 58.5 | 16.8 | 68.4 | 0.10 | 0 |
| Large compare, stepped | 132.3 | 66.8 | 58.5 | 18.6 | 77.2 | 0.13 | 0 |
| Four minute idle | 118.4 | 65.7 | 58.5 | 18.7 | 73.2 | 0.13 | 0 |
| Hand-off to F01 | 130.2 | 77.1 | 58.6 | 19.2 | 87.9 | 0.07 | 0 |

Unrelated Cargo builds started during parts of the run. The script flagged two
of three Open panel, F01, patch, small compare and idle samples, and one of
three large compare and hand-off samples as busy. CPU values above are the
observed medians, not a quiet-machine CPU baseline. Private dirty, glibc
allocator counters and GPU-resident totals stayed close across repeats. PSS
moved substantially when shared pages were charged differently: the idle range
was 70.9–130.8 MB while private dirty stayed within 65.7–65.9 MB. Repeat the CPU
baseline when the host stays quiet for a full pass.

The Open panel woke about six times a second and used 0.83–0.93% CPU, consistent
with the cursor-blink issue #69. Review views showed zero voluntary context
switches in many 30-second samples and 0.03–0.13% CPU, supporting a parked
Wayland frame loop rather than the macOS display-link problem in #76. A hand-off
to F01 left 50.9 MB more private dirty memory, 5.2 MB more allocator in-use
counter and 61.4 MB more combined glibc arena-plus-mmap bytes than a fresh F01
process; GPU residency did not increase. Because `uordblks` includes
tcache-held frees and `hblkhd` is separate mmap accounting, these counters do
not isolate retained arena memory. This motivates the #70 allocator experiment
and the #71 cache work.

An earlier diagnostic used the same binary with `--malloc-trim`: three
hand-off repeats returned `malloc_trim 1` and lowered the medians to 48.0 MB
private dirty and 101.7 MB PSS, from 77.1 MB and 130.2 MB without trim. The
ranges were 47.6–49.1 MB private dirty and 101.5–103.0 MB PSS. The allocator
in-use counter stayed at 19.2 MB and GPU residency at 58.6 MB. The combined
glibc arena-plus-mmap counter stayed near 88 MB; because `hblkhd` is separate
mmap-backed accounting, that combined value cannot establish how many arena
pages remained reserved. `malloc_trim` can also reduce resident pages without
changing the arena's reported address-space size. This diagnostic informed the
delayed production trim; verify its effect and navigation behavior with the A/B
protocol before claiming a production result.

The Vulkan code requests a 1360×900 window and up to two frames of latency.
A one-off diagnostic build instrumented `wgpu-hal`'s Vulkan swapchain creation
and queried the returned images. On this Wayland session, after an initial
64×64 FIFO surface and a 1360×900 Mailbox surface, the settled window had
**four 1700×1125 Mailbox images**. The final extent reflects the compositor's
1.25 scale. The ordinary profile measured 58.4–58.6 MB of GPU-resident memory, which
also includes resources other than those four images. The diagnostic
instrumentation was kept outside this repository and excluded from the baseline
binary. Issues #69 and #71–#73 remain open at this baseline, so their proposed
shared-code fixes have no before/after Linux result yet.

There is no Xorg session on this host. An Xwayland diagnostic can test the X11
client path, but the required real Xorg baseline remains open. For that
diagnostic, `WAYLAND_DISPLAY=` forced GPUI's X11 backend while GNOME Wayland
continued as compositor. The offline Open panel, F01 and large patch scenarios
used 90.5–90.8 MB GPU-resident memory, about 32 MB more than Wayland. The Vulkan
diagnostic found **three 2720×1800 FIFO images** in the settled
Xwayland window, after an initial 64×64 FIFO surface. The larger image extent
accounts for much of the GPU-resident difference; it reflects Xwayland's scaling
on this host and may differ in a real Xorg session. Three repeated
Open panel runs showed 65.3–65.9 voluntary switches/s and 0.70–0.73% CPU;
three F01 runs showed 60.13 switches/s and 0.10–0.17% CPU. The X11 backend's
visible-window refresh timer fires at the monitor rate even when it passes
`force_render: false`. Issue #88 tracks making that timer demand driven. These
numbers demonstrate wakeups, not a full redraw on every tick. One of the three
repeated samples in each view was flagged busy; the context-switch rate stayed
the same in the clean samples.

`scripts/profile-macos.sh` measures diffz's memory and CPU on macOS over a fixed
set of scenarios, so a change can be compared against a baseline. Each scenario
runs in a fresh process, several times, and the script writes raw samples and a
summary. It needs Xcode's command line tools (`footprint`, `vmmap`, `heap`) and
Python 3.
An SSH connection is enough on a headless Mac with an online display, as long
as its console desktop is unlocked. The script checks the live session and keeps
the display active while it runs.

```sh
bash scripts/profile-macos.sh                        # build the release binary, run everything
bash scripts/profile-macos.sh --bin /path/to/diffz --label main
bash scripts/profile-macos.sh --offline --repeat 1   # no network, one run each
bash scripts/profile-macos.sh --scenarios f01,idle --idle-seconds 60
bash scripts/profile-macos.sh --compare target/profile/main/summary.json --to target/profile/fix/summary.json
bash scripts/profile-macos.sh --help
```

Results go to `target/profile/<label>/` (under `CARGO_TARGET_DIR` when it is
set): `raw/` holds every tool's output for each run, `summary.json` the parsed
numbers, and `summary.md` the table. The label defaults to the short commit,
with `-dirty` for uncommitted changes. A full pass with three runs of each
scenario takes about 40 minutes, 12 of them in the idle scenario.

## Scenarios

| Scenario | What runs | Network |
| --- | --- | --- |
| `open-panel` | `diffz` with no source: the Open panel | no |
| `f01` | `--fixture F01` | no |
| `large-patch` | A 480-file patch the script generates deterministically (1.3 MB) | no |
| `large-patch-stepped` | The same patch after stepping through 20 files | no |
| `small-compare` | `dtolnay/anyhow` 1.0.70...1.0.81 | yes |
| `large-compare-fresh` | `rust-lang/cargo` 0.80.0...0.81.0, as opened | yes |
| `large-compare-stepped` | The cargo compare after stepping through 20 files | yes |
| `idle` | The stepped cargo compare after `--idle-seconds` (default 240) | yes |
| `handoff-to-f01` | The stepped cargo compare, then `diffz --fixture F01` hands off to it | yes |

`--offline` skips the network scenarios. The compares go through `gh`, so it must
be signed in. The tags are fixed, so the content does not change between runs.

## Method

Each run starts `diffz --foreground --state-dir /tmp/dzp.XXXXXX/<scenario>` with
an empty state directory, and sets `DIFFZ_PROFILE_STEPS`. The `/tmp/dzp.XXXXXX`
root is private to one invocation (short, because the state directory holds a
Unix socket), and the script stops every diffz it started and removes the root
when it exits, including on Ctrl-C.

- **Frame buffers first.** With the variable set, the window redraws on every
  frame for about two seconds after it opens, so GPUI's pool of frame buffers
  grows to its maximum of three on every run. A run that ends up with fewer,
  usually because the window was covered while it opened, is retried up to
  twice.
- **Stepping.** After the first source installs, diffz selects the next file
  N times, 300 ms apart, through the same path as `]`, and writes
  `diffz-profile stepped <count>` to stderr. The script waits for that line, and
  the summary flags a run that stepped short. A later install in the same
  process, such as the hand-off, does not step. Without the variable, none of
  this runs.
- **Settling.** The script waits 8 s, then until the SQLite WAL in the state
  directory has not changed and `phys_footprint` has stayed within 1 MB of its
  value at the start of that stretch, for 6 s.
  Releases and blame load after a compare shows, so memory keeps moving for a
  while after the window looks finished. A run that has not settled after 180 s
  is sampled anyway and flagged.
- **Sampling,** in this order, because the later tools briefly suspend the
  process: `footprint`, then 31 one-second samples of `top` (the first has no
  interval and is dropped), `powermetrics --samplers tasks` alongside when
  `sudo -n` works, then `ps`, `vmmap --summary` and `heap -s`.
- **Hand-off.** `handoff-to-f01` requires `diffz --json` to report
  `handed_off`. Any other result is recorded in the run's `meta.json` and
  flagged, and a window it launched instead is stopped.
- **Failures.** A run whose diffz exits early or whose `footprint` sample fails
  is recorded as failed rather than dropped, so the summary and `--compare`
  count it.
- **Repeats.** `--repeat N` (default 3) runs every scenario once per round,
  round after round, so slow drift in the machine spreads across scenarios. The
  summary gives the median with the minimum and maximum.

The window opens at 1360×900 points on the screen that has the key window
(`NSScreen.mainScreen`), and the script reads the scale of that screen. The script checks that
IOSurface is a whole number of frame buffers of that size at the display's
scale, which catches a window that opened at another size, and records how many
there were.

### Quiet machine

The script refuses to run unless the memory pressure level
(`kern.memorystatus_vm_pressure_level`) is 1, at least 25% of memory is free
(`memory_pressure -Q`), the one-minute load is at most a third of the CPU count,
no `cargo`, `rustc` or `clippy-driver` is running, and no other
`profile-macos.sh` is running. `--wait SECONDS` polls
every 30 s until the machine is quiet. `--force` runs anyway, and the summary
says which checks failed; treat its CPU numbers as noise. `PROFILE_MAX_LOAD` and
`PROFILE_MIN_FREE_PERCENT` change the limits. Each run also records the load and
pressure level at the time, and whether a build had started.

`summary.json` records the commit and whether the tree was dirty, the binary's
size and version, the macOS version, the display scale, Low Power Mode, the
thermal state, and whether `powermetrics` was available.

## Metrics

| Metric | Source | Meaning |
| --- | --- | --- |
| `phys_footprint` | `footprint` | What macOS charges the process for: dirty and compressed memory, including GPU surfaces. Activity Monitor's "Memory" column. |
| footprint − IOSurface | computed | The headline for allocator and cache work, with the frame buffers taken out. |
| IOSurface | `footprint` | Frame buffers. The headline for Metal drawable changes. |
| peak | `footprint` | `phys_footprint_peak`: the highest footprint since launch, which catches transient spikes while a source loads. |
| MALLOC dirty | `footprint` | Dirty pages in every `MALLOC_*` region. |
| live heap | `heap -s` | Bytes in live allocations. MALLOC dirty minus live heap is fragmentation and freed memory the allocator has kept. |
| IOAccelerator | `footprint` | Metal buffers and textures: the glyph atlas, instance buffers, path textures. |
| CPU % | `top` | Mean `%CPU` over 30 s after settling. |
| csw/s | `top` | Context switches per second over the same 30 s: how often the process wakes up. |

`vmmap --summary` is kept in `raw/` for the malloc zone table (allocated bytes and
fragmentation per zone) and region counts.

### Reading footprint categories

`footprint`'s dirty column already includes compressed pages; its swapped column
is a subset of dirty, not something to add. Reclaimable memory is excluded from
`phys_footprint`.

**IOSurface moves in steps of 19.6 MB.** A frame buffer at 1360×900 points on a
2x display is 2720×1800 pixels at 4 bytes each: 19,584,000 bytes. GPUI's Metal
layer allocates another buffer whenever it asks for a frame while the earlier
ones are still in use, up to three, and never gives them back. The count depends
on how many frames were drawn back to back, not on what the scenario did, so
without the redraw burst one run holds 19.6 MB and the next 58.8 MB for the same
work. The harness saturates the pool, so IOSurface is about 59.8 MB (three
buffers plus small surfaces) in every scenario, and the difference between two
runs lives in footprint − IOSurface. A larger window or display scale makes each
step bigger.

`__TEXT`, `__DATA_CONST` and mapped files are clean or shared and do not count
toward `phys_footprint`.

### CPU and wakeups

`top`'s `%CPU` and cumulative `CSW` are sampled every second for 30 s. Idle
wakeups (`IDLEW`) are recorded in `raw/` but are not a metric: macOS counts a
wakeup there only when it takes a core out of idle, so on a machine doing
anything else the count drops towards zero while the process wakes just as
often. Context switches count every wakeup. With passwordless `sudo`,
`powermetrics --samplers tasks --show-process-wakeups` output is saved next to
each run for a second opinion; it is not parsed.

## Attribution

`--attribute` adds one run of each scenario, after the measured runs, with
`MallocStackLogging=lite`, and saves `malloc_history -allBySize` and
`leaks --groupByType` for it under `raw/attribute/`. `summary.md` lists the
largest allocation sites. Stack logging adds its own memory, so these runs are
not in the medians. The release profile strips symbols, so build a binary that
keeps them for readable stacks:

```sh
CARGO_PROFILE_RELEASE_STRIP=none cargo build --locked --release -p diffz
cp target/release/diffz /tmp/diffz-symbols
bash scripts/profile-macos.sh --bin /tmp/diffz-symbols --label attribution --attribute --repeat 1
```

## Caveats

- Do not use or cover the diffz window during a run, and keep the display awake
  and unlocked. The harness rejects a locked desktop at startup and holds a
  user-active display assertion while it runs. Each window takes focus as it
  opens; a covered window or a sleeping display stops drawing, which changes
  both memory and CPU. If a run
  still has too few frame buffers or is not frontmost after three attempts, the
  harness records it as invalid and exits with an error rather than reporting
  its idle CPU. It also rejects a run that loses focus during CPU sampling.
- The window opens on the screen with the key window, not necessarily the
  built-in one. Record runs that are compared on the same display arrangement,
  with focus on the same screen; an external monitor at another scale changes every
  IOSurface number.
- Low Power Mode and thermal throttling change CPU numbers but not memory; both
  are recorded, and the two sides of an A/B run should match.
- The network scenarios depend on GitHub's response time. Settling waits for the
  data, but a slow or rate-limited `gh` makes those runs slower and noisier.
- The numbers are for macOS only. Linux renders through wgpu with different
  buffers and allocator.

## Adding a scenario

Add a line to `SCENARIOS` in the script: the name, `1` if it needs the network,
the source token, the number of files to step through, and what happens after
settling (`-`, `idle` or `handoff`). Add the token to `source_args`, and a row to
the table above. Keep names stable: `--compare` matches scenarios by name, and a
renamed scenario loses its history.

## A/B protocol for pull requests

A change that claims a memory or CPU effect carries a comparison in its PR:

1. Build both binaries first and copy them out of the target directory, so
   neither build runs during a measurement:
   ```sh
   git switch main && cargo build --locked --release -p diffz && cp target/release/diffz /tmp/diffz-main
   git switch my-branch && cargo build --locked --release -p diffz && cp target/release/diffz /tmp/diffz-branch
   ```
2. With no builds running, run the two back to back in one session, on the same
   display and power settings:
   ```sh
   bash scripts/profile-macos.sh --bin /tmp/diffz-main --label main
   bash scripts/profile-macos.sh --bin /tmp/diffz-branch --label branch \
     --compare target/profile/main/summary.json
   ```
   If the spread is wide, run main again afterwards and compare against both.
3. Paste the `--compare` table into the PR body. Its last two columns give the
   runs behind each median and any flags (failed runs, short frame buffers,
   failed hand-offs, settle timeouts); rerun a flagged scenario before drawing a
   conclusion from it. A change counts when the
   targeted metric moves beyond the old run's min–max spread (the table marks a
   change inside it with ≈), and no other scenario gets worse by more than its
   spread.

## Baseline

Release build of `3991807` (main at `51589ae` plus the profile hook, which does
nothing unless `DIFFZ_PROFILE_STEPS` is set), measured on 2026-09-29 on an Apple
Silicon Mac with 32 GB, macOS 15.7.9, the built-in display at 2x, Low Power Mode
on, thermal state nominal, `--repeat 3`. Megabytes (10⁶ bytes); medians with
min–max. The measured runs took 40 minutes.

| Scenario | phys_footprint | footprint − IOSurface | IOSurface | peak | live heap | CPU % | csw/s | runs |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `open-panel` | 94.8 (93.0–101.8) | 35.0 (33.2–42.0) | 59.8 | 101.9 (101.1–102.5) | 10.5 (10.1–10.8) | 0.07 (0.00–1.42) | 22 (16–351) | 3 |
| `f01` | 103.2 (102.8–103.2) | 43.4 (42.9–43.4) | 59.8 | 110.9 (110.4–130.8) | 16.3 (16.0–16.3) | 0.49 (0.00–0.56) | 301 (1–314) | 3 |
| `large-patch` | 101.6 (100.8–102.1) | 41.8 (40.9–42.3) | 59.8 | 109.3 (108.6–129.7) | 21.2 (20.8–21.2) | 0.49 (0.00–0.57) | 313 (2–314) | 3 |
| `large-patch-stepped` | 107.5 (105.4–110.5) | 47.7 (45.5–50.7) | 59.8 | 115.3 (113.1–118.2) | 22.1 (21.5–22.1) | 0.51 (0.25–0.55) | 313 (112–314) | 3 |
| `small-compare` | 103.5 (100.8–106.5) | 43.7 (40.9–46.6) | 59.8 | 111.2 (109.2–114.2) | 14.6 (14.4–14.7) | 0.55 (0.18–0.55) | 313 (93–315) | 3 |
| `large-compare-fresh` | 112.7 (112.2–117.5) | 52.9 (52.3–57.7) | 59.8 | 130.0 (125.5–140.4) | 18.0 (17.6–18.0) | 0.52 (0.00–0.58) | 314 (1–316) | 3 |
| `large-compare-stepped` | 118.6 (116.2–124.1) | 58.8 (56.4–64.2) | 59.8 | 129.3 (124.1–134.4) | 19.8 (19.8–19.8) | 0.57 (0.54–0.63) | 317 (315–320) | 3 |
| `idle` | 116.3 (115.2–122.5) | 56.5 (55.4–62.7) | 59.8 | 126.2 (123.2–130.8) | 19.8 (19.0–21.1) | 0.00 (0.00–1.04) | 1 (1–167) | 3 |
| `handoff-to-f01` | 117.2 (117.0–117.5) | 57.4 (57.1–57.7) | 59.8 | 125.8 (124.9–126.7) | 20.7 (20.2–21.2) | 0.00 (0.00–0.00) | 9 (1–16) | 2 |

Every run held three frame buffers, so IOSurface is 59.8 MB throughout and the
differences are in footprint − IOSurface. One `handoff-to-f01` run is missing:
its window drew a single frame buffer on all three attempts, most likely
because the display slept or the window was covered.

CPU and context switches split into two groups across runs of the same
scenario: about 0.5% CPU with 300 switches a second, or close to zero with about
one. Which group a run lands in is not yet explained (the window being key and
the display link running are the suspects in #69 and #76), so compare CPU only
between runs that land in the same group, and read it from the raw `top.txt`.
