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
scenario takes about an hour; the idle scenario alone waits four minutes a run.

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

Each run starts `diffz --foreground --state-dir /tmp/dzp/<scenario>` with an empty
state directory, and sets `DIFFZ_PROFILE_STEPS`:

- **Frame buffers first.** With the variable set, the window redraws on every
  frame for about two seconds after it opens, so GPUI's pool of frame buffers
  grows to its maximum of three on every run. A run that ends up with fewer,
  usually because the window was covered while it opened, is retried.
- **Stepping.** After the first source installs, diffz selects the next file
  N times, 300 ms apart, through the same path as `]`, and writes
  `diffz-profile stepped <count>` to stderr. The script waits for that line, and
  the summary flags a run that stepped short. A later install in the same
  process, such as the hand-off, does not step. Without the variable, none of
  this runs.
- **Settling.** The script waits 8 s, then until the SQLite WAL in the state
  directory has not changed and `phys_footprint` has stayed within 1 MB for 6 s.
  Releases and blame load after a compare shows, so memory keeps moving for a
  while after the window looks finished. A run that has not settled after 180 s
  is sampled anyway and flagged.
- **Sampling,** in this order, because the later tools briefly suspend the
  process: `footprint`, then 31 one-second samples of `top` (the first has no
  interval and is dropped), `powermetrics --samplers tasks` alongside when
  `sudo -n` works, then `ps`, `vmmap --summary` and `heap -s`.
- **Repeats.** `--repeat N` (default 3) runs every scenario once per round,
  round after round, so slow drift in the machine spreads across scenarios. The
  summary gives the median with the minimum and maximum.

The window opens at 1360×900 points on the main display. The script checks that
IOSurface is a whole number of frame buffers of that size at the display's
scale, which catches a window that opened at another size, and records how many
there were.

### Quiet machine

The script refuses to run unless the memory pressure level
(`kern.memorystatus_vm_pressure_level`) is 1, at least 25% of memory is free
(`memory_pressure -Q`), the one-minute load is at most a third of the CPU count,
and no `cargo`, `rustc` or `clippy-driver` is running. `--wait SECONDS` polls
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

- Do not use or cover the diffz window during a run. Each window takes focus as
  it opens; a covered window stops drawing, which changes both memory and CPU.
- The window opens on the main display. Record runs that are compared on the
  same display arrangement; an external monitor at another scale changes every
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
3. Paste the `--compare` table into the PR body. A change counts when the
   targeted metric moves beyond the old run's min–max spread (the table marks a
   change inside it with ≈), and no other scenario gets worse by more than its
   spread.

## Baseline

BASELINE_PLACEHOLDER
