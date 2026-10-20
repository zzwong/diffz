# Reusing the pinned Kache executable

Kache-backed Make commands install the pinned 0.21.0 executable once per local
host and tool directory. The default directory is:

```text
$XDG_CACHE_HOME/diffz/tools/kache/0.21.0/<rust-host>/bin/kache
```

An unset or relative `XDG_CACHE_HOME` uses `~/.cache`. The host is the selected
`rustc -vV` host triple, such as `x86_64-unknown-linux-gnu` or
`aarch64-apple-darwin`. Paths containing spaces are supported.

The version and host namespace separate incompatible tools. This directory is
for worktrees on the same local host. It is not a portable executable cache for
different machines, container images, libc versions, or network-mounted homes;
use an explicit `KACHE_ROOT` for each such native environment.

## What is reused

The first `make kache` performs `cargo install --locked --version 0.21.0` and
explicitly targets the selected Rust host. It clears both compiler wrappers and
gives bootstrap compilation its own temporary target and build directories.
An inherited `CARGO_BUILD_TARGET` cannot select a different install target.
Source downloads can still use Cargo's normal registry cache.

Later invocations verify and reuse the executable. A new checkout no longer
needs another source installation, and `cargo clean` does not remove the tool.
This avoids repeated wrapper installation; it does not itself speed up rustc
or establish a compiled-artifact cache hit.

The artifact-store policy remains in `scripts/kache.toml`. By default, Make keeps
runtime and socket under the checkout's `target/kache/runtime`. Before
starting Cargo, `scripts/run-kache.py` resolves the policy's runtime directory
against the checkout root and selects an absolute effective config. This keeps
compiler wrappers invoked from nested package directories on that same runtime.
Executable reuse does not merge Cargo target directories or incremental outputs.
Do not point concurrent
worktrees at one mutable Cargo target directory.
A target clean and an empty Kache artifact store are different cache conditions.

## Runtime configuration

The checked-in TOML remains the policy source. Kache's `ignore_env = true`
intentionally ignores an ambient `KACHE_RUNTIME_DIR`; setting that environment
variable alone cannot make the relative policy path independent of compiler cwd.
The launcher changes only `cache.runtime_dir` in an effective TOML snapshot and
preserves all other settings and comments. The source file is not rewritten.

Snapshots live under `target/kache/config/<content-sha256>.toml`, with owner-only
file permissions. Complete files are published atomically and reused unchanged.
Different concurrent config selections cannot overwrite a file another compiler
is about to read. A target clean removes these small generated files along with
other checkout outputs, and the next Make command prepares them again.

The launcher rejects invalid TOML, a non-string/empty/NUL-containing runtime setting,
and unexpected generated-file contents or ownership before starting the requested
command. Quoted or dotted keys, inline tables and multiline TOML strings are
supported. A custom policy that omits `cache.runtime_dir` receives the same
checkout-local `target/kache/runtime` default. Other policy settings retain
Kache's own meaning and validation.
The default artifact-store location and its 8 GiB policy are unchanged.

For an explicit policy, config and socket paths are resolved against the checkout
root; absolute paths and spaces are supported. For example:

```sh
make build KACHE_CONFIG='/absolute/policies/local kache.toml' \
  KACHE_SOCKET='target/custom runtime/daemon.sock'
```

An absolute or `~/` runtime path in that policy is preserved or home-expanded;
it can intentionally select a shared location. When a custom policy permits
environment overrides, an existing `KACHE_RUNTIME_DIR` keeps its precedence,
with relative paths resolved against the checkout before Cargo changes cwd.
The launcher continues to disable the separate host config and select the
Make-requested socket and compiler wrapper. It invokes the requested command
directly, preserving its arguments, exit code and termination signals.

## Concurrent use and failures

`scripts/bootstrap-kache.py` resolves parent-directory aliases and takes a
process lock beside the selected install directory. Two cooperating Make
invocations install once; the second waits and then validates the completed
tool. The lock inode is kept permanently so waiters cannot acquire different
locks for the same destination.

Installation happens in a unique hidden staging directory on the same
filesystem. The helper checks the candidate's version before atomically
renaming the completed install into place. Failed installs propagate their exit
status and remove only their own staging tree. An ordinary interrupted install
does not publish a partial tool. A forced kill or host crash can leave an unused
hidden stage, but later invocations do not select it.

An existing incomplete directory, wrong-version executable, or symlink in the
managed installation fails explicitly. The helper does not overwrite or remove
that state. An error identifies the selected path so it can be inspected before
repair. An unrelated external executable is never overwritten.

## Overrides and plain Cargo

Explicit Make overrides remain available:

```sh
# A separate native environment or an intentionally checkout-local tool.
make build KACHE_ROOT=/absolute/project-tool-cache/kache/0.21.0
make kache KACHE_ROOT=target/kache/0.21.0

# An already installed, exact-version executable. Missing/wrong versions fail.
make build KACHE_BIN='/absolute/preinstalled tools/kache'

# Wrapper-free escape routes; no Kache bootstrap.
make build-plain
make run KACHE=0 ARGS='--fixture F01'
make install-dev KACHE=0
```

Changing `KACHE_VERSION` also changes the default version namespace. It is an
experiment override, not qualification of a newer Kache configuration or cache
format. The migration and its cleanup/correctness policy remain tracked in
[#116](https://github.com/zzwong/diffz/issues/116).

## Removing owned tool state

Stop Kache-backed builds and daemons in the relevant worktrees first. Remove
only the exact version/host tool directory you intended to retire. Leave its
sibling `.install.lock` in place: unlinking it while an installer or waiter may
exist can split the lock. Do not remove a shared `diffz/tools` parent as a repair
shortcut. The next invocation recreates a removed tool through normal bootstrap.

After confirming that no installer is running, leftover hidden
`.<rust-host>.install-*` stages beside that version's host directory may be
removed individually. The helper deliberately has no automatic stale-stage or
artifact-store cleanup policy. It never runs `kache init`, edits global Cargo
configuration, or purges another checkout's outputs.

## Verification

`python3 scripts/tests/test_bootstrap_kache.py -v` uses subprocess fixtures to
exercise real file locks and atomic publication without compiling Kache. It
covers concurrent aliases, two-worktree reuse after target removal, failed and
wrong-version candidates, retained external/symlink state, host selection,
spaces, overrides, and both wrapper-free Make routes. CI runs these fixtures on
Linux and macOS; existing Rust/native checks remain in place.

These tests establish bootstrap behavior. They do not substitute for the
version-migration correctness experiments or measured artifact reuse in
[#116](https://github.com/zzwong/diffz/issues/116) and
[#117](https://github.com/zzwong/diffz/issues/117).

`python3 scripts/tests/test_run_kache.py -v` covers absolute runtime selection,
custom policies, concurrent immutable snapshots, spaces, plain-Cargo bypass and
command failure propagation. These fixtures run in the existing Linux/macOS CI
jobs without compiling or installing Kache. An optional real pinned-wrapper
probe runs `rustc -vV` from checkout and nested crate directories and requires
both passthrough events to land in the same runtime:

```sh
DIFFZ_TEST_KACHE_BIN=/absolute/path/to/kache-0.21.0 \
DIFFZ_TEST_RUSTC=/absolute/toolchain/bin/rustc \
  python3 scripts/tests/test_run_kache.py \
    RuntimeTests.test_real_pinned_wrapper_keeps_nested_crate_events_in_one_runtime -v
```

The real probe uses fresh isolated HOME/XDG/store/runtime paths, disables
automatic cleanup, and compiles no Rust code. It verifies runtime-path behavior;
the full application, migration, cache performance and daemon lifecycle gates
remain tracked separately in #116/#117.
