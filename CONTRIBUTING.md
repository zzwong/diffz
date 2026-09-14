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
make help
```

`make build` and `make check` use the optional pinned Kache 0.21.0 workflow;
`make build-plain` uses regular Cargo. The first Kache run includes bootstrap
and cold-build cost. For a full cold seed, run `cargo clean` first; that also
removes the bootstrapped Kache binary, so the next Make invocation installs it
again. A package-scoped clean such as `cargo clean -p diffz` preserves Kache.

For desktop changes, run `make check-native` on macOS and try the affected
interaction. Keep PRs focused, add regression tests where useful, and mention
what you tested. Contributions use the project's MIT license.
