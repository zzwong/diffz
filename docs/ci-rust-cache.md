# Keep CI's Rust cache identity tied to the selected toolchain

The three ordinary CI jobs install the repository's pinned Rust toolchain in a
fresh, job-owned `RUSTUP_HOME` before restoring Cargo artifacts. This prevents an
unused Rust version on the hosted runner from invalidating the dependency cache.
The cache action still hashes the selected compiler, Cargo manifests, lockfile,
configuration and relevant environment variables.

## Evidence for the cache miss

Two successful native jobs on 2026-10-19 used the same selected Rust 1.99.0
compiler and the same manifest/lockfile key suffix, but different installed
toolchain inventories:

| Evidence | Earlier main job | Later PR #128 job |
| --- | --- | --- |
| Job | [118301759295](https://github.com/zzwong/diffz/actions/runs/39323056996/job/118301759295) | [118342539318](https://github.com/zzwong/diffz/actions/runs/39335113689/job/118342539318) |
| Selected compiler | 1.99.0, commit `b940084d7eb6a299eb4bfeb8e34901bc051e7ac4` | Same |
| Distinct installed compiler identities in the cache log | 1.99.0 | 1.99.0 and unused 1.99.1 |
| Cache key | `v0-rust-native-Linux-x64-b9e20461-afe67c81` | `v0-rust-native-Linux-x64-e3a6d410-afe67c81` |
| Restore result | Exact hit, approximately 1.83 GB | No cache found |

The pinned [rust-cache v2.9.2 implementation](https://github.com/Swatinem/rust-cache/blob/6323deb102c322ba6fcbdcafc7e3dddab59af2b6/src/config.ts)
collects `rustc -vV` and then runs `rustc -vV` for **every** installed rustup
toolchain. Its environment key hashes the sorted, deduplicated compiler
identities followed by selected environment values. Replaying that exact
sequence with the two jobs' logged inputs reproduces `b9e20461` and `e3a6d410`.
The only added compiler identity was
`1.99.1 x86_64-unknown-linux-gnu 58cf8f9c35b355199727627d07ae34a4e2846d67`.
Both logs selected Rust 1.99.0 and hashed `CARGO_INCREMENTAL=0`.

That changed environment prefix prevented the later job from matching the
earlier known cache key or its restore prefix. This establishes a concrete key
invalidation cause; it does not establish whether other cache entries were
evicted or unavailable under GitHub's branch scope. The jobs also ran different
commits and are not a controlled performance comparison.

## Setup and correctness

[`scripts/prepare-ci-rust.py`](../scripts/prepare-ci-rust.py) requires Python 3.11
or newer and the hosted runner's `rustup` executable. It reads
`rust-toolchain.toml`, installs its channel, profile, components and optional
targets with `--no-self-update`, and validates the result before publishing
`RUSTUP_HOME` through `GITHUB_ENV`. Exactly one installed toolchain must be
active; its executable must live in the owned home, and `rustc` on `PATH` must
report the same compiler. Ambient `RUSTUP_TOOLCHAIN` or `RUSTC` overrides are
rejected so they cannot silently replace the repository selection.

The home is always `$RUNNER_TEMP/diffz-rustup`. With the observed hosted-runner
temporary-directory roots, this resolves to `/home/runner/work/_temp/diffz-rustup` on Linux and
`/Users/runner/work/_temp/diffz-rustup` on macOS. The cache already separates OS,
architecture and job. A random suffix would be incorrect: the cache action also
hashes the value of `RUSTUP_HOME` because it includes nonempty `RUST*` environment
variables. A runner path change can therefore still cause a safe cache miss.

Setup refuses an existing home, including a symlink. It never removes installed
image toolchains, reuses another job's mutable home, or disables compiler/config
hashing. Installation and verification failure fail the step and publish no
environment change. Any partially populated owned directory remains for
inspection and the hosted runner's normal temporary-directory cleanup.

The dependency-audit Docker step keeps `RUSTUP_HOME=/usr/local/rustup`, the
[Rust 1.85.0 Alpine image's own default](https://github.com/rust-lang/docker-rust/blob/ba7100abbc606d3aa8a2966463212aae023e1663/stable/alpine3.20/Dockerfile).
This is a step-scoped override: subsequent host steps still use the job's
isolated home. The [pinned audit entrypoint](https://github.com/EmbarkStudios/cargo-deny-action/blob/3c6349835b2b7b196a839186cb8b78e02f7b5f25/entrypoint.sh)
can install the repository's toolchain inside its musl container. Inheriting
the host home would mix those toolchains because the
[Actions runner mounts its temporary directory into Docker actions](https://github.com/actions/runner/blob/v2.337.0/src/Runner.Worker/Handlers/ContainerActionHandler.cs)
and translates environment paths. The explicit override preserves the audit's
existing container environment and avoids modifying the host's Rust home.

Adding `RUSTUP_HOME` changes the environment key once, so the first run in the
new namespace can be cold. Both observed jobs already installed the pinned
1.99.0 distribution components during `rustup show` (about nine seconds in the
earlier warm job); explicit installation in the owned home keeps that bootstrap
work visible. No reduction in total CI time is claimed until subsequent normal
runs demonstrate reuse.

The core checks, all-feature native tests and Clippy, installer smoke test,
macOS check, and separate package workflows retain their existing commands and
requirements. Local development and release build profiles are unchanged.

## Verification and remaining cache work

### Hosted Ubuntu native dependencies

The ordinary Ubuntu `native` job identifies its prepared native environment
before restoring compiled dependencies. All native package installation,
including `desktop-file-utils` for the later installer smoke test, and the
pinned Rust setup happen first. No package installation remains between the
identity step and compilation. This extends the Debian package job's existing
collector; its container receipt and digest format remain unchanged.

`scripts/debian-build-environment.py --hosted-ubuntu --output <new-receipt>`
records the hosted image's `ImageOS` and `ImageVersion`, installed Debian
package versions/architectures, actual Rust and native compiler/linker binary
hashes and versions, and the C target. The hosted labels are supplied by the
[runner image setup](https://github.com/actions/runner-images/blob/main/images/ubuntu/scripts/build/configure-environment.sh);
they are not an immutable Docker digest. Package and executable changes
therefore invalidate the identity even when those labels stay the same.
Missing/malformed metadata or a failed probe stops before cache restore, rather
than silently selecting an unqualified compiled cache. The receipt is written
exclusively outside the cached target and printed to the identity step's log
before its SHA-256 is exposed to the cache step. Only allowlisted metadata is
collected; no complete environment dump is included.

The digest goes in `prefix-key`, which the pinned action includes in **both**
exact and fallback keys. Rust/config/manifest hashing, job/OS/architecture
partitioning, GitHub's existing PR/main trust scope, and cache publication
behavior remain intact. No whole-commit key or cross-PR promotion is added.
Expect a cold first run in the new namespace, and further safe misses when
native packages or the hosted image change. This does not claim faster CI or
eliminate all possible non-hermetic toolchain inputs.

The compiler-free hosted fixtures can be run with:

```sh
python3 scripts/tests/test_hosted_build_environment.py -v
```

They validate data identity, failure/receipt behavior, unchanged container
schema, and workflow ordering/key wiring without executing Rust, apt, Docker
or native tools. The existing container subprocess fixtures remain separate.
Real hosted collection and current-head CI are correctness gates. A subsequent
same-identity cache hit is separate reuse evidence, not a prerequisite for
merging the identity fix; it may depend on normal trusted-main publication.
Do not claim reuse or a speedup without that evidence. This does not extend native
identity to the core/check or macOS jobs, modify release/manual package cache
policy, qualify shared-worktree reuse, or close #117's broader acceptance.

Run the focused setup regressions with:

```sh
python3 scripts/tests/test_prepare_ci_rust.py -v
```

These fixtures use fake rustup/compiler subprocesses to verify configuration
forwarding, ownership and failure behavior, compiler mismatch rejection, and
the observed cache-key calculation. They do not download a compiler or measure
Rust build performance. The normal CI run exercises real pinned-toolchain
installation and all existing required build gates.

This fix addresses the unused-toolchain invalidation under
[#117](https://github.com/zzwong/diffz/issues/117). That broader issue remains
open: native package/compiler environment identity, cache quota and eviction,
restore/save costs, and persistent worktree reuse still require their own
evidence and changes. This setup does not make the hosted builder's native
environment immutable or add remote execution.
