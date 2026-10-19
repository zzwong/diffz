# Further Bazel trials

For the later uncached compilation experiments, native-cache controls, and
optional Cargo build-tool profile, see [cold-build profiling](bazel-cold-builds.md).

Measured on 2026-10-19 in the existing isolated `bazel-trial` checkout and the
primary checkout. Same Linux host, pinned Rust 1.98.1, Bazel 8.4.2, and
rules_rust 0.74.0 as the [first comparison](build-performance.md). No Cargo
manifest, Cargo lockfile, default Make target, or CI workflow changed.

The useful result is narrower than a general migration: released Bazel rules
help app/test iteration through test-result reuse; experimental incremental Rust
workers also improve the measured core/UI edit-and-test cycles. Artifact-cold
builds still show no convincing Bazel advantage. Keep Cargo as the default and
use the worker driver as an opt-in trial while its upstream patch is a draft.

## Actual edit-and-test cycles

Medians of three real source changes per row. Each command includes compilation
and tests. Cargo uses its normal incremental development profile; Nextest uses
the same Cargo artifacts. Bazel reuses unaffected test results. Both build
schedulers use 16 jobs; Nextest runs up to 16 test processes. Compiler/test
internal threading remains tool-specific. Kache and compiler wrappers are off.

| Change | Cargo test | Cargo + Nextest | Released rules + pipelining | Draft workers |
| --- | ---: | ---: | ---: | ---: |
| Core parser default | 5.24 s | 4.54 s | 7.41 s | 4.13 s |
| UI debug environment name | 4.43 s | 3.72 s | 4.86 s | 2.77 s |
| App version text and expected value | 3.65 s | 2.96 s | 1.82 s | 1.74 s |
| Retention assertion message | 2.92 s | 2.26 s | 0.83 s | 0.85 s |

The core probe changes the default file limit, the UI probe changes a debug
configuration variable's name, the app probe changes version output and its
expected value, and the test probe changes an assertion message. Each is
restored before the next sample. These are actual value changes, not comments.
The final two columns include the Wasmtime fix described below. Workers use
256 workspace codegen units and eight native AWS-LC jobs; released rules use
16 workspace codegen units. Both keep the repository's optimization/debug
levels and the trial's static icon embedding. The existing Kache-backed Make
workflow was not benchmarked here.

Worker gains are substantial against stock Bazel's compiler behavior, but
most app/test savings already come from selective test execution. On these
samples, workers bring core iteration close to Cargo + Nextest and improve UI
iteration further. They do not materially improve the tiny assertion example
beyond normal Bazel caching. Shared-desktop samples are not confidence intervals.

## Cold builds and native parallelism

Single artifact-cold screening runs, with downloaded dependencies/toolchains and
a warm server. These precede the final Wasmtime patch and should not be used to
rank small differences. The worker row is the initial no-pipelining prototype.

| Bazel configuration | Full desktop, examples, and test binaries |
| --- | ---: |
| Baseline with LLD | 153.59 s |
| Pipelining, core temporarily excluded | 160.57 s |
| Eight AWS-LC jobs, CPU reservation | 160.35 s |
| Native parallelism plus pipelining | 155.45 s |
| Draft workers, 256 workspace codegen units | 162.92 s |

The first comparison's normal Cargo artifact-cold build took 128.36 s. These
trials did not reverse that result. Increasing workspace codegen units to 256
without incremental state also produced no convincing edit-cycle improvement.

The action profiles explain why isolated improvements did not shorten the whole
build: AWS-LC fell from **85.05 s to 20.68 s** with eight jobs, while the remaining
critical work included SQLite and the GPUI chain. Native build scripts in the
pinned rules default to `NUM_JOBS=1`. The trial supplies a selectable override and
[a small reservation patch](../bazel/build-script-resources.patch), so requesting
eight native jobs also reserves eight CPUs. Aquery confirms `NUM_JOBS=8` and
`cpu:8` for the measured configurations; the overall scheduler budget is 16.

## Pipelining correctness and deterministic bindings

Enabling [Rust pipelining](https://bazelbuild.github.io/rules_rust/rust_settings.html#pipelined_compilation)
initially broke adapter integration builds with E0463. Compiler tracing showed
that core's metadata and full library had different crate hashes. Temporarily
excluding core made the suite pass, establishing a narrow working boundary.

Wasmtime's binding macro emitted dummy `include_bytes!` expressions containing
canonical absolute WIT paths. Separate Bazel sandboxes therefore produced
different expanded source. The [Bazel-only macro patch](../bazel/wasmtime-bindgen-bazel-inputs.patch)
removes those redundant dependency-tracking expressions; core's WIT files remain
explicit `compile_data` inputs. Cargo continues to use the original dependency.
Core no longer needs a pipelining exception.

Three WIT comment changes each triggered compilation and produced the same
byte-for-byte core library. Deliberately invalid WIT failed compilation, proving
that input changes are still tracked. Restoring it passed all tests. The binding
API and project Rust source are unchanged.

## Incremental workers

The [upstream worker proposal](https://github.com/bazelbuild/rules_rust/pull/4184)
is still a draft. This trial pins commit
`9dfe42a282847d3188b404d9f1081ddf403477b7` and ports its argument handling to
0.74.0. Its own process-wrapper unit tests and the project suite pass. Aquery
confirms worker support, the incremental flag, 256 workspace codegen units,
and state under the checkout's `target/`.

The packaged driver verifies the official source archive and applies the checked-in
patch. It uses a separate server/output root and convenience symlinks, and disables
experimental-module lockfile writes. Normal Bazel and Cargo remain usable.
Worker state belongs to live worker processes, so restarts may require seeding it
again. Combining the draft workers with pipelining failed on a read-only metadata
output; the worker driver therefore leaves pipelining off. It does not establish
remote execution or shared incremental-state support.

```sh
scripts/bazel-worker-trial.sh build //:all_build
scripts/bazel-worker-trial.sh test //:tests
scripts/bazel-worker-trial.sh test //:tests --cache_test_results=no
scripts/bazel-worker-trial.sh shutdown
```

## Action-cache reuse across checkouts

A clean build with action-cache population cost **154.17 s**.
After `bazel clean`, restoring the full build from that cache cost
**2.54 s**, followed by a successful uncached test run.
This measures restoration of known artifacts, not compilation of new code.

For a stronger test, the primary checkout changed the core file limit to 33,033,
a source state never built before. Cargo reused the existing trial target
directory; Bazel shared repository-download and action caches on the same host.

| Operation in the other checkout | Cargo | Bazel |
| --- | ---: | ---: |
| Dependency/setup command | 0.30 s | 18.12 s |
| Build with the unseen core edit | 7.57 s | 9.68 s |
| Restore original source and build | 2.96 s | 1.07 s |

Bazel reused 2,685 actions and executed 36 sandboxed actions for the fresh edit.
The populated action cache occupied 5.7 GiB on disk.
Its cache worked across roots, but Cargo still won that new-source build, even
before including Bazel's setup. Replaying the original state favored Bazel.
Both tools passed tests on the changed and restored source. This is local,
same-host evidence; cache transfer costs, remote services, container images,
and native-system-library portability were not measured.

## Test runner and final checks

Five warm full-suite commands took a median **2.62 s** with
Cargo and **1.91 s** with Nextest. Nextest's
[parallel process model](https://nexte.st/docs/design/how-it-works/)
provides a smaller test-speed improvement without changing build tools. It does
not provide Bazel's unaffected-result caching. Nextest was downloaded temporarily
for these measurements, not globally installed.

Cargo, Nextest, released Bazel rules, and the draft-worker build have identical
named results: **413 passed, four ignored, across 30 executables**. The minimal,
syntax, and Wasm check matrix passes; all-feature Clippy with warnings denied,
Cargo doctests, formatting, native builds, and script syntax checks also pass.
Cargo, normal Bazel, worker Bazel, and the other-checkout build also report the
same version and produce identical parsed `--inspect --fixture F01` JSON.
Bazel's suite still covers unit/integration executables rather than doctests.
Interactive GUI checks, release/LTO, macOS, and a production CI migration remain
outside this trial.

## Repeating the comparison

```sh
# Released rules, with optional measured configurations:
scripts/bazel-trial.sh test //:tests --config=pipeline --config=native8
scripts/bazel-trial.sh shutdown

# Full benchmark protocol with incremental workers:
python3 scripts/benchmark-bazel.py --iterations 3 \
  --bazel-driver scripts/bazel-worker-trial.sh \
  --output "$HOME/.local/state/diffz/evidence/bazel-workers-new-comparison"
```

The benchmark also accepts repeated `--bazel-config` values for the normal
trial. Read [the worker notes](../bazel/experimental/README.md) before comparing
it with released rules. Configuration switches can require substantial rebuilding;
keep one mode active during an edit loop.

Evidence: `~/.local/state/diffz/evidence/bazel-trial-2-2026-10-19/` contains
raw command/exit/timing records, profiles, source snapshots, compiler/CPU audits,
matching named-test proof, WIT positive/negative controls, and binary smoke checks.
`measurement-notes.md` identifies failed setup attempts and comparison limits.
The earlier evidence remains in `bazel-trial-2026-10-19/`.

The retained trial checkout is
`~/.local/share/codex/worktrees/diffz/bazel-trial/`. Source changes, generated
worker-rule sources, and reusable build/action caches remain under the two
checkouts and their `target/` directories. Per-process worker state and task
scratch were removed after verification; all task Bazel servers are stopped.
