# Changelog

This file records every notable change to the project.

Formatting follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1] - 2026-09-17

### Changed

- `--inspect` prints repository paths as JSON strings. Paths that are not valid UTF-8 stay byte arrays, and saved reviews and snapshot IDs are unchanged.
- Local Arch package builds no longer produce a separate `diffz-debug` package.

### Fixed

- `--inspect`, `--doctor`, `--help`, and `--version` exit cleanly when their output is piped into a command that stops reading, instead of aborting.
- GitHub pull requests and GitLab merge requests open when `gh` or `glab` is a symlinked shim, such as a mise shim on a desktop launcher's `PATH`.
- Errors from `gh` and `glab` include a short, redacted excerpt of the tool's own error output.
- The release `SHA256SUMS` manifest lists Debian package filenames as GitHub publishes them.

## [0.1.0] - 2026-09-14

### Added

- Reads unified patch files
- Compares local Git revisions
- Opens GitHub pull requests through `gh` and GitLab merge requests through `glab`
- Three view modes: split, unified, and wrap
- Rich side-by-side rendering for Markdown and other prose files
- Line and file comments drafted locally, a review preview, and publishing to GitHub or GitLab behind an opt-in flag
- Omarchy themes through `--theme`, with live reload when the theme file changes
- Linux x86_64 RPM and portable archive packages.
- An x86_64 Arch Linux package.
- Distro-specific amd64 DEB packages for Debian 12, Debian 13, Ubuntu 24.04, and Ubuntu 26.04.
- An Apple Silicon macOS DMG release that is ad-hoc signed and not notarized.
- Recognizable file-tree icons across common languages, frameworks, and build and infrastructure formats.

### Changed

- Updated Unicode segmentation to 1.13.3, the CSS grammar to 0.25.0, and SHA-2 to 0.11.0. Snapshot and file hash encoding remains unchanged.
- Improved memory use and rendering performance for large diffs.
- Refreshed macOS fullscreen title-bar spacing and layout.
- Bracket adjacent-file navigation now resets to the first hunk.
- Dragging or flinging the file tree left collapses it and restores its previous width when reopened.

### Fixed

- Theme choices expose accessible names, appearance, and the currently applied theme.
- The keyboard outline for built-in themes matches the choice applied with Enter.
- Comment editors distinguish file comments from line comments in their accessible labels.
- Linux monospace font resolution uses Fontconfig to select a fixed-width system font.
- Corrected TSX highlighting and made syntax colors clearer.
