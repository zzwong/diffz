# Bazel trial and build performance

Measured on 2026-10-19, starting at `fe0581b`, on x86_64 Linux with a Ryzen AI
MAX+ 395 (16 cores / 32 threads). Rust is 1.98.1, Bazel 8.4.2, and rules_rust
0.74.0. The trial builds the desktop executable, all four crates, all 30 unit
and integration-test executables, and the `store_size` example using native
`rules_rust` targets. It includes syntax highlighting, Gleam, Wasm, GPUI test
support, and the repository's pinned forks and vendored Linux patch.

Keep Cargo as the default build workflow and retain Bazel as an opt-in trial.
Cargo's normal incremental profile wins the measured fresh-source builds.
Bazel improves the measured app/test edit-and-test cycles through parallel test
execution and reuse of unaffected results. Its local action cache also helps
when revisiting a previously built source state.

These are the first-pass measurements. The [follow-up trials](bazel-tuning.md)
test native parallelism, pipelining, Nextest, incremental Rust workers, and
artifact reuse across checkouts. They also fix sandbox-dependent Wasmtime crate
hashes; the worker mode remains a separate upstream-draft experiment.

## Build and test timings

Wall-clock medians of three runs, except artifact-cold builds (one run each).
Dependencies and toolchains are already downloaded. Cold builds start with empty
compiled-artifact directories; warm Bazel runs reuse its server. Both schedulers
use 16 jobs. Kache and Rust compiler wrappers are disabled throughout.

| Operation | Cargo normal dev profile | Cargo controlled profile | Bazel with LLD |
| --- | ---: | ---: | ---: |
| Artifact-cold build | 128.36 s | — | 161.63 s |
| Unchanged build | 0.35 s | 0.34 s | 0.38 s |
| App source edit, build | 1.28 s | 1.47 s | 1.81 s |
| Core source edit, build | 2.58 s | 6.08 s | 7.47 s |
| UI source edit, build | 2.07 s | 4.41 s | 4.53 s |
| Test source edit, build | 0.58 s | 0.66 s | 0.67 s |
| Execute all tests | 2.66 s | 2.62 s | 1.55 s |
| Reuse unchanged test results | — | — | 0.32 s |

The ordinary Cargo column uses the repository's existing development profile
with incremental compilation enabled. The controlled column disables
incremental compilation and uses 16 codegen units, matching Bazel. Optimization
and debug info match the manifest: workspace crates at optimization level 0 /
debug level 1, dependencies at optimization level 2 / debug level 0, and GPUI
exceptions at debug level 1. Overflow checks remain enabled. Compiler invocation
audits are retained with the measurements.

Bazel embeds the GPUI icon assets so its binaries can load them after the
compiler sandbox disappears. The controlled Cargo column applies the same
`debug-assertions=false` overrides to `gpui-kit-assets` and `rust-embed`. The
normal Cargo column retains its original development asset-loading behavior.
The controlled cold run predates the stale-socket fixture fix and is excluded
from the final cold comparison. Native build-script and compiler scheduling
remain tool-specific even with equal scheduler job counts.

The source-edit rows append a different comment to the named crate's source or
one integration test, then build all targets. Sources are restored after each
sample. These measure source invalidation, compilation and linking; actual code
changes can invalidate different Rust incremental queries. As an additional
check, changing the parser's file limit took 3.10 s with normal Cargo and
7.54 s with Bazel; changing the UI magnification cap took 2.78 s and 5.54 s
respectively. Those semantic build checks each have one sample and were also
restored before running tests.

These are local measurements on a shared desktop. The raw samples and compiler
settings matter more than small differences between subsecond invocations.

## Complete edit-and-test cycles

These include compilation and the test command. Cargo uses its normal
incremental profile. Bazel reuses results for unaffected tests.

| Actual source change followed by tests | Cargo | Bazel |
| --- | ---: | ---: |
| App version text and its expected test value | 3.62 s | 1.88 s |
| Retention-test assertion message | 2.95 s | 0.90 s |

These rows use three different real value changes, rather than comments. All
changed tests pass. For the assertion change, Bazel executed one of the 30
executables and reused the other 29 results. Full uncached runs were performed
before and after the experiments.

## Bazel improvements made during the trial

The initial Bazel configuration used the host GNU linker. App rebuilds took
5.65 s. Selecting the pinned Rust toolchain's LLD reduced them to 1.81 s.
The configured linker directory is a declared Rust sysroot input; this trial's
path assumes x86_64 Linux and Bazel's `fastbuild` layout. Cargo already uses
[LLD by default on this target](https://doc.rust-lang.org/rustc/codegen-options/index.html#linker-features).

The wrapper also keeps the test environment identical for build and test
analysis. Previously, switching commands discarded Bazel's analysis cache.
Unix socket tests receive a short, isolated temporary directory on disk, with
cleanup after every executable.

With an explicit local action cache, revisiting the same core edit took
0.78 s and restoring the original source took 0.79 s. Seeding the two source
states cost 7.77 s and 7.13 s first. This experiment measures reuse of known
states; the fresh-edit timings above use no disk action cache. No remote cache
or remote execution service was used.

## Correctness and scope

Cargo normal, Cargo controlled, and Bazel produced identical named test results:
**413 passed and four ignored across 30 executables**. The desktop binaries
report the same version and produce identical parsed JSON for
`--inspect --fixture F01`. Formatting and all-feature Clippy pass. The benchmark
runner's orchestration was also rehearsed with execution mocked: all source
probes restored their original bytes and Cargo profile routing was checked.

Repeated testing exposed two fixture problems:

- The retention test depended on setup completing within one wall-clock second.
  Saving blame refreshes recency; the test now assigns its intended ordering
  explicitly after setup. A forced-delay reproduction passed after that fix.
- A stale-listener fixture could remain alive in another test's forked child
  briefly, returning `Unanswered` instead of `NotRunning`. A controlled inherited
  descriptor reproduction confirmed that behavior. The fixture now creates its
  listener in a separate, completed process. Fifty complete adapter unit suites
  passed afterward: 2,050 test executions.

Fixture paths in the rename and compare tests now work with both Cargo's crate
working directory and Bazel runfiles. These changes preserve the existing test
assertions. Production handoff and retention behavior are unchanged.

The Bazel integration also needs explicit declarations for WIT files, manifests
read by GPUI's facade macros, icon inputs and the existing local GPUI Linux
patch. GPUI Component's pinned build script only forwards an icon directory;
Bazel supplies the equivalent environment with declared files instead of a path
into an expired build-script sandbox.

This trial covers the all-feature Linux development/test workload. The separate
minimal/syntax/Wasm check matrix, doctest integration, packaging, release/LTO,
interactive GUI verification, macOS, and remote-cache portability remain outside
its scope. The tests use host Git/Python/native libraries; their system contents
are not fully modeled as hermetic inputs. Use an uncached gate when validating
those dependencies. Cargo remains responsible for existing CI and publishing.

## Running the trial

Install the native build prerequisites from [Linux development](linux.md).
The wrapper downloads and verifies pinned Bazel into `target/bazel-bootstrap`
on first use; `BAZEL=/path/to/bazel` overrides that executable.

```sh
scripts/bazel-trial.sh build //:diffz
scripts/bazel-trial.sh build //:all_build
scripts/bazel-trial.sh test //:tests
scripts/bazel-trial.sh test //:tests --cache_test_results=no
scripts/bazel-trial.sh shutdown
```

Local action caching is opt-in:

```sh
scripts/bazel-trial.sh build //:all_build \
  --disk_cache="$PWD/target/bazel-action-cache"
```

For a new comparison, run in an otherwise idle checkout:

```sh
python3 scripts/benchmark-bazel.py --iterations 3 --disk-cache-trial \
  --output "$HOME/.local/state/diffz/evidence/bazel-new-comparison"
```

The runner refuses an existing evidence directory, clears only its own benchmark
Cargo directories, cleans Bazel outputs for the cold run, temporarily edits and
restores the probe files, and shuts down Bazel on exit. Build artifacts stay
under the checkout's `target/`. Bootstrap/test scratch uses task directories
beneath `$TMPDIR` (default `/var/tmp`) and is removed after use.

The Cargo manifests and `Cargo.lock` remain unchanged.
[Crate Universe](https://bazelbuild.github.io/rules_rust/crate_universe_bzlmod.html)
generates external dependencies from them. The explicit vendored Linux target
and profile annotations are pinned trial configuration and must be reviewed
when upgrading project metadata, toolchains or dependencies.

## Retained evidence

`~/.local/state/diffz/evidence/bazel-trial-2026-10-19/` contains
`summary.json`, every measured command and exit code in `measurements.jsonl`,
raw logs, Cargo HTML timings, Bazel profiles and compiler audits, matching-test
proof, binary smoke outputs, fixture reproductions and source/config snapshots.
The initial failed warm Cargo run is retained and excluded from the final
medians; `qualified-*` measurements follow the stale-socket fix. Final Bazel
warm invocations use the corrected shared test environment. Cold samples are
single runs, not confidence intervals.

The isolated trial checkout is at
`~/.local/share/codex/worktrees/diffz/bazel-trial/`. Its source changes and build
artifacts are intentionally retained. The task's Bazel server is stopped and
throwaway scratch is removed after verification.
