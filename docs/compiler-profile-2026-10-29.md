# Current compiler qualification — 29 October 2026

Retain the existing LLVM development and release profiles. On the current
Wasmtime/Cranelift graph, disabling Cranelift's local ThinLTO made the screened
builds faster but slowed an actual extension workflow. The Cranelift rustc backend
failed the full application's and test graph's link gates. TPDE is not yet an
available nightly option. This completes the bounded investigation in
[#127](https://github.com/zzwong/diffz/issues/127); broader workflow and adoption
work remains in [#115](https://github.com/zzwong/diffz/issues/115) and
[#118](https://github.com/zzwong/diffz/issues/118).

No compiler, profile, dependency, release, or application default is promoted by
this report. The repository change adds the timing analyzer and its fixtures.

## Source and controls

- Source: `045b8fe2483dd892e3fc04ce9a4c848ad98c7dc4`; Cargo.lock SHA-256:
  `10e047857c7f91bded01bea417bd88d10428aec6fb7fbd6fb87328907d1b1850`.
- Current packages: Wasmtime **48.0.7**, Cranelift **0.135.7**, GPUI **0.3.9** /
  Kit **0.7.3**. Earlier 0.135.6 compiler measurements are historical inputs.
- Fedora 44, x86-64 Linux, Ryzen AI MAX+ 395, 16 cores / 32 logical CPUs,
  approximately 125 GiB physical RAM. Affinity was CPUs 0–31; Cargo jobs **16**.
- The normal baseline uses pinned Rust/Cargo **1.99.0**, rustc
  `b940084d7eb6a299eb4bfeb8e34901bc051e7ac4`, LLVM **23.1.1**.
- Explicit desktop workload: `-p diffz --bin diffz`, default
  `desktop,syntax,wasm` features, dev profile, incremental **off**. These are
  artifact-cold builds with downloaded sources and ordinary OS caches warm;
  they are not incremental edit-loop measurements.
- Both Rust wrappers were cleared. Native tools were `/usr/bin/gcc` and
  `/usr/bin/g++`, with `CCACHE_DISABLE=1`. Each build had its own empty target
  inside the isolated worktree. Fetch/setup and tests were recorded separately.

The workstation retained ordinary desktop activity. There was no competing
compiler or cache server started by this task. Samples are a bounded local
screen, not a cross-host, macOS, or CI speedup estimate.

Raw evidence is retained at
`~/.local/state/diffz/evidence/127-2026-10-29/`. The checked-in
[measurement summary](compiler-profile-2026-10-29.json) contains individual
samples and compiler events; the evidence directory contains full commands,
logs, Cargo HTML, profiles, tool hashes, and qualification receipts.

## Current Cargo critical paths

The normal pinned-toolchain desktop build completed in **100.118 s**. Cargo's
849 unit spans cover **99.82 s**. At least one ready unit waited for a CPU slot
for **71.15 s** of that trace, with at most **92** ready units queued.

| Unit | Elapsed span | Cargo frontend / metadata span | Cargo codegen span |
| --- | ---: | ---: | ---: |
| cranelift-codegen | 49.27 s | 13.42 s | 35.85 s |
| gpui-pre | 41.61 s | 9.86 s | 31.75 s |
| libsqlite3-sys build-script run | 26.29 s | — | — |
| naga | 23.27 s | 5.25 s | 18.02 s |
| wgpu-core | 22.61 s | 3.89 s | 18.72 s |
| gpui-component | 22.09 s | 10.40 s | 11.69 s |
| gpui-base | 21.42 s | 8.84 s | 12.58 s |
| final diffz binary | 0.73 s | not isolated | not isolated |

These invocations overlap. Their elapsed spans are neither additive wall time
nor CPU time. Cargo's metadata release can start a consumer while its dependency
continues code generation. The analyzer follows one latest-release recorded
unblock path and exposes all recorded predecessors for each selected step; it
does not predict the gain from removing a dependency. See
[Cargo's timing definitions](https://doc.rust-lang.org/cargo/reference/timings.html).

In this trace the final path reaches GPUI Base's metadata, GPUI Component's full
completion at 99.33 s, then the application. SQLite's build script ends at
96.37 s on another branch. A long recorded ready gap precedes the late GPUI
work. This makes scheduler contention and native build work relevant alongside
compiler cost. It does not establish that a frontend-thread change will shorten
the complete build.

The final binary's entire 0.73-second span bounds the opportunity for a linker
replacement in this sample. The separate compiler replay records a 0.50-second
linker event. A linker trial is not selected; release ThinLTO tuning remains a
separate workload in #118.

## Compiler self-profiles

Six captured Rust 1.99.0 compiler invocations were replayed into separate output
directories with a 16-token jobserver. Diagnostic `RUSTC_BOOTSTRAP` was restricted
to each named crate; replays requested profiler/time-passes flags. It is not
used in the paired build/runtime trials or project defaults.
[The compiler's bootstrap behavior](https://doc.rust-lang.org/unstable-book/compiler-environment-variables/RUSTC_BOOTSTRAP.html)
is explicitly outside stable guarantees.

The matching analysis tool source is measureme **12.0.3**, revision
`5ac839c602b59eee9c908b3b35b6d6c0cd1c42f7`. Both `summarize --json` and `crox`
successfully decoded every real profile. An analysis-tool-only
`--cap-lints=warn` allowed upstream redundant-unsafe warnings under Rust 1.99;
application compiler flags were unaffected.

| Crate | LLVM module optimization | LLVM local LTO optimization | LLVM object emission | `typeck_root` self | Mono graph-walk self |
| --- | ---: | ---: | ---: | ---: | ---: |
| cranelift-codegen | 18.560 s | 17.490 s | 16.300 s | 1.744 s | 0.033 s |
| gpui-pre | 14.928 s | 12.727 s | 11.925 s | 0.872 s | 0.035 s |
| gpui-base | 22.135 s | 19.495 s | 19.912 s | 0.717 s | 0.052 s |
| gpui-component | 23.631 s | 20.809 s | 24.565 s | 1.023 s | 0.058 s |

These are **summed event self times across profiled threads**, not elapsed
budgets or kernel CPU counters. Kernel user/system time, maximum individual
compiler-process RSS, event counts, and every remaining event are in the raw
receipts. Replay wall times also differ from contended full-build spans.
The selected type-checking event is not the total of all frontend queries.

The primary profiles use `default` events. A separate `default,args` calibration
spent **51.7 s** serializing GPUI query keys alone, versus a 14.7-second default
event replay of that crate. Those detailed traces are retained as calibration,
not used to rank compile latency. The compiler documents this
[extra tracing cost](https://doc.rust-lang.org/unstable-book/compiler-flags/self-profile-events.html).

The measured work supports a codegen experiment. It does not identify a dominant
frontend/query action or a narrow, justified type-erasure boundary. No frontend
thread sweep or speculative crate/API split is selected. Revisit those choices
when a profile of the target workflow identifies the corresponding bottleneck.

## Current Cranelift local-ThinLTO / CGU16 trial

Both lanes used the exact production rustc **1.99.0**. Dated-nightly Cargo
**1.101.0**, `nightly-2026-10-28`, was used only to expose the experimental
[per-package rustflags interface](https://doc.rust-lang.org/cargo/reference/unstable.html#profile-rustflags-option).
`RUSTC` and `RUSTDOC` explicitly selected the stable 1.99.0 binaries; there was
no `RUSTC_BOOTSTRAP`. This driver difference is why these paired times are
reported separately from the ordinary Cargo 1.99.0 baseline.

Both lanes explicitly set Cranelift to CGU16, the
[non-incremental compiler default](https://doc.rust-lang.org/rustc/codegen-options/index.html#codegen-units).
Only the candidate adds `-Clto=off` to `cranelift-codegen`, for dev and test
profiles. Other package profiles/features are retained. Every build had 849
units. The six verbose compiler logs confirm the actual compiler and selected
flag; the normal global LLVM release profile is unchanged.

Order was baseline/candidate, candidate/baseline, baseline/candidate. Each target
started empty. No compiler replay or tool installation overlapped these builds.

| Lane | Individual complete build times | Median | Range |
| --- | --- | ---: | ---: |
| Baseline local ThinLTO, CGU16 | 109.275, 109.133, 100.582 s | 109.133 s | 100.582–109.275 s |
| Cranelift LTO off, CGU16 | 103.475, 104.140, 99.069 s | 103.475 s | 99.069–104.140 s |

The screened median is 5.2% lower. All three pairs improve, by **1.5–5.8 s**;
there is substantial variation and overlap between the ranges. The median
Cranelift invocation span changes **50.90→45.13 s**. Its Cargo frontend span also
varies under contention, so that difference must not be attributed directly to
the codegen flag.

Both all-feature workspace lib/bin/test suites passed **440 tests, four ignored**. F01–F03 parsed
inspection JSON is identical, as are help/version output. Both default desktop
binaries retain DWARF and unwind sections.

The two actual extension tests were then run in new processes, 15 samples per
lane/workload, alternating lane order:

| Workflow | Baseline median / range | Candidate median / range | Median change |
| --- | --- | --- | ---: |
| Wasm grammar highlight/compile | 46.18 / 43.75–65.14 ms | 54.29 / 51.81–74.58 ms | +17.6% |
| Component annotation through registry | 44.03 / 38.03–55.55 ms | 45.84 / 44.37–63.10 ms | +4.1% |

Every exact named test executed once and passed. These process-level screens
include startup, extension validation, compilation, and instantiation; they do
not estimate steady-state throughput or frame pacing. The observed median
slowdowns prevent promoting this candidate despite its modest build-time result.
**Decision: retain experiment; keep the existing profile.**

## Alternative backend compatibility

The LLVM and Cranelift backend lanes used the same
**nightly-2026-10-28** toolchain: rustc **1.101.0** at
`41089d07a0a2afc16adfe28a760d94d73d7298a4`, LLVM **23.1.4**. The installed
Cranelift backend SHA-256 is
`041b5cb8e1b8cc1b2d966f3ae3bc3958ebd590473a9debe9b676e8f2a34c7093`.
The distribution manifest, source-tree identities and actual commands are
retained. This is a single compatibility screen, not a speedup comparison with
Rust 1.99.0.

| Gate | Nightly LLVM | Nightly Cranelift backend |
| --- | --- | --- |
| Full default desktop build | passed | Cargo 101, link failed |
| Compile all-feature workspace test graph | passed | Cargo 101, link failed |
| Execute complete test graph | passed | unavailable after failed compile |
| F01–F03 inspection | passed, matches stable LLVM | no qualified executable |

Both failed Cranelift graphs report unresolved `libpulp_v0_22_3_*` symbols,
including `libpulp_v0_22_3_ld_b32s_0000000000000000`. The matched LLVM control
passes. The logs establish an integration incompatibility, without attributing
an upstream root cause from linker output alone. No panic-abort or feature
removal workaround was used to manufacture a pass.

The backend documents incomplete SIMD support and platform-dependent unwind
limitations in its [source README](https://github.com/rust-lang/rustc_codegen_cranelift/blob/d53d0f7b3783cdb8657df73c8cf339ebe336112b/Readme.md).
**Decision: defer** until the real application and full test graph link under a
pinned backend; then require fixture, runtime, GUI and debug/unwind qualification
before adoption. LLVM release qualification remains authoritative.

## TPDE availability and final decisions

As checked on 29 October, [bootstrap integration #163731](https://github.com/rust-lang/rust/pull/163731)
is merged, while the actual [backend option #164108](https://github.com/rust-lang/rust/pull/164108)
remains open. The installed pinned nightly's `-Zhelp` has no `llvm-tpde` option.
The [project goal](https://goals.rust-lang.org/2026/tpde.html) and
[tracking issue](https://github.com/rust-lang/goals/issues/802) describe experimental
Linux/ELF work, not a stable or macOS replacement. **Defer** until a pinned
nightly exposes the backend option and ships the required plugin, then run the
same actual-app/test gates. No unavailable feature is marked implemented.

| Candidate | Decision and next condition |
| --- | --- |
| Cranelift 0.135.7 LTO off / CGU16 | **Retain experiment**; observed extension-workflow regression blocks promotion. |
| rustc_codegen_cranelift | **Defer**; both real graphs must link before runtime qualification. |
| TPDE | **Defer**; wait for merged/backend-distributed nightly support. |
| Frontend threads | **Defer**; no dominant frontend action established in this profile. |
| Generic boundary / crate split | **Defer**; no measured narrow source boundary supports an implementation change. |
| Linker / release ThinLTO | **Defer**; final-link opportunity is small here; release is a separate workload. |

Native Linux and macOS promotion qualification is explicitly out of scope
because no behavior is promoted. Linux correctness/runtime observations are
limited to the recorded workloads; no macOS benchmarking or GUI qualification
is claimed.
Existing Linux/macOS CI and release gates are retained. This result does not
close the broader baseline/cache/profile adoption issues.

## Reproduction and retained evidence

Use a clean isolated worktree and a fresh evidence directory:

```sh
RUSTC_WRAPPER= RUSTC_WORKSPACE_WRAPPER= RUSTFLAGS= CARGO_ENCODED_RUSTFLAGS= \
  taskset -c 0-31 python3 scripts/benchmark-cold-builds.py \
  --tool cargo --cargo-workload desktop --cargo-toolchain 1.99.0 \
  --cargo-incremental off --jobs 16 --iterations 1 --cargo-arg=-vv \
  --output "$HOME/.local/state/diffz/evidence/127-repeat/stable-baseline"

python3 scripts/analyze-cargo-timings.py \
  "$HOME/.local/state/diffz/evidence/127-repeat/stable-baseline/cargo-timing-1.html" \
  --output "$HOME/.local/state/diffz/evidence/127-repeat/analysis.json"
```

The analyzer parses JSON embedded in Cargo's HTML without executing JavaScript.
Missing timing/concurrency records, malformed values or invalid edges fail. Its
fixtures cover overlapping units, metadata release, multiple predecessors,
ready gaps, native build scripts, UTF-8 and CLI non-success behavior. The
embedded Cargo format is not a stable API; schema changes require revalidation.

The evidence root contains:

- `stable-baseline/`: setup, exact versions/commands, raw build/test logs,
  measurements, Cargo HTML, extracted invocations, analysis and completion.
- `self-profiles-default/`: six command/usage receipts, raw `.mm_profdata.zst`,
  summaries and decoder results. One compressed Chrome trace is retained;
  other Chrome projections can be regenerated with pinned `crox`.
- `self-profiles/`: the separate detailed-event calibration and its overhead.
- `lto-trial/`: all six cold-build logs/HTML, two full test logs, exact fixture
  outputs, DWARF/unwind checks and every alternating runtime sample.
- `backend-trial/`: pinned distribution/compiler/backend identities, all
  successful LLVM results and both failed Cranelift compile/link attempts.
- `runners/`, `tool-provenance.json`, `storage-manifest.json`, `summary.json`:
  actual collectors, source/binary hashes, lossless compression receipts and
  individual results. Decompress profiles into project-specific `$TMPDIR`
  scratch before using the pinned measureme tools.

Builds and toolchains are disposable worktree outputs, not retained evidence.
