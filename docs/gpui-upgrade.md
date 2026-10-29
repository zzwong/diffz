# GPUI upgrade and patch maintenance

Diffz uses GPUI Kit **0.7.3** with the **0.3.9** GPUI snapshot family,
from Zed revision `93d745668805029476e6d7fbb19652116fed51de`.
[Kit 0.7.3 release](https://github.com/longbridge/gpui-kit/releases/tag/v0.7.3)
requires that snapshot. A version-only bump bypasses patches whose package
versions no longer match, leaving magnification APIs unavailable. This upgrade
combines application PR #102 and Dependabot #107.

## Sources and patches

`Cargo.toml` pins each fork to an exact Git commit; validation is revision-specific. GPUI's integration branch
is `zzwong/gpui-pre:diffz/main`; GPUI PR #3 integrates the 0.3.9 port. Old
revisions remain usable by consumers pinned to them.

| Source | Patch retained | Exact revision |
| --- | --- | --- |
| gpui-pre | Visual magnification, exact live raster/cache retention, viewport/accessibility, overlay mapping and activation-context API | `7f8bb39b9e7febfd21d17075934dc9dc0cf5501f` |
| gpui-pre-wgpu | Vulkan-first initialization and lazy path textures | `389e7f33bed4f6237ccb53f2b4884d996274a453` |
| gpui-base | Caret blink timeout and inactive-window behavior | `6235089ccddd423369749017dabb4f6df8a3c169` |
| gpui-pre-macos | Demand-driven display link and idle timeout | `28057de61df9442cb521497a63193e70dc36019f` |
| gpui-pre-linux | X11 frame-demand scheduling and direct Wayland activation handoff | Vendored published 0.3.9 plus `patches/gpui-pre-linux-0.3.9-x11-demand.patch` and `patches/gpui-pre-linux-0.3.9-activation.patch` |

The 0.3.9 port preserves upstream display-change notifications and frame-signal
metadata. Window resize re-clamps magnification and remaps the platform mouse
position. A GPUI regression exercises both resize and scale-factor callbacks.
The application adopts Kit's window initialization helper and heading style API.
Main's Wasmtime 48.0.6 and Rustls 0.23.46 security updates are retained.

The Bazel graph matches Cargo's new snapshot and icon assets. After changing
`Cargo.lock`, run `python3 scripts/bazel-vendor-linux.py` and repin Crate Universe
with `CARGO_BAZEL_REPIN=1 scripts/bazel-trial.sh fetch //:all_build`. The generated
Linux target excludes inactive optional dependencies, including weak `scap?/x11`
edges that Cargo metadata can retain. Check both build paths after regeneration.

## Activation integration (October 24, 2026)

[GPUI PR #4](https://github.com/zzwong/gpui-pre/pull/4) adds default-unsupported
activation-context APIs and token-safe Debug formatting. The only real override
is Diffz's vendored Wayland `PlatformWindow`; X11/macOS retain the default and
ordinary activation. [Activation handoff](activation-handoff.md) documents the
finite ownership, limits and qualification boundaries.

The source/manifest-only update changes no dependency declarations inside GPUI.
The Git source in `Cargo.lock` matches the exact reviewed companion commit;
`--locked` application CI must still validate that graph. The historical
October 19 results below do **not** qualify the new revision. Direct GPUI/vendor
tests, isolated backend feature builds and native focus tests remain separate
from the ordinary workspace CI.

The owner [retired the Bazel pilot](https://github.com/zzwong/diffz/issues/119#issuecomment-6386995653)
and [closed its trial without merge](https://github.com/zzwong/diffz/pull/150#issuecomment-6386995399).
The Bazel regeneration guidance elsewhere on this page describes that historical
pilot, not an activation-patch acceptance gate. Tracked Bazel files are retained
unchanged and their old GPUI graph is **not qualified for this new pin**. No
regeneration, further pilot qualification, or tracked-file deletion is part of
this update. Cargo's exact lockfile and authentic GPUI integration remain required.

## October 19, 2026 validation

Local Linux checks passed:

- Locked all-feature application build, workspace tests and strict all-target
  Clippy using the `fast-dev` profile; workspace formatting.
- Bazel `//:all_build` with `--config=fast-dev --config=cache-eager`, then
  `//:tests --nocache_test_results`: all 30 targets pass.
- 400 GPUI library tests, including painting/input mapping, exact live raster,
  accessibility/overlay geometry, display tracking and resize/scale callbacks.
  The standalone harness supplies upstream font/dev fixtures omitted from the
  published crate and runs three perf assertions as ordinary tests. One upstream
  property test is excluded because its published macro fails to compile; two
  upstream tests remain ignored. These are test-harness changes, not runtime
  changes to the pinned fork.
- The vendored Linux source exactly matches published 0.3.9 plus the recorded
  X11 patch. Downloaded source archives match the official crates.io index
  SHA-256 checksums.

Rendered checks use a private Xvfb display with Lavapipe, isolated fixture state,
and XTest input. Both main's 0.3.3 build and the 0.3.9 candidate render F01 and
accept comment text. The candidate shows a caret immediately on editor focus,
renders heading previews, repaints after resizing to 900×620, and accepts typing
and toolbar clicks with `DIFFZ_DEBUG_MAGNIFY=2@100,100`. Screenshots were inspected.
This is functional verification with software Vulkan, not a GPU performance
comparison. It does not establish hardware pinch behavior or native Wayland,
macOS or Windows runtime behavior. The new macOS CI job checks all application
targets with all features; it is a compile gate, not a runtime test.

Logs, source checksums, patch deltas, the standalone GPUI harness and screenshots:
`/home/aaron/.local/state/diffz/evidence/gpui-refresh-2026-10-19/`.

## Future upgrades

Import a complete matching snapshot family, then port the four fork patches and
vendored X11 delta before changing Kit. Preserve upstream changes at conflicts
rather than replaying old platform code wholesale. Verify exact Git pins remain
fetchable, regenerate Cargo/Bazel locks and icons, and refresh the Arch source
commit/checksum after committing application changes. Run Linux tests, macOS
compilation and rendered input checks before merging. Remove a patch only when
the chosen upstream snapshot includes its behavior and regression coverage.

Historical 0.3.3 GPUI work branches remain under archive tags:
`archive/0.3.3-upstream-baseline`, `archive/0.3.3-visual-zoom`,
`archive/0.3.3-live-pinch-exact-raster`, and `archive/0.3.3-magnification-polish`.
Earlier 0.3.7 candidate checks and limited Wayland observations are historical;
they do not qualify this 0.3.9 port.
