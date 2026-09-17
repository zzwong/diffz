# Contributing

Issues and small pull requests are welcome. For bugs, include your OS, diffz
version, reproduction steps, and what you expected. Remove private code and
credentials from screenshots or example patches. Report vulnerabilities through
the private channel described in [SECURITY.md](SECURITY.md).

See the [README](README.md#build-from-source) for build requirements. Rustup
uses the toolchain pinned in the repository. The development Make targets are:

```sh
make build
make build-release
make build-plain
make check
make check-native
make run ARGS="--fixture F01"
make install-dev
make uninstall-dev
make help
```

`make build` and `make check` use the optional pinned Kache 0.21.0 workflow;
`make build-plain` uses regular Cargo. The first Kache run includes bootstrap
and cold-build cost. For a full cold seed, run `cargo clean` first; that also
removes the bootstrapped Kache binary, so the next Make invocation installs it
again. A package-scoped clean such as `cargo clean -p diffz` preserves Kache.

### Development builds beside an installed release

A second diffz using the release's state directory fails on its lock, and a
development build could migrate the database past what the release reads. So
`make run` passes `--state-dir $XDG_STATE_HOME/diffz-dev` (default
`~/.local/state/diffz-dev`; set `DEV_STATE_DIR` to change it) before `ARGS`:

```sh
make run ARGS="--git . --base main"
make run ARGS="'/tmp/review files/change.patch'"  # ARGS is split by the shell
make run RELEASE=1 ARGS=--inspect                 # release profile (slow: thin LTO)
```

A `--state-dir` in `ARGS` takes precedence. `--doctor` must be the only
argument, so `make run ARGS=--doctor` omits the state directory.

On Linux, `make install-dev` builds the same way and installs
`~/.local/bin/diffz-dev`, a "Diffz (dev)" launcher
(`io.github.zzwong.Diffz.Dev.desktop`) and its icon under `~/.local/share`,
using the development state directory. It never replaces the release's
`diffz` files, and it does not register for patch files, so the release stays
their default handler. `make uninstall-dev` removes those three files and leaves
the state directory. Both take absolute `PREFIX` and `DATADIR` (default
`$XDG_DATA_HOME` or `PREFIX/share`) paths. Add `KACHE=0` to build with regular
Cargo, or `BIN=target/debug/diffz` to install a binary that is already built.
Windows opened by either build share the `io.github.zzwong.Diffz` app ID.

For desktop changes, run `make check-native` on macOS and try the affected
interaction. Keep PRs focused, add regression tests where useful, and mention
what you tested. Contributions use the project's MIT license.
