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
