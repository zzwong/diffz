# Contributing

Issues and small pull requests are welcome. For bugs, include your OS, diffz
version, reproduction steps, and what you expected. Remove private code and
credentials from screenshots or example patches. Report vulnerabilities through
the private channel described in [SECURITY.md](SECURITY.md).

See the [README](README.md#build-from-source) for build requirements. Rustup
uses the toolchain pinned in the repository. To build and check your changes:

```sh
cargo build -p diffz
bash scripts/check.sh core
```

For desktop changes, also run `bash scripts/check.sh native` on macOS and try
the affected interaction. Keep PRs focused, add regression tests where useful,
and mention what you tested. Contributions use the project's MIT license.

## Optional local build cache

For repeated builds, including builds from separate worktrees, you can opt in
to the pinned Kache wrapper:

```sh
bash scripts/kache-build.sh --locked --release -p diffz
```

The first invocation bootstraps Kache 0.21.0 under `target/kache/0.21.0` with
`cargo install --locked`, then runs `cargo build`. The wrapper does not run
`cargo clean` or rebuild Cargo-fresh targets. For a full cold seed, run
`cargo clean` explicitly first; that also removes the bootstrapped Kache
binary, so the next wrapper invocation installs Kache again. To rebuild only a
package while preserving the Kache binary, use a package-scoped clean such as
`cargo clean -p diffz`, then run the wrapper.

The wrapper explicitly selects the checked-in `scripts/kache.toml` and
`scripts/kache-cargo.toml`. The Cargo layer forces the pinned Kache wrapper,
local-only settings, empty host-config/fallback values, and per-checkout
runtime/socket paths even when ambient Cargo configuration supplies forced
environment entries. The Kache config gives garbage collection an 8 GiB
budget for registered cache-blob bytes. This is a GC budget rather than a hard
disk usage ceiling: shared build outputs can keep blocks retained after GC.
The wrapper overrides Cargo's workspace wrapper for this command and does not
edit global Cargo or Kache configuration, install a service, or use a remote
cache. Kache's normal per-user local store is shared by local worktrees; it is
not part of the repository and is not committed.

After Kache has been installed, check the pinned setup without compiling with:

```sh
bash scripts/kache-build.sh --dry-run --locked --release -p diffz
```

This verifies the pinned installation and prints the effective isolation
paths without compiling diffz.

After the wrapper has bootstrapped Kache, the hostile-environment regression
checks can be run directly with:

```sh
python3 scripts/tests/test_kache_build.py -v
```

The regular `cargo` and `scripts/check.sh` commands remain the supported
default and do not use Kache.
