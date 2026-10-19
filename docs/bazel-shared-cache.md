# Shared Bazel action-cache trials

On this workstation, a shared cache makes fresh checkouts substantially faster
than restoring Cargo artifacts. It does not eliminate compilation of new source.
After dependencies are local, Cargo still wins core/UI edits; Bazel can make
small app edit/test cycles faster through its test runner.

The useful configuration changes are delivered: bounded opt-in local action
caching, a shared endpoint option, one `fast-dev` Bazel configuration, pinned
Rust archive checksums, reusable unpacked repositories, and a stable test scratch
path. See [setup](bazel-cache-setup.md). Cargo remains the default build/CI tool.

## Workload and controls

Measured 2026-10-19, same Linux workstation and Rust/Bazel versions as
[cold profiling](bazel-cold-builds.md): Ryzen AI MAX+ 395, 16 cores/32 threads,
128 GiB memory, Rust 1.98.1, Bazel 8.4.2, rules_rust 0.74.0. Compilation is
serialized between tools, with 32 jobs, explicit GNU CC/CXX, ccache disabled,
and Rust compiler wrappers disabled.

Both build graphs include the desktop binary, example, and all test executables.
The full suite contains 417 named statuses: 413 passed and four ignored, in
30 executables. Bazel's test wrappers remain necessary for Cargo-compatible
fixtures and short Unix socket paths.

Cargo uses the optional `fast-dev` profile; Bazel uses pipeline/native2/buildtools0,
now available as `--config=fast-dev`. These are the useful configurations from
the previous trials, not identical compiler settings: Cargo workspace crates
use CGU256 and incremental compilation; released Bazel rules use CGU16 without
rustc incremental state. The earlier static-icon-embedding difference also
remains. No release, macOS, GPU/UI-performance, or remote-execution claim follows.

Writer and consumer are distinct Git worktrees at `fe0581b`, with the existing
dirty source overlay copied into both. A 389-file SHA-256 manifest records the
initial matching trees. Cargo registries/Git dependencies and repository download
archives are already local. Fresh consumer output directories start empty;
Bazel disk caching is disabled during shared-cache measurements. Startup,
analysis, repository materialization, cache transfers, Cargo extraction and any
recompilation are included in consumer wall time.

This is **same-host evidence with bandwidth shaping**, not a measured WAN or
fresh machine. OS page caches are warm. The unpacked toolchain cache is warm
in the improved cases; a new runner must retain it, restore it, or use a prepared
image. Native compilers, system headers/libraries and CMake are not yet fully
hermetic, so cross-machine cache correctness still needs a pinned environment.

## Population and fresh checkout results

The seed cost was Cargo **90.86 s** plus **2.64 s** for tests and **3.37 s** to
create its zstd archive. Bazel compiled in **132.73 s**, then ran tests in
**2.56 s**, uploading about **5.07 GB** of action outputs. Cache population is
a separate cost; enabling a cache on a local no-op does not backfill all outputs.

For the controlled fresh-checkout comparison, each sample assigns all 389 source
files new timestamps, newer than the seeded Cargo archive, to model a Git
checkout. Bytes remain identical. Cargo consequently recompiles nine units:
the four workspace crates, vendored GPUI Linux, and its GPUI platform/base/
component/kit dependents. Both samples reproduce that list. An optimistic
timestamp-preserving control reused Cargo completely; it is not the final
fresh-Git comparison.

| Fresh build, including cache retrieval/setup | Samples | Wall time | Download |
| --- | ---: | ---: | ---: |
| Cargo archive, fresh Git timestamps | 2 | **53.67, 54.35 s** | 1,449 MB compressed archive each |
| Bazel HTTP, reusable unpacked repositories | 2 | **14.47, 14.44 s** | 328 MB artifact bodies each |
| Bazel gRPC, compression off, `toplevel` | 1 | **11.73 s** | 328 MB TCP payload |
| Bazel gRPC/zstd, `toplevel` | 1 | **10.28 s** | **71 MB TCP payload** |
| Bazel gRPC/zstd, `all` | 1 | **20.43 s** | 1,298 MB TCP payload |

All fresh Bazel builds report **2,721 remote action hits and zero compilation
misses**. The Cargo rows split into roughly 15 s download, 5.7–6.1 s extraction,
and 33.1 s rebuilding timestamp-invalidated crates. With preserved source
timestamps, the earlier loopback Cargo restore needed roughly 5–6 s total and
zero compilation; that different scenario should not be combined with fresh Git.

The HTTP endpoint caps aggregate GET bodies at 100 MiB/s and adds 5 ms per GET.
Its parallel calibration delivered 96.9 MiB/s. The standard gRPC server uses a
separate aggregate 100 MiB/s downlink TCP relay with loopback latency and no
artificial per-RPC delay. Uploads are unshaped. TCP counters include HTTP/2/gRPC
framing but exclude TCP/IP headers; HTTP counters count artifact bodies. Use the
paired gRPC compression-off/on cases to assess compression; the HTTP/gRPC timing
difference also includes protocol/latency changes. gRPC results are single
screening samples, not repeated estimates of a production service.

The gRPC service is **bazel-remote 2.6.3**, downloaded from its release and SHA-256
verified. Existing seed blobs were imported through its HTTP API in 5.78 s,
without compiling again. The service stores about 1.3 GiB with zstd and has a
10 GiB eviction limit. Its compressed transfer support and storage compression
are separate settings. [Server documentation](https://github.com/buchgr/bazel-remote/blob/v2.6.3/README.md),
[Bazel compression setting](https://github.com/bazelbuild/bazel/blob/8.4.2/src/main/java/com/google/devtools/build/lib/remote/options/RemoteOptions.java).

## Removing toolchain materialization from the warm path

Before this improvement, a fully cached fresh Bazel build still took
**25.47–29.99 s** on loopback. Its profile spent about **18 s** materializing the
Rust toolchain. Action caching does not cache that repository setup.

Bazel 8.4.2 has `--repo_contents_cache`, disabled by default. The Rust rule only
marks its toolchain reproducible when the supplied archive-checksum map matches
the resolved one. We pinned all five verified Rust archive hashes in
`MODULE.bazel`, enabled a shared unpacked-repository cache, and primed it in
**15.56 s**. Repeated fresh loopback builds fell to **10.2–10.3 s**, retaining
all 2,721 action hits. The compiler/toolchain contents did not change.

The 1.7 GiB repository cache resides in the dedicated cache-host worktree's
`target/repositories`, outside the worktrees performing builds. The pinned
rules also read `BAZEL_RUST_STDLIB_LINKFLAGS` without tracking it as a repository
input; the wrapper rejects it when this cache is enabled. It was unset in every
trial. This guard does not make the remaining native environment hermetic.

## Real source edits and download policy

Semantic probes change a core file limit, UI zoom cap (with its matching test
expectation), and app version text (with its matching test expectation).
Each iteration uses a new value. Consumer uploads are disabled, so these edits
cannot obtain compilation hits from another sample. Sources are restored after
every probe and after the complete run.

The first core edit after a `toplevel` HTTP restore downloaded **2.57 GB** of
intermediates and took **29.79 s**. Its 36 changed actions compiled locally;
third-party dependencies were downloaded, not recompiled. The second edit
downloaded no artifacts and took **7.04 s**. Cargo took **3.17 and 3.11 s** for
those same two edits after its restored target had warmed.

Compression and eager hydration make the tradeoff more practical:

| gRPC/zstd policy | Fresh build | First core edit | First-edit download | Second core edit |
| --- | ---: | ---: | ---: | ---: |
| `toplevel` | 10.28 s | 15.54 s | 728 MB | 7.02 s |
| `all` | 20.43 s | 7.31 s | 3 KB control traffic | 7.25 s |

`--config=cache-eager` selects `all`. It moves transfer work before editing;
it does not demonstrate a universal reduction in total checkout-plus-edit time.
Known-state undo builds in the disk-disabled experiment also download cached
outputs again. An everyday local disk tier avoids repeatedly fetching those
known outputs from the service.

With dependencies already local, the second unique edit/test cycle was:

| Edit | Cargo build + full tests | Bazel build + forced full tests |
| --- | ---: | ---: |
| Core | 3.11 + 2.57 = **5.68 s** | 7.04 + 1.42 = **8.45 s** |
| UI | 2.15 + 2.65 = **4.80 s** | 4.69 + 1.51 = **6.19 s** |
| App | 1.34 + 2.67 = **4.01 s** | 1.89 + 1.45 = **3.34 s** |

These individual warm samples describe the measured workflow, not a statistical
ranking of small differences. Cached tests are a separate option: the compressed
gRPC consumer reused all 30 test results in **1.22 s / 67 KB**. Forcing execution
after a lazy restore downloaded another 278 MB and took **3.50 s**; eager mode
had already downloaded its test binaries and ran them in **1.71 s**.

## Delivered setup and verification

- `DIFFZ_BAZEL_CACHE=1`: opt-in local disk cache, 10 GiB idle-GC target.
- `DIFFZ_BAZEL_REMOTE_CACHE`: shared endpoint, with a read-only option.
- `DIFFZ_BAZEL_REPO_CONTENTS_CACHE`: validated external location for unpacked repositories.
- `DIFFZ_BAZEL_TEST_TMP_ROOT`: stable short path, so per-job random temporary
  directories do not invalidate shared test results. Tests still create unique children.
- `--config=fast-dev`: one consistent measured build configuration.
- `--config=cache-eager`: pre-download intermediates for an active editing session.

The primary checkout's local cache is populated. A genuinely fresh local output
root restored **all 2,721 actions from disk in 12.47 s**, with the remote endpoint
disabled. The ordinary primary output root is also retained and warm; restarting
its server and building locally took 4.80 s. Final forced tests in that checkout
took 2.08 s and reported **413 passed / four ignored**. Both binaries report
`diffz 0.3.1` and produce identical parsed F01 inspection JSON, SHA-256
`43a765a22882e455ec756051df47b7e807c1ee2248ffd42bf614df78ca6c33f3`.
The documented `--config=fast-dev --config=cache-eager` command was also run
with no remote endpoint: **4.55 s**, 3,473 local action hits, no compilation.

The first UI benchmark probe omitted a hardcoded expectation update and failed.
That failed test/probe is excluded; the corrected UI/app/core trials all passed.
No application source, Cargo dependency, or default Cargo profile change remains
from this cache round. Root caches were not benchmarked through Kache.

## Repeating and evidence

The drivers are `scripts/benchmark-shared-cache.py` (instrumented HTTP, Cargo
archive restore, timestamp controls, semantic probes) and
`scripts/benchmark-grpc-cache.py` (official service, compression/download policy,
TCP-byte accounting, and local cache restoration). They stop the servers they
start. The lightweight HTTP helper is for benchmarking, not a public service.

The raw evidence is retained at
`/home/aaron/.local/state/diffz/evidence/shared-cache-2026-10-19/`:
the root contains seed/initial controls, `repo-contents/` contains the corrected
semantic trials and repository-cache improvement, `fresh-git/` contains the two
controlled Git-timestamp comparisons, and `grpc/` contains service import,
compression, eager-download, and disk-only verification. JSONL commands/timings,
request bytes, Bazel profiles, source hashes/snapshots, timestamp audits, runtime
outputs, and Pi review notes are retained there. Two Luna agents managed the
requested `opencode-go/deepseek-v4.1-flash` workers; a stalled final optional Pi
review was terminated after the completed design review and proxy smoke checks.

The next production step is a persistent authenticated service and a pinned
native image, with trusted CI writers and developer/PR readers. Share action
keys across commits rather than creating a new cache namespace per Git SHA;
separate incompatible native environments. No cloud service, credentials,
recurring job, or CI migration was configured by this local trial.

## Cleanup and retained state

All trial Bazel servers, HTTP/gRPC services, TCP relays and Pi workers were
stopped. Task scratch directories were removed. Nine redundant output roots
were deleted; the dirty trial worktrees were retained. Earlier-round caches
were left untouched.

Intentionally retained current-round artifacts (approximately 39 GiB total):

| Location | Purpose | Size |
| --- | --- | ---: |
| `/home/aaron/projects/diffz/target/bazel` | Ready primary output root | 5.9 GiB |
| `/home/aaron/projects/diffz/target/bazel-disk-cache` | Bounded local action cache | 4.8 GiB |
| `~/.local/share/codex/worktrees/diffz/shared-cache-store/target` | Compressed service store and unpacked repositories | 3.0 GiB |
| `~/.local/share/codex/worktrees/diffz/bazel-trial/target/shared-cache-trial` | Raw HTTP seed, Cargo archive and seed target | 13 GiB |
| `~/.local/share/codex/worktrees/diffz/bazel-trial/target/bazel-remote-bootstrap` | Verified 2.6.3 server binary | 24 MB |
| `~/.local/share/codex/worktrees/diffz/shared-cache-reader/target/grpc-cache-trial` | Warm eager Bazel consumer | 5.5 GiB |
| `~/.local/share/codex/worktrees/diffz/shared-cache-reader/target/shared-cache-real-git` | Warm Cargo consumer | 6.6 GiB |
| `~/.local/state/diffz/evidence/shared-cache-2026-10-19` | Reports, raw logs, profiles and source provenance | retained evidence |

The new `shared-cache-reader` and `shared-cache-store` Git worktrees remain
under the prescribed worktree directory. `cleanup.json` records exact removed
and retained paths and confirms no matching live processes or task scratch.
