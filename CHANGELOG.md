# Changelog

This file records every notable change to the project.

Formatting follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Hovering the file-tree toggle, or resting the pointer against the window's
  left edge, while the file panel is collapsed floats the panel over the diff,
  so a file can be picked without pinning the panel back open. It stays while
  the pointer is on the toggle, the edge, or the panel, and closes shortly
  after the pointer leaves them; clicking the toggle pins it as before.
  The panel fades in as it slides the last few pixels into place and fades back
  out on the way, so catching it again with the pointer holds it where it is.

### Changed

- The patched GPUI renderer allocates its path-rasterization targets on the
  first frame that draws a vector path and then keeps them, rather than
  releasing them after two idle seconds. diffz draws no vector paths, so it
  still allocates none of them and its memory footprint is unchanged.

### Fixed

- The rich prose view now shows the same scrollbar as the source view: a thumb
  appears along the right edge while scrolling, reports how far through the file
  the reader is, and can be dragged. Markdown files previously scrolled with no
  scrollbar at all.

## [0.1.2] - 2026-09-19

### Changed

- Touchpad scrolling coasts after the finger lifts on Linux, where the
  compositor delivers raw finger deltas and nothing more; a quick flick now
  travels three to five times as far as the finger did, in line with browsers.
  A finger set back on the pad stops the coast.
- Turning to the neighbouring file at a file edge is now a deliberate pull: a
  second gesture in the same direction fills a bar along that edge and the file
  turns once the bar is full. A slow drag that paused at the edge, a resting
  finger's jitter, or a coast can no longer turn the file by accident.
- `DIFFZ_SCROLL_TRACE=1` writes each wheel event and coast start to stderr.
- Linux windows start faster and use much less memory: the renderer no longer
  initialises Mesa's GL stack beside Vulkan, and path-rasterization targets are
  allocated only while vector paths are drawn. Measured on a 2880x1920 display,
  resident memory falls from 209 MB to 74 MB, GPU memory from 246 MB to 136 MB,
  and content appears about 90 ms sooner. Machines without a usable Vulkan
  driver still fall back to GL.

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
