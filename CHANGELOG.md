# Changelog

This file records every notable change to the project.

Formatting follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Reads unified patch files
- Compares local Git revisions
- Opens GitHub pull requests through `gh` and GitLab merge requests through `glab`
- Three view modes: split, unified, and wrap
- Rich side-by-side rendering for Markdown and other prose files
- Line and file comments drafted locally, a review preview, and publishing to GitHub or GitLab behind an opt-in flag
- Omarchy themes through `--theme`, with live reload when the theme file changes

### Changed

- Updated Unicode segmentation to 1.13.3, the CSS grammar to 0.25.0, and SHA-2 to 0.11.0. Snapshot and file hash encoding remains unchanged.

### Fixed

- Theme choices expose accessible names, appearance, and the currently applied theme.
- The keyboard outline for built-in themes matches the choice applied with Enter.
- Comment editors distinguish file comments from line comments in their accessible labels.
