# Releases

The version in the Cargo workspace's `[workspace.package]` section is the
source of truth for every release.

## Release-preparation PR

The release-preparation PR updates the Cargo workspace version in `Cargo.toml`
and moves the release entries in `CHANGELOG.md` under a dated
`## [<version>] - YYYY-MM-DD` section, leaving `## [Unreleased]` empty. Wait
for its required checks to pass, review it, and merge it before preparing the
Arch pin.

After that PR merges, compute the reproducible source archive from its tested
merge commit and record its SHA-256. Then open a small Arch-pin PR that sets
`pkgver`, `pkgrel`, `_commit`, and `sha256sums` in
`packaging/arch/PKGBUILD`; `_commit` must point to the tested release-preparation
merge commit, not to the Arch-pin PR's own merge commit, while `sha256sums`
verifies the generated source archive. Wait for the Arch-pin PR's required
checks to pass and merge it.

## Linux release

After the Arch-pin PR is green, create the immutable `v<version>` tag at the
tested Arch-pin merge commit and push it. The tag must match the Cargo workspace
version and triggers the Linux release workflow, which builds every supported
Linux format:

- x86_64 RPM
- x86_64 portable `.tar.gz` archive
- x86_64 Arch Linux package
- distro-specific `amd64` DEBs for Debian 12, Debian 13, Ubuntu 24.04 LTS, and
  Ubuntu 26.04 LTS

The workflow creates the GitHub Release with generated notes and one combined
`SHA256SUMS` manifest covering all Linux assets. See [Linux](linux.md) for
package details and local build instructions.

A manual workflow dispatch is build-only: it does not create or publish a
GitHub Release. Never rewrite a release tag; make a new version and tag for a
correction.

## macOS prerequisite

macOS 15 or newer is required. macOS signing and notarization are a separate
prerequisite for any public macOS download. Unsigned ZIPs may be used for local
testing, but they are not public release assets and must not be presented as
releases.
