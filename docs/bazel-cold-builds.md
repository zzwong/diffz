# Cold build profiling and further trials

On this Linux workstation, Cargo remains the faster route for uncached compilation.
The useful improvement is compiling build-time tools at O0 while retaining the
existing runtime optimization/debug settings. The opt-in `fast-dev` profile took
**87.93 s versus 109.34 s** for stock Cargo at 32 jobs (**19.6% faster**).
The default Cargo profile, Make targets, lockfile, application code, and release
profile are unchanged by this round.

## Matched workload and controls

Measured 2026-10-19, same checkout/revision and host as the
[earlier trials](bazel-tuning.md): Ryzen AI MAX+ 395, 16 cores / 32 hardware
threads, Rust 1.98.1, Bazel 8.4.2, rules_rust 0.74.0 with the existing compatibility
patches. Every build includes the desktop binary, examples, and all unit/integration
test executables, with syntax, Gleam and Wasm enabled.

These are **artifact-cold compilations**, with dependency downloads already
available, a warm Bazel JVM, and ordinary warm OS file caches. Build outputs and
local action records are cleaned before each sample. Remote/disk action caches,
Kache and Rust compiler wrappers are disabled. Native compilers are explicitly
`/usr/bin/gcc` and `/usr/bin/g++`, with ccache disabled. Jobs/CPU scheduler settings
are shown separately; compiler-internal threading remains tool-specific.
Cargo retains its normal workspace incremental setting, starting with no state.
The existing Kache-backed Make workflow is outside this comparison.

An initial 126.10-second Cargo screen accidentally inherited `/usr/lib64/ccache/cc`;
its SQLite compilation hit a native object cache. It is excluded. The corrected,
verbose flag-audit run was 139.14 s. The table's stock 16-job row uses the ordinary
non-verbose repeat. The prototype development-manifest rewrite took 108.92 s at
16 jobs; the packaged optional profile has its own repeated measurements below.

## Measurements

Medians where repeated; individual screening runs otherwise. Small differences
between one-off configurations should not be treated as established gains.

| Configuration | Samples | Median |
| --- | ---: | ---: |
| Cargo stock, 16 jobs | 1 | 132.44 s |
| Cargo fast-dev, 16 jobs | 2 | 102.31 s |
| Cargo stock, 32 jobs | 2 | 109.34 s |
| Cargo fast-dev, 32 jobs | 2 | 87.93 s |
| Bazel pipeline/native8, 16 jobs | 1 | 155.68 s |
| Bazel pipeline/native8, 32 jobs | 1 | 157.91 s |
| Bazel no pipeline/native8, 32 jobs | 1 | 171.50 s |
| Bazel local/no pipeline/native8, 32 jobs | 1 | 170.16 s |
| Bazel selective pipeline/native8, 32 jobs | 1 | 154.22 s |
| Bazel pipeline/native2, 16 jobs | 1 | 165.03 s |
| Bazel pipeline/native2/buildtools0, 16 jobs | 2 | 134.18 s |
| Bazel pipeline/native2/buildtools0, 32 jobs | 1 | 129.17 s |
| Bazel internal helpers only O0, 32 jobs | 1 | 149.88 s |
| Bazel shared jobserver/native2, 16 jobs | 1 | 158.55 s |
| Bazel shared jobserver/native2/buildtools0, 16 jobs | 1 | 131.27 s |

`native2`/`native8` set AWS-LC's native parallelism and reserve that many CPUs.
`buildtools0` changes Rust compilation of exec tools, including build scripts,
procedural macros and Bazel's own helpers. Native C/C++ optimization remains O2.
Workspace codegen units remain Cargo's normal incremental defaults (256) versus
16 in the released Bazel trial. Existing static icon embedding in Bazel also
remains as documented in the earlier report.

## What the profiles showed

Cargo's HTML timings separate frontend work from code generation. In the strict
16-job baseline, Cranelift took 72.91 s, AWS-LC 65.61 s, GPUI 33.14 s and SQLite
26.94 s. These overlap; their durations cannot be added to estimate wall time.
The late GPUI component compilation took 26.42 s and lay close to completion.

Bazel's baseline at 16 jobs spent a summed 1,725 action-seconds; the 32-job run
spent 3,040 action-seconds and finished slightly later. Those sums are elapsed
spans across concurrent actions, **not CPU time**. Extra concurrency lengthened
individual actions under contention. Moving AWS-LC from eight CPUs to two did
not improve completion. Removing sandboxing changed the paired non-pipelined
runs by only about a second.

After native parallelism, the remaining critical paths run through combinations
of image/GPUI and Naga/WGPU/platform dependencies, then GPUI base/component and
the application's UI. Bazel pipelining shortens dependency waits but performs
separate metadata invocations. Disabling pipelining for eight substantial
Wasm-related crates reduced duplicate work modestly and shifted the critical
path to Wasm; it did not establish an advantage over Cargo.

Build tools were a more useful target. Cargo's wildcard dependency O2 override
also applies to build scripts, proc macros and their dependencies, taking
precedence over `build-override`. Lowering those tools to O0 shortened both
Cargo and Bazel builds. A separate Bazel patch lowering only its internal Cargo
manifest helper dependency graph also worked, but its smaller gain does not
justify retaining a 68-file generated patch in the normal configuration.

A local FIFO jobserver prototype coordinated rustc/LLVM threads across Bazel
actions. Instrumentation confirmed compiler token borrowing, and all 16 tokens
were returned after builds/tests. Metadata children that are killed early were
kept out of token borrowing. The wrapper's own tests and all project tests passed.
It added little beyond `buildtools0`; it remains evidence-only, with no new
jobserver dependency or prototype patch added to the normal build.

## Using the optional Cargo profile

```sh
cargo build --profile fast-dev --locked -p diffz
cargo test --profile fast-dev --locked \
  --workspace --all-features --lib --bins --tests
```

The profile is defined in `.cargo/config.toml` and is selected explicitly with
`--profile fast-dev`. Ordinary Cargo/Make commands retain the existing dev profile.
It keeps application/dependency optimization levels, workspace and GPUI debug
information, assertions, overflow checks, unwind behavior and effective codegen
units. A Cargo unit-graph audit compared **598 runtime units** and found identical
runtime dependency features and effective profile settings. Build-time units
change optimization level, and some shared host/runtime crates split into two
units. The audit alone uses Cargo's unstable unit-graph inspection interface;
ordinary builds/tests use the pinned stable toolchain without bootstrap flags.

The delivered config is byte-identical to the explicitly loaded config used in
the cold samples; standard Cargo discovery also makes the profile available to
Clippy. A follow-up audit found all **978 default-profile units unchanged**.

The profile inherits `release` solely to avoid inheriting the dev wildcard O2
override, then explicitly restores the development settings. Keep its workspace
and local-path dependency entries in sync when adding crates or changing profiles.
It has a separate `target/fast-dev/` artifact directory; use the profile consistently
to benefit from its warm artifacts. Normal Cargo/Make behavior remains available.

For Bazel's simpler opt-in improvement:

```sh
scripts/bazel-trial.sh build //:all_build \
  --config=pipeline --config=native2 --config=buildtools0
scripts/bazel-trial.sh test //:tests \
  --config=pipeline --config=native2 --config=buildtools0
scripts/bazel-trial.sh shutdown
```

## Repeating artifact-cold comparisons

```sh
python3 scripts/benchmark-cold-builds.py --jobs 32 --iterations 3 \
  --bazel-config pipeline --bazel-config native8 \
  --output "$HOME/.local/state/diffz/evidence/cold-new"

python3 scripts/benchmark-cold-builds.py --tool cargo --jobs 32 --iterations 3 \
  --cargo-profile fast-dev \
  --output "$HOME/.local/state/diffz/evidence/cold-fast-new"
```

The runner owns fresh directories under the checkout's `target/`, records setup
separately, alternates tool order when comparing both, preserves Cargo HTML and
Bazel action profiles, runs tests, disables native/compiler/action caches, and
shuts down its Bazel server. An optional `--repository-cache` shares downloaded
source archives only. Build output is intentionally retained; temporary scratch
is removed automatically.

## Warm iteration and verification

Three real value changes per row, medians including compilation and all tests.
Both tools use 32 jobs and their own warmed incremental artifacts.

| Change | Stock Cargo | fast-dev |
| --- | ---: | ---: |
| core edit plus full tests | 5.32 s | 5.29 s |
| ui edit plus full tests | 4.42 s | 4.65 s |
| app edit plus full tests | 3.66 s | 3.68 s |

Final checks: identical **413 passed / four ignored / 30 executables** by test
name across stock Cargo, fast-dev and the final Bazel build; fast-dev all-target
Clippy with warnings denied and doctests pass. The three binaries report the same
version and emit identical parsed fixture-inspection JSON; debug and unwind
sections remain present. Rust source and original manifests/lockfiles are restored.
Interactive GUI behavior, release/LTO, macOS and remote execution were not measured.

## Research from other projects

[Roman Kashitsyn's Internet Computer migration account](https://mmapped.blog/posts/17-scaling-rust-builds-with-bazel)
describes the value of mixed build graphs, shared CI/developer workflows, remote
caching and distributed execution. It supports those architectural benefits;
it does not supply a same-host uncached compilation win for this repository.
Its explicit test-name parity check is also used in these trials.

[Cargo's profile documentation](https://doc.rust-lang.org/cargo/reference/profiles.html#overrides)
explains why adding a build override alone does not defeat the wildcard O2 setting,
and why splitting host/runtime units can increase initial work.
The [official build-performance guide](https://doc.rust-lang.org/cargo/guide/build-performance.html)
identifies compiler/debug/backend/feature tradeoffs. This trial preserves the
current runtime backend/features and debug settings.
The Rust team's [parallel-compiler explanation](https://blog.rust-lang.org/2023/11/09/parallel-rustc/)
explains shared jobserver coordination. Its historical speedup figures are not
used as measurements of this compiler or project.

The [newer rules_rust pipelining proposal](https://github.com/bazelbuild/rules_rust/issues/3977)
uses unstable no-codegen/bootstrap support and is absent from the pinned release.
It was researched, not silently enabled or claimed as a tested speedup.
Distributed compilation and compiler-backend changes remain separate experiments;
the current data does not justify switching the default build tool to Bazel.

Evidence: `~/.local/state/diffz/evidence/bazel-cold-2026-10-19/` contains all raw
commands, exits, timing records, profiles, compiler graph audits, prototype patches,
failed setup exclusions and final verification. Reports from the first two rounds
remain in their original evidence directories.

## Retained files and cleanup

Task scratch was removed and task Bazel servers/processes are stopped. The dirty
trial checkout is retained at `/home/aaron/.local/share/codex/worktrees/diffz/bazel-trial`. Existing caches from earlier rounds
are retained. Redundant outputs from this round were deleted; the three reusable
benchmark output roots retained are:

- `/home/aaron/.local/share/codex/worktrees/diffz/bazel-trial/target/cold-build-7xczhhe_`
- `/home/aaron/.local/share/codex/worktrees/diffz/bazel-trial/target/cold-build-bhzg3p6q`
- `/home/aaron/.local/share/codex/worktrees/diffz/bazel-trial/target/cold-build-hdwnd0as`

The optional Cargo profile, Bazel presets, benchmark runner and reports remain
in the primary and trial checkouts. `completion.json` in the evidence directory
records cleanup and original-source hash checks.

## Controlled desktop and fresh-worktree trials

The runner also supports explicit Cargo workloads, real edit probes, paired
profile comparisons, and a separate paired nightly fingerprint experiment. These
extend the benchmark controls from [#124](https://github.com/zzwong/diffz/issues/124)
and do not constitute new performance results.
Current Linux/macOS measurements and any decision to promote a profile or cache
remain tracked in [#115](https://github.com/zzwong/diffz/issues/115).

Run the preflight first. It checks executable availability, installed Rust
toolchains, the Git revision, and configuration consistency without downloading
dependencies, installing toolchains, or bootstrapping Bazel:

```sh
python3 scripts/benchmark-cold-builds.py --tool cargo --preflight
```

The default workload remains `--workspace --all-features --all-targets`, including
the desktop and test graph. `--cargo-workload desktop` instead builds the native
application with `-p diffz --bin diffz` and its default features. A bare
`cargo build` selects only the core/adapters default members, so it is not a
desktop benchmark. Desktop and reuse modes require `--tool cargo`; the existing
matched Cargo/Bazel cold comparison remains available with the default workload.

### Paired Cargo profile comparison

Compare the existing `dev` and `fast-dev` profiles in one invocation. Each
iteration builds both profiles from empty compiled-output directories, reversing
their order on alternating iterations: `dev`, `fast-dev`, then `fast-dev`, `dev`.
Both variants use the same committed source worktree and have independent target
and build directories. Requested no-op, edit/undo, and fresh-worktree scenarios
run for each profile. Three iterations therefore include six cold builds, the
requested follow-up builds, and final all-feature workspace tests for both
profiles. These commands are opt-in local trials, not additional CI jobs.

```sh
python3 scripts/benchmark-cold-builds.py --tool cargo --preflight --jobs 4 \
  --cargo-workload desktop --cargo-profile dev --cargo-compare-profile fast-dev \
  --cargo-incremental on --cargo-scenario noop --cargo-scenario edit

python3 scripts/benchmark-cold-builds.py --tool cargo --jobs 4 --iterations 3 \
  --cargo-workload desktop --cargo-profile dev --cargo-compare-profile fast-dev \
  --cargo-incremental on --cargo-scenario noop --cargo-scenario edit \
  --output "$HOME/.local/state/diffz/evidence/desktop-profile-pair-new"
```

Keep revision, toolchain, jobs, native compilers, flags, features and host the
same, and choose a job count supported by the machine's memory allocation. Unset
unexpected profile or Rust flag environment overrides before a trial; the
allowlisted values are recorded in setup. The runner retains its owned build
artifacts for inspection, including both profile targets and any fresh-worktree
copies, so provision enough disk for that state. The script does not modify
profile defaults. Native compilers default to `/usr/bin/gcc` and `/usr/bin/g++` on
Linux, and Apple's `/usr/bin/clang` and `/usr/bin/clang++` on macOS; `--cc` and
`--cxx` select explicit alternatives.

`--cargo-incremental on` remains the runner's default. It explicitly sets
`CARGO_INCREMENTAL=1`, including when the calling shell set a different value.
Use `--cargo-incremental off` for a separately named non-incremental experiment;
that sets `CARGO_INCREMENTAL=0` for every variant and its verification. This
option also applies to single-profile trials. Cargo's [incremental override and
codegen-unit defaults](https://doc.rust-lang.org/cargo/reference/profiles.html#incremental)
mean an incremental-off `dev`/`fast-dev` comparison can also change codegen-unit
counts: this measures the complete profiles, not only their host-tool
optimization difference. Normal incremental edit latency requires its own
incremental-on observations.

Profile pairs require `--tool cargo`, two distinct profile names, the default
fingerprint mode, and no arbitrary `--cargo-arg` overrides. Evaluate fingerprint
changes separately on one fixed profile. Single-profile commands retain their
existing labels and CLI defaults. Profile pairs use `cargo-profile-1` and
`cargo-profile-2` labels; setup maps these labels to the exact profile names.
Final verification explicitly selects each compared profile, including `dev`.
Legacy single-profile `dev` verification keeps Cargo's default `test` profile,
which inherits `dev`; the actual selected verification profile is recorded.

### Matched nightly fingerprint comparison

[Cargo's content fingerprint experiment](https://doc.rust-lang.org/nightly/cargo/reference/unstable.html#checksum-freshness)
requires both `-Zchecksum-freshness` and `build.fingerprint="content"`. Using only
the flag does not select the current content mode. Install one dated nightly
separately, then use that exact toolchain for both sides of the comparison:

```sh
rustup toolchain install nightly-2026-10-19 --profile minimal

python3 scripts/benchmark-cold-builds.py --tool cargo --preflight \
  --cargo-toolchain nightly-2026-10-19 --cargo-fingerprint both \
  --cargo-workload desktop --cargo-profile fast-dev \
  --cargo-scenario noop --cargo-scenario edit --cargo-scenario fresh-worktree

python3 scripts/benchmark-cold-builds.py --tool cargo --jobs 32 --iterations 3 \
  --cargo-toolchain nightly-2026-10-19 --cargo-fingerprint both \
  --cargo-workload desktop --cargo-profile fast-dev \
  --cargo-scenario noop --cargo-scenario edit --cargo-scenario fresh-worktree \
  --output "$HOME/.local/state/diffz/evidence/desktop-fingerprints-new"
```

The runner requires a dated nightly from 2026-09-02 onward, checks that both
Cargo and rustc report nightly, and uses the resolved rustc from that toolchain.
It explicitly sets `mtime` and `content` in separate owned output directories,
reversing their order on alternating iterations. Comparing stable `mtime` against
nightly `content` would mix compiler changes with fingerprint changes. Arbitrary
`--cargo-arg` overrides and `RUSTC`/`RUSTC_BOOTSTRAP` overrides are rejected for
fingerprint trials. Profile experiments should use tracked configuration and
`--cargo-profile`.

### Scenarios, evidence and interpretation

Every iteration starts with an artifact-cold build. Additional phases are:

| Phase | What it measures |
| --- | --- |
| `noop` | Repeating the identical build with warm artifacts. |
| `edit-core`, `edit-ui`, `edit-app` | Real value changes to the existing core limit, UI scale, and application version probes. Each iteration uses a new value. |
| `undo-*` | Rebuilding after restoring the original source bytes; recorded separately from the edit. |
| `fresh-*-restore` | Copying that mode's seeded Cargo artifacts to a new owned output directory, preserving artifact timestamps. |
| `fresh-*` | Building an actual new Git worktree of the same commit using the copied artifacts. Git's new source timestamps are left untouched. |
| `verification` | Full workspace, all-feature tests after the build scenarios, using the selected profile/toolchain. |

Fingerprint, reuse and profile-comparison scenarios require a clean, committed
checkout. They create disposable source worktrees, keep the caller's source bytes and timestamps intact,
and check that the worktrees inherit the same Cargo configuration. Semantic edits
use the existing probe/restore approach, including the UI assertion's scale ratio.
Only an edit's own bytes are restored; concurrent changes cause failure and leave
the affected worktree for inspection. Failed or partially created worktrees are
removed with non-forced Git commands. A cleanup failure produces a failing exit
and `completed: false`, while an earlier compilation failure remains the primary
error.

`setup.json`, `measurements.jsonl`, and `completion.json` carry `schema_version`.
Setup records the revision, workload, profiles, explicit incremental setting,
toolchain versions, relevant build flags, source/configuration hashes and owned
paths. `cargo_variants` maps each output label to its profile and fingerprint
mode. Each phase records its exact command or copy operation, working directory,
output paths, exit code and duration; Cargo observations also record
`cargo_profile`, `cargo_incremental` (`"1"` or `"0"`), `scenario`, and `iteration`.
Final verification has no iteration number. These are additive schema-v1 fields;
the original `cargo_profile` setup field still names the baseline profile.
Raw logs and each build's Cargo HTML timing report are retained. Aggregate medians
by matching scenario, profile, incremental setting and fingerprint mode, keeping
the ordered individual samples. Do not combine cold, no-op, edits or undo builds
merely because they all have `phase: "build"`. The total runner duration also
includes setup, artifact copying, undo builds and final verification. In particular, final verification
expands the desktop graph to all-feature workspace tests and is not part of the
desktop build measurement.

Content fingerprints are still experimental. Build-script `rerun-if-changed`
inputs still use timestamps, and native/generated inputs or absolute paths can
still cause cross-worktree rebuilds. The local copy scenario excludes archive
compression and network transfer; use `benchmark-shared-cache.py` for those costs.
Retain the logs and investigate unexpectedly missing recompilation before
trusting a large gain. Validate the chosen configuration on the actual Linux and
macOS hardware, including test results and an interactive desktop smoke check,
before changing normal developer or CI defaults.

The harness regression suite uses real temporary Git worktrees with explicitly
fake compiler executables. It checks isolation, controls, restoration and failure
handling; its timing records are not Rust performance measurements:

```sh
python3 -m unittest discover -s scripts/tests -p test_benchmark_cold_builds.py -v
```

