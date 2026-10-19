# Opt-in Bazel action cache

The Bazel trial keeps action caching disabled by default. For everyday builds,
set `DIFFZ_BAZEL_CACHE=1` to enable a local disk cache at
`target/bazel-disk-cache`:

```sh
DIFFZ_BAZEL_CACHE=1 scripts/bazel-trial.sh build --config=fast-dev //:all_build
DIFFZ_BAZEL_CACHE=1 scripts/bazel-trial.sh test --config=fast-dev //:tests
```

The opt-in `fast-dev` config selects pipelining, two native build jobs,
unoptimized build tools, 32 Bazel jobs, and a 32-CPU reservation. It does not
change the default build configuration.

## Stable test temporary directory

Rust test launchers create isolated temporary directories under
`DIFFZ_TEST_TMP_ROOT`. By default, the wrapper uses
`${TMPDIR:-/var/tmp}/diffz-bazel-tests`. Set `DIFFZ_BAZEL_TEST_TMP_ROOT` to
override that location. Keep the same absolute path across repeated test runs
and on machines sharing an action cache so the test environment stays stable.
On this host, where `TMPDIR=/var/tmp`, the default is
`/var/tmp/diffz-bazel-tests`.

```sh
DIFFZ_BAZEL_TEST_TMP_ROOT=/var/tmp/diffz-bazel-tests \
DIFFZ_BAZEL_CACHE=1 \
scripts/bazel-trial.sh test --config=fast-dev //:tests
```

The directory must be writable; the wrapper creates it for test runs.

The local cache is capped at 10 GiB. Bazel garbage-collects it in the
background after the server has been idle for a while, so it may temporarily
exceed that size while builds are active. The cache lives outside Bazel's
output root and survives `bazel clean`; remove `target/bazel-disk-cache`
when you want to discard it. The repository already ignores `target/`.

For a shared cache over gRPC, add `--remote_cache_compression` to transfer CAS
blobs with zstd. `--config=cache-eager` downloads all intermediate outputs before
editing. It increases fresh-checkout transfer time but avoids paying for those
downloads on the first source edit; the [trial results](bazel-shared-cache.md)
measure both policies. New source still requires compilation.

## Reuse repository contents across worktrees

The action cache and Bazel's repository contents cache serve different work.
The latter reuses downloaded and unpacked external repositories, such as the
pinned Rust toolchain, across clean worktrees on the same host. The toolchain
archives are pinned and SHA-256 verified in `MODULE.bazel`. Keep this cache
inside the dedicated cache-host worktree's `target/`, outside the worktrees
performing builds. Compiled outputs and the local action cache stay under each
build worktree's own `target/`.

Set `DIFFZ_BAZEL_REPO_CONTENTS_CACHE` along with the cache opt-in. The wrapper
resolves the value to an absolute path and rejects it if that path resolves
inside the current workspace. The directory may be created on first use. This
setting applies to `fetch`, `build`, and `test`; `fetch` receives only that
repository-cache flag, while build and test also receive the action-cache
options described above. A nonempty value requires `DIFFZ_BAZEL_CACHE=1`;
other Bazel commands ignore this variable.

Keep `BAZEL_RUST_STDLIB_LINKFLAGS` unset when using this cache. The current
`rules_rust` repository rule reads it without declaring it as a tracked
environment input, so Bazel cannot invalidate cached repository contents
when it changes.

```sh
repo_cache="$HOME/.local/share/codex/worktrees/diffz/shared-cache-store/target/repositories"
mkdir -p "$repo_cache"
DIFFZ_BAZEL_CACHE=1 \
DIFFZ_BAZEL_REPO_CONTENTS_CACHE="$repo_cache" \
scripts/bazel-trial.sh fetch //:all_build

DIFFZ_BAZEL_CACHE=1 \
DIFFZ_BAZEL_REPO_CONTENTS_CACHE="$repo_cache" \
scripts/bazel-trial.sh build --config=fast-dev //:all_build
```

An explicit `--repo_contents_cache=` command-line option takes precedence over
the environment variable. This host-local repository cache does not enable
remote CI caching; the native build environment still needs a pinned image
before cross-host CI results can be trusted.

This remains a Linux/x86_64 trial. The repository cache is host-local, and
native toolchain inputs still need a pinned, compatible environment before
cross-host cache results can be treated as production-portable.

The wrapper can also send action-cache requests to a remote endpoint. This
keeps the bounded local disk cache enabled as well:

```sh
DIFFZ_BAZEL_CACHE=1 \
DIFFZ_BAZEL_REMOTE_CACHE=http://127.0.0.1:8080 \
scripts/bazel-trial.sh build --config=fast-dev //:all_build
```

Bazel uploads locally produced action results by default when a remote cache
is configured. Set `DIFFZ_BAZEL_REMOTE_CACHE_READ_ONLY=1` to disable those
uploads while allowing cache reads:

```sh
DIFFZ_BAZEL_CACHE=1 \
DIFFZ_BAZEL_REMOTE_CACHE=https://cache.example.org \
DIFFZ_BAZEL_REMOTE_CACHE_READ_ONLY=1 \
scripts/bazel-trial.sh test --config=fast-dev //:tests
```

These variables affect `build` and `test`. The wrapper inserts their flags
before command-line arguments, so explicit `--disk_cache=` and
`--remote_cache=` options still take precedence. The cold-build benchmark
passes both empty options last, keeping its action cache disabled even when
the opt-in environment variables are set.

The trial's native build actions depend on host compilers, libraries, and
environment details that Bazel does not fully model as declared inputs.
Treat a shared remote cache as specific to compatible, trusted build
environments; the read-only setting does not make artifacts portable or safe
across different host toolchains. This setup does not add credentials or
authentication headers to the repository. If a remote service requires
authentication, configure that through its supported external credential
mechanism and verify the setup independently before sharing the cache. Do not
put credentials in the endpoint URL or commit them to this repository.

## Restart the retained local trial service

The trial populated the primary checkout's disk cache and retained a verified
`bazel-remote` 2.6.3 binary plus its compressed shared store. All servers were
stopped after verification. To reuse that store, run this in a terminal:

```sh
cache_host="$HOME/.local/share/codex/worktrees/diffz/shared-cache-store"
cache_bin="$HOME/.local/share/codex/worktrees/diffz/bazel-trial/target/bazel-remote-bootstrap/bazel-remote-2.6.3-linux-amd64"
"$cache_bin" --dir="$cache_host/target/bazel-remote" --max_size=10 \
  --storage_mode=zstd --http_address=127.0.0.1:8080 \
  --grpc_address=127.0.0.1:9092
```

In the build terminal:

```sh
export DIFFZ_BAZEL_CACHE=1
export DIFFZ_BAZEL_REPO_CONTENTS_CACHE="$HOME/.local/share/codex/worktrees/diffz/shared-cache-store/target/repositories"
export DIFFZ_BAZEL_REMOTE_CACHE=grpc://127.0.0.1:9092
export DIFFZ_BAZEL_REMOTE_CACHE_READ_ONLY=1
export DIFFZ_BAZEL_TEST_TMP_ROOT=/var/tmp/diffz-shared-cache-tests
scripts/bazel-trial.sh build --config=fast-dev --config=cache-eager \
  --remote_cache_compression //:all_build
scripts/bazel-trial.sh test --config=fast-dev --config=cache-eager \
  --remote_cache_compression //:tests
scripts/bazel-trial.sh shutdown
```

Stop the foreground service with Ctrl-C when finished. It binds only to
loopback and has no authentication; these instructions reuse the trusted local
trial, not a public service. For disk-only operation, unset the remote endpoint
and read-only variables. Keep the same configuration on seed and consumer
builds; do not create a separate shared-cache namespace for each commit.

## Updating the trial

Cargo remains the source of dependency versions. When changing `Cargo.lock`, run
`build //:all_build` and `test //:tests` with the wrapper and commit the regenerated
`MODULE.bazel.lock`. The vendored Linux GPUI target in `bazel/vendor_linux.bzl`
currently names resolved crate versions explicitly; update those labels when its
dependency versions change. The override in `MODULE.bazel` must also match the
vendored GPUI package version. This extra maintenance is one reason the trial
has not replaced Cargo CI.

When changing `rust-toolchain.toml`, update the Rust version and the five official
archive SHA-256 values in `MODULE.bazel` together, then rebuild and test. Bazel
reads the workspace application version directly from `Cargo.toml`; it does not
need a separate application-version edit. The current integration uses Rust
1.99.0 and diffz 0.4.0. The performance reports describe the earlier `fe0581b` /
Rust 1.98.1 trial; their timings have not been relabeled as current-main results.

Install the Linux native dependencies listed in [Linux setup](linux.md) before
building. The trial uses `/usr/bin/gcc` and `/usr/bin/g++`, plus CMake and
pkg-config from `/usr/bin` or the standard Linuxbrew prefix. The native tools
are host dependencies, so reuse a shared cache only between compatible trusted
build environments.
