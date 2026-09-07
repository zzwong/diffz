# Contributing to diffz

Thanks for wanting to help out. This document explains how to file bugs,
set up the dev environment, and send pull requests.

## Reporting bugs

File a GitHub issue with the bug report template. Tell us:

* Your OS and its version
* The diffz version (`diffz --version`) and, for a source build, the commit (`git rev-parse --short HEAD`)
* The steps that reproduce the problem
* What you expected, and what you got instead

## Development setup

Building needs a Rust toolchain. Because `rustup` reads the pinned version from
`rust-toolchain.toml`, installing rustup is all you need:

```sh
rustup show
```

The core crate builds without any GUI dependencies:

```sh
cargo build -p diffz-core
```

Before you push, run these core checks:

```sh
bash scripts/check.sh core
```

and clippy across the workspace:

```sh
cargo clippy --workspace --all-features --all-targets -- -D warnings
```

The desktop app builds without extra setup on macOS. On Linux, first install
the system packages the README lists.

## Pull requests

* Keep each pull request small and about a single concern.
* Add tests when you change `diffz-core` or `diffz-adapters`.
* Run `cargo fmt` so the formatting matches the rest of the repository.
* Complete the PR template and list which checks you ran.

## Commit messages

Subject lines are imperative and stay under 72 characters. For example:
`Add live theme reload support`.

## License

Contributing means your work is covered by the project's license, the MIT License.
