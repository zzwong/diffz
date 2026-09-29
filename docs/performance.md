# Performance

`scripts/profile-macos.sh` measures diffz's memory and CPU on macOS over a fixed
set of scenarios, so a change can be compared against a baseline. Each scenario
runs in a fresh process, several times, and the script writes raw samples and a
summary. It needs Xcode's command line tools (`footprint`, `vmmap`, `heap`) and
Python 3.

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
  and unlocked. Each window takes focus as it opens; a covered window or a
  sleeping display stops drawing, which changes both memory and CPU. A run that
  still has too few frame buffers after three attempts is kept and flagged.
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
