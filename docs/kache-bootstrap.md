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

The artifact-store policy remains in `scripts/kache.toml`. The Make-selected
socket stays under the checkout's `target/kache/runtime`. The existing TOML
runtime directory is relative; qualifying its behavior for wrappers running
from dependency directories remains part of #116. Executable reuse does not
merge Cargo target directories or incremental outputs. Do not point concurrent
worktrees at one mutable Cargo target directory.
A target clean and an empty Kache artifact store are different cache conditions.

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
