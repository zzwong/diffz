# GPUI upgrade and visual magnification

## Current decision

Keep the Zed-derived `gpui-pre` stack while evaluating framework migrations
separately. GPUI-CE supplies pinch input, but its current `Window` implementation
does not supply diffz's whole-window visual magnification or live raster policy.
GPUI Kit pins the Zed-derived snapshot, so CE is not a dependency-only replacement.

## Zoom polish

The `codex/magnification-polish` branch of `zzwong/gpui-pre` maps accessibility
rectangles to magnified window/device coordinates and fits anchored menus and
tooltips against the visible content viewport. Layout and accessibility click
targets remain in content coordinates.

Focused magnification, anchored-menu, and exact-live-cache tests pass on both
the 0.3.3 baseline and the port to 0.3.7. These are headless tests, not native
screen-reader or high-refresh rendering validation. Oversized overlays can still
exceed the visible viewport; fitting position alone does not resize their content.
Diffz's four focused `magnify` library tests also pass with the polished 0.3.3
dependency pin, including live cache retention at the zoom cap.

## Snapshot port

`zzwong/gpui-pre` branch `codex/gpui-pre-0.3.7` imports the published 0.3.7
snapshot (Zed `1a28cff4b409169bac058bca40dfbfeb7621d19b`) and carries the
magnification, exact-live-raster, and viewport/accessibility polish patches.
The port adapts glyph caching to the upstream owned atlas-key API. All 377
GPUI library tests and library Clippy with warnings denied pass with
`test-support`. The standalone crate's existing SVG font fixtures were supplied
through local-only include paths; those paths are not committed.

The `codex/gpui-0.3.7` diffz branch uses this core port with GPUI Kit 0.7.0
and the complete 0.3.7 snapshot family. Window creation uses Kit's new
`open_window` helper, which owns Base Root/overlay hosting. Comment preview
heading sizes are preserved through the new heading-style callback.

The supporting forks retain Vulkan-first startup, lazy frame-sized path
textures, macOS display-link demand/idle handling, and inactive-window/idle
caret handling. The caret port preserves upstream's newer 300 ms typing pause,
stale-task guard, and stop cleanup. The vendored Linux snapshot retains the X11
frame-demand patch alongside upstream's visibility, power, and recovery changes.
Its exact patch applies cleanly to the published 0.3.7 source.

## Integration validation

All-feature workspace tests and all-feature, all-target workspace Clippy pass,
with warnings denied. The UI suite passes 95 tests, including all four zoom
tests; its two benchmarks and the separate native desktop probe remain ignored.
All 45 WGPU library tests pass, including the new lazy path-texture regression
test. The standalone snapshot omits the upstream font fixtures and Naga shader
test dependency; these were supplied locally without changing the published pin.
All nine focused caret tests pass, including idle settling, input-driven resume,
pause/blur cleanup, and inactive-window behavior. The standalone Base test
setup redirects its README fixture locally and omits unused benchmark-only
development dependencies; none of these setup changes are in the published pin.
Formatting and diff checks pass. The dependency audit remains for CI because
`cargo-deny` is not installed on this machine.
The new headless WGPU regression test checks that quad-only frames allocate no
path textures, the first path frame allocates them, and subsequent path frames
resize them correctly after a quad-only resize.

## Native follow-up

- Run native macOS/Windows CI and visible pinch/reversal checks.
- Check the macOS display-link patch on macOS; it cannot be tested natively here.
- Validate X11 idle/wake behavior in a real Xorg session; setup is deferred on
  this machine.
- Check accessibility highlighting/clicks, IME positioning, menus, tooltips,
  resizing, window chrome, and scale-factor changes while magnified.

Any native diffz test on this machine must use a leased unused workspace and
silent placement without changing the user's active workspace or focus.

## Upstream references

- Pinch input: https://github.com/zed-industries/zed/pull/47351 and
  https://github.com/zed-industries/zed/pull/51354
- Atlas changes: https://github.com/zed-industries/zed/pull/64331
- Renderer benchmark sessions: https://github.com/zed-industries/zed/pull/64109
- Kit migration: https://github.com/longbridge/gpui-kit/releases/tag/v0.7.0
- Community Edition: https://github.com/gpui-ce/gpui-ce

Introduce whole-window magnification upstream as a framework design proposal;
do not describe existing pinch input support as new work. Include the cache
lifetime/raster quality separation and benchmark limitations. Native platform
validation and overlay sizing remain outstanding before claiming a fully
production-ready upstream implementation.
