# Cranelift generator reproducibility

The opt-in Bazel graph carries two narrow patches for the locked Cranelift
0.135.8 release. The Wasmtime 48.0.8 security update advances the locked
Cranelift dependencies; features, optimization, debug policy and release gates
are unchanged. The upstream assembler and ISLE build scripts retain the exact
source identities guarded by the verifier. Real generator qualification remains
an enforced CI gate for the updated archives. Tracking: [#142](https://github.com/zzwong/diffz/issues/142).

`cranelift-assembler-x64` generates `generated-files.rs` for its path-listing
helper. The original embeds the producer's absolute output directory. The patch
emits an output-relative path joined to the **consumer helper's compile-time**
`OUT_DIR`. It leaves `assembler.rs` generation unchanged. This makes the metadata
source portable, not an already-compiled path-listing helper.

`cranelift-isle` generates Rust test declarations by iterating example directories.
The patch sorts filenames within each existing directory/runner group. Test names,
runner calls, input paths, and membership are preserved. Rust's
[`read_dir` contract](https://doc.rust-lang.org/std/fs/fn.read_dir.html) explicitly
does not guarantee iteration order.

## Real generator check

Use the repository's pinned Rust toolchain. First run the ordinary core/Wasm
checks so the locked archives are cached, then:

```sh
bash scripts/check.sh core
python3 scripts/verify-cranelift-generators.py \
  --output "$HOME/.local/state/diffz/evidence/cranelift-generators-new"
```

The verifier does not fetch sources, update Cargo.lock, or compile the desktop.
It verifies the four source archives against Cargo.lock, rejects version/source
drift, unpacks regular files into owned scratch, and applies the checked-in
patches there with zero fuzz. It builds the **real** srcgen and assembler meta
libraries, both upstream and patched build scripts, and the actual path-listing
helper using `rustc`. It checks:

- Identical assembler code across three output roots per arm, including spaces,
  quotes, backslashes and Unicode; candidate metadata must be byte-identical.
- Correct helper paths in all roots and after transferring metadata and
  recompiling the helper against a new consumer `OUT_DIR`.
- Identical ISLE output under opposite directory-creation orders, the same test
  membership as upstream, and lexical ordering within each existing group.
- A new `.isle` input appears, restoring it recovers the original generated
  bytes, unrelated files are ignored, and a missing input directory fails.

It records commands, exit codes, compiler identity, source checksums and generated
output hashes in a fresh evidence directory. Missing tools/archives, unexpected
outputs and compilation failures are **errors**, not skipped passing checks.
Earlier evidence is never overwritten. The existing Linux CI check runs this
bounded test after normal core tests populate its source cache; no compiler
variant matrix or extra full-desktop job is added.

The Python guard suite runs without Rust and tests extraction, source selection,
failure propagation and evidence preservation. Its fixture results are not Rust
build measurements:

```sh
python3 scripts/tests/test_cranelift_generators.py -v
```

## Scope and remaining gates

Successful generator checks establish reproducible metadata and unchanged
assembler source/test membership. They do **not** establish faster whole builds,
GUI behavior, macOS qualification, cross-host cache reuse or remote execution.
Regenerate the actual Bazel dependency graph, compile the affected desktop and
extension tests, then measure a fresh cache consumer before asserting those gains.
That integration/adoption work remains in [#119](https://github.com/zzwong/diffz/issues/119).
Do not interpret Bazel's local `DIFFERENT_FILES` counter as remote-cache misses.

When upgrading Cranelift, recheck the upstream fixes and remove these patches when
unneeded. Until then, the explicit version and source checks require requalification.

Upstream source: [assembler](https://github.com/bytecodealliance/wasmtime/blob/v48.0.8/cranelift/assembler-x64/build.rs),
[helper](https://github.com/bytecodealliance/wasmtime/blob/v48.0.8/cranelift/assembler-x64/src/main.rs),
[ISLE](https://github.com/bytecodealliance/wasmtime/blob/v48.0.8/cranelift/isle/isle/build.rs).
