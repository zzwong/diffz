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
